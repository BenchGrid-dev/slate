#!/usr/bin/env python3
"""Install SlateOS from its image in QEMU, then boot what was installed.

    python3 tests/image/install_test.py --iso result/iso/slateos-*.iso --firmware OVMF.fd [--encrypt]

The image starts as it does for a person: firmware, boot menu, live desktop. The test
logs in on a console of the live system (a virtio console; root has no password there),
runs `slateos-install` on an empty disk and powers off. It then boots the disk without
the image, types the disk's passphrase on the keyboard when the installation is
encrypted, logs in as the new user on the console (their login shell is slash) and
checks that the desktop runs. Logs and a screenshot of the desktop stay in --workdir.

With --update REPO it goes on from there, as software update does on a person's
computer: it makes a release one version newer from the repository (its Slate package
in a binary cache, its source and signed manifest on a web server the guest reaches),
installs it with `slateos-update`, and restarts. First with the new version's boot
entry made unable to start: the machine is reset, the version from before comes up,
and nothing of the new one may be left. Then left alone: the new version comes up,
and becomes the default once its session stays up. Then back to the version before.

Needs qemu-system-<arch> and qemu-img on PATH and a UEFI firmware image (OVMF.fd for
x86_64, QEMU_EFI.fd for aarch64). With /dev/kvm it takes about five minutes, without
it (emulation) more than an hour; the update adds about a quarter of an hour, and needs
the firmware's code and variable store as two files (--firmware OVMF_CODE.fd
--firmware-vars OVMF_VARS.fd). .github/workflows/image.yml runs it for minor releases.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
import platform
import re
import select
import shutil
import socket
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import zlib

USER = "tester"
PASSWORD = "slateos-ci"
# mkpasswd -m sha-512 slateos-ci
PASSWORD_HASH = "$6$slateosci2026x$0X7tTXEQJ.90vQYTMuw/I5G3XMyoKC3ibm1QJVXMHncQfP0kDXuSwrQdlVCaqG7RurDtRw19hpb0tXk9iD.ff/"
PASSPHRASE = "slateos-ci-disk"
# An API connection to nowhere: enough to see that it arrives in the new home.
PROVIDER = ['connection = "api"', "", "[api]", 'format = "openai"', 'base_url = "http://127.0.0.1:9/v1"', 'api_key = "sk-image-test"', 'model = "image-test-model"']
EXIT = re.compile(r"@@EXIT:(\d+)@@")
ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07|\x1b[=>]")


class Failure(Exception):
    pass


def log(message):
    print(f"[install-test] {message}", flush=True)


class Machine:
    """One QEMU run: a console to talk to the guest and QMP for the keyboard and screen."""

    def __init__(self, args, name, with_image):
        self.args, self.name = args, name
        self.console_path = os.path.join(args.workdir, f"{name}-console.sock")
        self.qmp_path = os.path.join(args.workdir, f"{name}-qmp.sock")
        self.buffer = ""
        self.seen = 0
        self.line = ""
        self.console_log = open(os.path.join(args.workdir, f"{name}-console.log"), "w")
        for path in (self.console_path, self.qmp_path):
            if os.path.exists(path):
                os.remove(path)
        cpu = "host" if args.accel == "kvm" else "max"
        command = [
            f"qemu-system-{args.arch}",
            "-machine", ("q35" if args.arch == "x86_64" else "virt") + f",accel={args.accel}",
            "-cpu", cpu, "-smp", str(args.cpus), "-m", str(args.memory),
            "-display", "none",
            "-device", "qemu-xhci", "-device", "usb-kbd", "-device", "usb-tablet",
            "-device", "virtio-rng-pci",
            # Without a network when asked to: the minimal installation must not need one.
            "-nic", "none" if (args.offline and with_image) else "user,model=virtio-net-pci",
            "-drive", f"file={args.disk},if=none,id=target,format=qcow2,discard=unmap",
            "-device", "virtio-blk-pci,drive=target,serial=slateos-target,bootindex=1",
            "-device", "virtio-serial-pci",
            "-chardev", f"socket,id=console,path={self.console_path},server=on,wait=off",
            "-device", "virtconsole,chardev=console",
            "-qmp", f"unix:{self.qmp_path},server=on,wait=off",
            "-serial", f"file:{os.path.join(args.workdir, name + '-serial.log')}",
        ]
        if args.vars:
            # Firmware with a variable store, as computers have: what the boot loader
            # is told for the next start (the update's trial) survives the restart.
            command += ["-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={args.firmware}",
                        "-drive", f"if=pflash,format=raw,unit=1,file={args.vars}"]
        else:
            command += ["-bios", args.firmware]
        if args.arch == "x86_64":
            command += ["-vga", "none", "-device", "virtio-vga"]
        else:
            command += ["-device", "virtio-gpu-pci"]
        if with_image:
            command += [
                "-device", "virtio-scsi-pci",
                "-drive", f"file={args.iso},if=none,id=image,media=cdrom,readonly=on,format=raw",
                "-device", "scsi-cd,drive=image,bootindex=2",
            ]
        log(f"{name}: starting QEMU ({args.accel}, {args.cpus} CPUs, {args.memory} MB)")
        self.process = subprocess.Popen(command, stdin=subprocess.DEVNULL)
        self.console = self._connect(self.console_path)
        self.qmp = self._connect(self.qmp_path)
        self.qmp_file = self.qmp.makefile("rw")
        self.qmp_file.readline()  # the greeting
        self.command("qmp_capabilities")

    def _connect(self, path):
        deadline = time.monotonic() + 30
        while True:
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(path)
                return s
            except OSError:
                if self.process.poll() is not None:
                    raise Failure(f"QEMU exited with {self.process.returncode} while starting")
                if time.monotonic() > deadline:
                    raise Failure(f"no socket at {path}")
                time.sleep(0.2)

    # QMP

    def command(self, name, **arguments):
        message = {"execute": name}
        if arguments:
            message["arguments"] = arguments
        self.qmp_file.write(json.dumps(message) + "\n")
        self.qmp_file.flush()
        while True:
            line = self.qmp_file.readline()
            if not line:
                raise Failure("QEMU closed its control socket")
            answer = json.loads(line)
            if "return" in answer:
                return answer["return"]
            if "error" in answer:
                raise Failure(f"QMP {name}: {answer['error'].get('desc')}")

    def check_running(self):
        if self.process.poll() is not None:
            raise Failure(f"QEMU exited with {self.process.returncode}")
        status = self.command("query-status")
        if status.get("status") not in ("running", "prelaunch"):
            # A full host disk pauses the guest (io-error) and every wait would hang.
            raise Failure(f"the guest is not running: {status.get('status')}")

    def screen(self):
        """The screen's pixels: (width, height, RGB bytes)."""
        path = os.path.join(self.args.workdir, f"{self.name}-screen.ppm")
        self.command("screendump", filename=path)
        with open(path, "rb") as f:
            data = f.read()
        fields, offset = [], 0
        while len(fields) < 4:  # P6, width, height, maximum
            end = offset
            while data[end:end + 1] not in (b" ", b"\n", b"\t", b"\r"):
                end += 1
            fields.append(data[offset:end])
            offset = end + 1
        return int(fields[1]), int(fields[2]), data[offset:]

    def screenshot(self, name):
        width, height, pixels = self.screen()
        path = os.path.join(self.args.workdir, name)
        write_png(path, width, height, pixels)
        log(f"{self.name}: screenshot {path}")

    def type_keys(self, text):
        names = {"-": "minus", "\n": "ret", " ": "spc"}
        for char in text:
            key = {"type": "qcode", "data": names.get(char, char)}
            self.command("send-key", keys=[key])
            time.sleep(0.15 * self.args.patience)

    # The console

    def _read(self, wait):
        ready, _, _ = select.select([self.console], [], [], wait)
        if not ready:
            return
        data = self.console.recv(65536)
        if not data:
            time.sleep(wait)
            return
        text = ANSI.sub("", data.decode("utf-8", "replace")).replace("\r", "")
        self.buffer += text
        self.console_log.write(text)
        self.console_log.flush()
        if self.args.verbose:
            self.line += text
            *lines, self.line = self.line.split("\n")
            for line in lines:
                print(f"  | {line}", flush=True)

    def expect(self, pattern, seconds, what):
        """Wait for `pattern` in what the console has printed since the last match."""
        regex = re.compile(pattern)
        deadline = time.monotonic() + seconds * self.args.patience
        checked = time.monotonic()
        while True:
            match = regex.search(self.buffer, self.seen)
            if match:
                self.seen = match.end()
                return match
            if time.monotonic() > deadline:
                tail = self.buffer[-1500:]
                raise Failure(f"{self.name}: timed out waiting for {what}. The console's last output:\n{tail}")
            if time.monotonic() - checked > 15:
                self.check_running()
                checked = time.monotonic()
            self._read(1.0)

    def send(self, text):
        self.console.sendall(text.encode())

    def run(self, command, seconds, prefix=""):
        """Run a shell command in the guest: (exit status, its output)."""
        start = len(self.buffer)
        self.seen = start
        self.send(f'{prefix}{command}; echo "@@EXIT:$?@@"\n')
        match = self.expect(EXIT.pattern, seconds, f"the end of `{command[:60]}`")
        result = int(match.group(1)), self.buffer[start:match.start()]
        if prefix:
            # slash reads the next line once its prompt is back; what is typed while it
            # returns to its line editor is cut short (the terminal's pending input is
            # dropped when it changes mode).
            self.expect("❯", 60, "slash's prompt after the command")
            time.sleep(0.5 * self.args.patience)
        return result

    def stop(self, seconds=0):
        """Wait for the guest to power off, then make sure QEMU is gone."""
        try:
            if seconds:
                self.process.wait(timeout=seconds * self.args.patience)
        except subprocess.TimeoutExpired:
            log(f"{self.name}: the guest did not power off in time; stopping QEMU")
        if self.process.poll() is None:
            try:
                self.command("quit")
            except Exception:  # noqa: BLE001
                self.process.kill()
            self.process.wait(timeout=30)
        self.console_log.close()


def write_png(path, width, height, rgb):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(b"\x00" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(rows, 6)))
        f.write(chunk(b"IEND", b""))


def install(args):
    """Boot the image and install to the disk from the live system's console."""
    vm = Machine(args, "install", with_image=True)
    try:
        vm.expect(r"login: ", 600, "the live system's login prompt")
        log("install: the live system is up; logging in as root")
        vm.send("root\n")
        time.sleep(3 * args.patience)
        # A plain shell: no line editor redrawing the commands, no echo of them.
        vm.send("exec bash --norc --noediting\n")
        time.sleep(1 * args.patience)
        vm.send("stty -echo; PS1=\n")
        status, _ = vm.run("true", 60)
        if status != 0:
            raise Failure("no shell on the live system's console")
        status, out = vm.run("slateos-version; readlink -f /dev/disk/by-id/virtio-slateos-target", 60)
        disk = out.strip().splitlines()[-1] if out.strip() else ""
        if status != 0 or not disk.startswith("/dev/"):
            raise Failure(f"the live system does not see the target disk:\n{out}")
        log(f"install: live system: {out.strip().splitlines()[0]}; target disk {disk}")
        # The image holds no agent program: Claude Code is not SlateOS's to hand on. It
        # names them (which version, where from) and an installation downloads them.
        # (The store path itself: its recipe, a .drv, and the stand-in that holds its
        # build tools, "inputDerivation-claude-code-…", are on the image on purpose.)
        status, out = vm.run("slateos-live-agents --status; ls /nix/store | grep -E '^[a-z0-9]{32}-(claude-code|codex)-[0-9.]+$'; echo listed", 60)
        states = re.findall(r"^(claude-code|codex) \S+ (here|missing)$", out, re.M)
        if sorted(states) != [("claude-code", "missing"), ("codex", "missing")] or re.search(r"^[a-z0-9]{32}-(claude-code|codex)-[0-9.]+$", out, re.M):
            raise Failure(f"the image should name the agent programs and hold none of them:\n{out}")
        log("install: the image holds no agent program")
        if args.live_agent:
            # What a sign-in in the installer does: Claude Code comes into the live
            # session. Then the network goes away, and the installation has to take the
            # program from the live session, not from Anthropic a second time.
            started = time.monotonic()
            status, out = vm.run("slateos-live-agents claude-code > /tmp/live-agents.log 2>&1; echo LIVE-EXIT:$?; tail -n 5 /tmp/live-agents.log; "
                                 "claude --version; nmcli networking off; sleep 3; "
                                 "curl -fsS -o /dev/null --max-time 8 https://cache.nixos.org/nix-cache-info && echo STILL-ONLINE; true", 1800)
            if "LIVE-EXIT:0" not in out or "(Claude Code)" not in out or "STILL-ONLINE" in out:
                raise Failure(f"the live session did not get Claude Code, or the network did not go away:\n{out[-2000:]}")
            log(f"install: the live session downloaded Claude Code in {int(time.monotonic() - started)} s; the network is off")
        # The agent's connection, the way the graphical installer hands it over: files
        # for the new home directory, the key readable by its owner alone.
        vm.run("mkdir -p /tmp/agent-home/.config/slate && chmod -R 700 /tmp/agent-home && "
               f"printf '%s\\n' {' '.join(repr(line) for line in PROVIDER)} > /tmp/agent-home/.config/slate/provider.toml && "
               "chmod 600 /tmp/agent-home/.config/slate/provider.toml", 30)
        options = (f"--disk {disk} --user {USER} --host slateos-test --tz UTC --apps minimal --yes --password-hash '{PASSWORD_HASH}' "
                   "--home-files /tmp/agent-home")
        if args.encrypt:
            vm.run(f"printf %s '{PASSPHRASE}' > /tmp/passphrase", 30)
            options += " --passphrase-file /tmp/passphrase"
        log("install: running slateos-install" + (" --encrypt" if args.encrypt else "") + (", without a network" if args.offline else ""))
        started = time.monotonic()
        status, out = vm.run(f"slateos-install {options} 2>&1", 3000)
        if status != 0:
            raise Failure(f"slateos-install exited with {status}. Its last output:\n{out[-3000:]}")
        programs = re.findall(r"^Agent programs: (.*)$", out, re.M)
        if not programs or programs[-1].split() != (expected_programs(args) or ["none"]):
            raise Failure(f"slateos-install should have installed the agent programs {expected_programs(args) or 'none'}; it says {programs}. "
                          f"Its last output:\n{out[-3000:]}")
        if args.live_agent and re.search(r"building '/nix/store/[a-z0-9]+-claude(-code-[^']*)?\.drv'", out):
            raise Failure("the installation built Claude Code again instead of copying it from the live session")
        log(f"install: installed in {int(time.monotonic() - started)} s, agent programs: {programs[-1]}; powering off")
        vm.run("sync", 120)
        vm.send("poweroff\n")
        vm.stop(180)
    except BaseException:
        vm.stop()
        raise


def expected_programs(args):
    """The agent programs an installation ends up with: none without a network, the one
    the live session held when the network went away, else every one."""
    if args.offline:
        return []
    return ["claude-code"] if args.live_agent else ["claude-code", "codex"]


def unlock(vm):
    """Type the disk's passphrase once the boot has stopped to ask for it. Nothing tells
    the outside that it has, so: the screen shows something and has not changed over
    three intervals of six seconds. The boot menu counts down for five seconds, and keys
    typed into it would be commands (`e` edits the kernel's command line)."""
    still = 0
    previous = None
    typed = 0
    deadline = time.monotonic() + 900 * vm.args.patience
    while time.monotonic() < deadline:
        if "login: " in vm.buffer:
            return
        # A whole interval between two looks at the screen, whatever the console prints
        # meanwhile (the firmware and the boot menu write to it too). Under emulation
        # the kernel's first lines also stand still for a while.
        interval_end = time.monotonic() + 6.0 * min(vm.args.patience, 3.0)
        while time.monotonic() < interval_end:
            vm._read(min(1.0, max(0.05, interval_end - time.monotonic())))
        vm.check_running()
        _, _, pixels = vm.screen()
        lit = len(pixels) - pixels.count(0)
        if previous is not None and len(previous) == len(pixels):
            changed = sum(1 for i in range(0, len(pixels), 4096) if pixels[i:i + 4096] != previous[i:i + 4096])
            still = still + 1 if changed <= 2 and lit > 2000 else 0
        previous = pixels
        if still >= 3 and typed < 8:
            typed += 1
            log(f"boot: the screen is still; typing the disk's passphrase (attempt {typed})")
            vm.screenshot(f"passphrase-prompt-{typed}.png")
            vm.type_keys(PASSPHRASE + "\n")
            still = 0
            previous = None
    raise Failure("boot: the encrypted system did not come up after its passphrase was typed")


def login(vm, what="the installed system"):
    """Wait for the login prompt and log in as the new user. Their shell is slash."""
    if vm.args.encrypt:
        unlock(vm)
    vm.expect(r"login: ", 600, f"{what}'s login prompt")
    log(f"boot: {what} is up; logging in as the new user")
    vm.send(f"{USER}\n")
    vm.expect(r"Password: ", 60, "the password prompt")
    vm.send(f"{PASSWORD}\n")
    time.sleep(8 * vm.args.patience)


def boot(args):
    """Boot the installed disk without the image and check the desktop."""
    vm = Machine(args, "boot", with_image=False)
    try:
        login(vm)
        # The login shell is slash: `!` hands a line to the real shell. slash shows the
        # line as it is typed, so nothing a check looks for is spelled out in it.
        checks = (
            "systemctl --user is-active slate-desktop slate-dock slate-panel slated | tr '\\n' ' '; echo; "
            "echo sway=$(pgrep -cx sway); slateos-version; "
            "echo root=$(findmnt -no FSTYPE /); echo disks=$(lsblk -rno TYPE | sort -u | tr '\\n' ,); "
            "echo provider=$(stat -c %a:%U ~/.config/slate/provider.toml):$(slate provider show | tr -d ' '); "
            # The agent programs: the ones the installation downloaded, and they run.
            "echo programs=$(tr -d ' \\n' < /etc/slateos/settings.json):"
            "$(command -v claude > /dev/null && claude --version | tr ' ' _):$(command -v codex > /dev/null && codex --version | tr ' ' _)"
        )
        want = expected_programs(args)
        programs = (r'programs=\{"agents":\[' + ",".join(f'"{name}"' for name in want) + r"\]\}:"
                    + (r"[0-9.]+_\(Claude_Code\)" if "claude-code" in want else "") + ":"
                    + (r"codex\S*[0-9]" if "codex" in want else "$"))
        deadline = time.monotonic() + 300 * args.patience
        while True:
            _, out = vm.run(checks, 120, prefix="!")
            missing = []
            if "active active active active" not in out:
                missing.append("the desktop's services (slate-desktop, slate-dock, slate-panel, slated)")
            if not re.search(r"sway=[1-9]", out):
                missing.append("the compositor")
            if args.version and f"SlateOS {args.version}" not in out:
                missing.append(f"SlateOS {args.version} in slateos-version")
            if "root=btrfs" not in out:
                missing.append("a btrfs root")
            if args.encrypt and not re.search(r"disks=\S*crypt", out):
                missing.append("an encrypted disk")
            if not re.search(rf'provider=600:{USER}:.*"connection":"api".*"model":"image-test-model"', out):
                missing.append("the agent's connection in the new home (provider.toml, private, the user's)")
            if not re.search(programs, out, re.M):
                missing.append(f"the agent programs {', '.join(want) or 'none (and none on PATH)'}, in settings.json and running")
            if not missing:
                break
            if time.monotonic() > deadline:
                raise Failure("boot: the installed system lacks " + "; ".join(missing) + f". The checks printed:\n{out}")
            time.sleep(10 * args.patience)
        results = [line.strip() for line in out.splitlines() if line.strip()][-7:]  # after the echoed command
        log("boot: the desktop runs: " + " | ".join(results))
        time.sleep(10 * args.patience)
        vm.screenshot("desktop.png")
        if args.live_agent:
            add_programs(vm)
        if args.update:
            update(vm, args)
        vm.stop()
    except BaseException:
        try:
            vm.screenshot("failure.png")
        except Exception:  # noqa: BLE001
            pass
        vm.stop()
        raise


def add_programs(vm):
    """What the welcome and Settings do on a system that lacks an agent program: this
    one was installed with Claude Code alone, and now has a network. The rebuild
    downloads Codex and keeps Claude Code."""
    log("boot: adding the missing agent program, as Settings does")
    started = time.monotonic()
    status, out = as_root(vm, "add-programs", ["slateos-settings set agents all 2>&1 | tail -n 25"], 2400)
    _, now = vm.run("echo now=$(tr -d ' \\n' < /etc/slateos/settings.json):$(codex --version | tr ' ' _):$(claude --version | tr ' ' _)", 120, prefix="!")
    if not re.search(r'now=\{"agents":\["claude-code","codex"\]\}:codex\S*[0-9]:[0-9.]+_\(Claude_Code\)', now):
        raise Failure(f"slateos-settings set agents all did not leave both agent programs installed and running:\n{out[-2500:]}\n{now[-600:]}")
    log(f"boot: both agent programs are installed, {int(time.monotonic() - started)} s after asking")


def next_version(version):
    major, minor, patch = version.split(".")
    return f"{major}.{minor}.{int(patch) + 1}"


class QuietFiles(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


class Release:
    """A SlateOS release newer than the image's, made from the repository and served to
    the guest over HTTP (10.0.2.2 is the host, seen from QEMU's user network): its source
    and signed manifest, and a binary cache with its Slate package, so that the guest
    only builds what carries its own names, as a person's computer does with a cache."""

    def __init__(self, args):
        self.version = next_version(args.version)
        work = os.path.join(args.workdir, "release")
        www = os.path.join(work, "www")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(os.path.join(www, "release"))
        log(f"update: making release {self.version} from {args.update}")
        tree = os.path.join(work, f"slateos-{self.version}")
        tracked = subprocess.run(["git", "-C", args.update, "ls-files", "-z"], check=True, capture_output=True).stdout
        for name in filter(None, tracked.decode().split("\0")):
            target = os.path.join(tree, name)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.copy2(os.path.join(args.update, name), target, follow_symlinks=False)
        self.set_version(tree, args.version, args.update)

        def nix(*command):
            return subprocess.run(["nix", "--extra-experimental-features", "nix-command flakes", *command],
                                  check=True, capture_output=True, text=True).stdout.strip()
        started = time.monotonic()
        package = nix("build", f"path:{tree}#slate", "--no-link", "--print-out-paths").splitlines()[-1]
        log(f"update: built {package} in {int(time.monotonic() - started)} s")
        updater = os.path.join(package, "bin", "slateos-update")
        secret, public = os.path.join(work, "cache.secret"), os.path.join(work, "cache.public")
        subprocess.run(["nix-store", "--generate-binary-cache-key", "slateos-image-test-1", secret, public], check=True)
        with open(public, encoding="utf-8") as f:
            self.cache_key = f.read().strip()
        nix("copy", "--to", f"file://{www}/cache?secret-key={secret}", package)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(QuietFiles, directory=www))
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.address = f"http://10.0.2.2:{self.server.server_address[1]}"

        archive = os.path.join(www, "release", f"slateos-{self.version}-source.tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(tree, arcname=f"slateos-{self.version}")
        with open(archive, "rb") as f:
            data = f.read()
        manifest = os.path.join(www, "release", "slateos-update.json")
        with open(manifest, "w", encoding="utf-8") as f:
            json.dump({"format": 1, "version": self.version, "released": time.strftime("%Y-%m-%d"),
                       "summary": "A release made by the image test.", "notes": "",
                       "source": {"url": os.path.basename(archive), "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)},
                       # The release's own binary cache, as its manifest names it.
                       "cache": {"url": f"{self.address}/cache", "key": self.cache_key}}, f)
        key = os.path.join(work, "release.key")
        self.release_key = subprocess.run([updater, "keygen", key], check=True, capture_output=True, text=True).stdout.strip()
        with open(manifest + ".sig", "w", encoding="utf-8") as f:
            f.write(subprocess.run([updater, "sign", key, manifest], check=True, capture_output=True, text=True).stdout)

        log(f"update: release {self.version} is served at {self.address} (the guest's view)")

    @staticmethod
    def set_version(tree, old, repo):
        """The version, wherever a release names it: the workspace, its lock file, the flake."""
        def replace(name, pattern, replacement, count=1):
            path = os.path.join(tree, name)
            with open(path, encoding="utf-8") as f:
                text = f.read()
            text, done = re.subn(pattern, replacement, text, flags=re.M)
            if done < count:
                raise Failure(f"update: {name} does not name version {old} where a release does")
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
        new = next_version(old)
        replace("Cargo.toml", rf'^version = "{re.escape(old)}"$', f'version = "{new}"')
        replace("flake.nix", rf'version = "{re.escape(old)}";', f'version = "{new}";')
        members = []
        for crate in sorted(os.listdir(os.path.join(repo, "crates"))):
            manifest = os.path.join(repo, "crates", crate, "Cargo.toml")
            if os.path.isfile(manifest):
                with open(manifest, encoding="utf-8") as f:
                    members.append(re.search(r'^name = "(.+)"$', f.read(), re.M).group(1))
        names = "|".join(re.escape(m) for m in members)
        replace("Cargo.lock", rf'^(name = "(?:{names})"\nversion = "){re.escape(old)}"$', rf'\g<1>{new}"', count=len(members))


def as_root(vm, name, lines, seconds):
    """Run shell lines as root in the guest: (exit status, output). They go into a file
    (so: no single quotes in them) that sudo runs, with the person's password on its input."""
    quoted = " ".join(f"'{line}'" for line in lines)
    vm.run(f"printf '%s\\n' {quoted} > /tmp/{name}", 60, prefix="!")
    return vm.run(f'echo {PASSWORD} | sudo -S -p "" sh /tmp/{name}', seconds, prefix="!")


def update_state(vm):
    """What slateos-update knows, as the lines of its JSON that name a state."""
    _, out = vm.run("slateos-update status --json | grep -E 'phase|outcome|generation|trial|current'", 120, prefix="!")
    return out


def wait_for(vm, pattern, seconds, what):
    deadline = time.monotonic() + seconds * vm.args.patience
    while True:
        out = update_state(vm)
        if re.search(pattern, out):
            return out
        if time.monotonic() > deadline:
            raise Failure(f"update: {what} did not happen. slateos-update says:\n{out}")
        time.sleep(5 * vm.args.patience)


def restart(vm, comes_up=True):
    """Restart the guest from inside. A version that does not come up shows no login
    prompt; after a while the machine is reset, as a person does with the power button."""
    vm.send(f'!echo {PASSWORD} | sudo -S -p "" systemctl reboot\n')
    time.sleep(5 * vm.args.patience)
    vm.seen = len(vm.buffer)
    if comes_up:
        return
    deadline = time.monotonic() + 100 * vm.args.patience
    while time.monotonic() < deadline:
        vm._read(1.0)
        if "login: " in vm.buffer[vm.seen:]:
            raise Failure("update: the version that was made unable to start came up")
    vm.check_running()
    vm.screenshot("update-does-not-start.png")
    log("update: no login prompt after 100 s; resetting the machine")
    vm.command("system_reset")
    vm.seen = len(vm.buffer)


def update(vm, args):
    """From the installed system: a new release is installed and started on trial. Made
    unable to start, it is gone at the next start; installed again, it comes up and
    becomes the default; then the version before is gone back to."""
    old = args.version
    release = Release(args)
    new = release.version
    config = {"version": old, "url": f"{release.address}/release/slateos-update.json", "keys": [release.release_key],
              "config": "/etc/slateos", "trial": True, "health": {"user": USER, "processes": ["slate-desktop"]}}
    vm.run(f"printf '%s\\n' '{json.dumps(config)}' > /tmp/update.json", 60, prefix="!")
    install = ["slateos-update --config /tmp/update.json install"]

    def install_update(attempt):
        log(f"update: installing {new} ({attempt})")
        started = time.monotonic()
        status, out = as_root(vm, "install.sh", install, 2400)
        if status != 0:
            _, tail = as_root(vm, "log.sh", ["tail -n 40 /var/lib/slateos-update/log"], 60)
            raise Failure(f"update: slateos-update install exited with {status}:\n{out[-2000:]}\nIts log ends:\n{tail[-3000:]}")
        if "if it does not come up" not in out:
            raise Failure(f"update: {new} was installed without a trial start:\n{out[-1500:]}")
        state = update_state(vm)
        generation = re.search(r'"generation": (\d+)', state)
        if not re.search(r'"phase": "restart"', state) or not generation:
            raise Failure(f"update: after the installation slateos-update says:\n{state}")
        log(f"update: {new} is generation {generation.group(1)}, installed in {int(time.monotonic() - started)} s")
        return generation.group(1)

    def running(version, what):
        _, out = vm.run("slateos-version", 120, prefix="!")
        if f"SlateOS {version} " not in out:
            raise Failure(f"update: {what}, and slateos-version says:\n{out}")

    # 1. A version that does not start: the old one is back by itself, everything as it was.
    generation = install_update("to be kept from starting")
    as_root(vm, "break.sh", [f'echo "options systemd.unit=emergency.target" >> /boot/loader/entries/nixos-generation-{generation}.conf', "sync"], 60)
    restart(vm, comes_up=False)
    login(vm, "the version from before")
    running(old, f"after {new} did not start, {old} should run again")
    wait_for(vm, r'"outcome": "rolled-back"', 180, f"noticing that {new} did not start")
    _, left = as_root(vm, "left.sh", ["ls /boot/loader/entries /nix/var/nix/profiles", "grep -m1 ^version /etc/slateos/slate/Cargo.toml", "ls /etc/slateos"], 60)
    if f"generation-{generation}.conf" in left or f"system-{generation}-link" in left or f'"{old}"' not in left or "slate.previous" in left:
        raise Failure(f"update: {new} did not start, and something of it is left:\n{left}")
    log(f"update: {new} did not start; {old} runs again and nothing of {new} is left")

    # 2. The same release, left alone: it comes up and becomes the default.
    generation = install_update("to be started")
    restart(vm)
    login(vm, f"SlateOS {new}")
    running(new, f"{new} was started on trial")
    wait_for(vm, r'"outcome": "installed"', 300, f"{new} becoming the default once its session stays up")
    _, default = as_root(vm, "default.sh", ["cat /boot/loader/loader.conf"], 60)
    if f"nixos-generation-{generation}.conf" not in default:
        raise Failure(f"update: {new} runs, but the boot menu's default is:\n{default}")
    vm.screenshot("updated.png")
    log(f"update: {new} came up on trial and is the default now")

    # 3. Back to the version before.
    status, out = as_root(vm, "back.sh", ["slateos-update back"], 600)
    if status != 0 or "starts again at the next restart" not in out:
        raise Failure(f"update: going back exited with {status}:\n{out[-1500:]}")
    restart(vm)
    login(vm, "the version gone back to")
    running(old, f"after going back, {old} should run")
    wait_for(vm, r'"outcome": "went-back"', 180, "recording the way back")
    log(f"update: went back to {old}")
    release.server.shutdown()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--iso", required=True, help="the image to install from")
    parser.add_argument("--firmware", default=os.environ.get("OVMF_FD"), help="UEFI firmware image (or $OVMF_FD)")
    parser.add_argument("--firmware-vars", help="the firmware's variable store template (OVMF_VARS.fd): --firmware is then its code (OVMF_CODE.fd), "
                        "and variables survive a restart, which an update's trial start needs")
    parser.add_argument("--update", metavar="REPO", help="after the boot: make a newer release from this repository and update to it "
                        "(needs --firmware-vars, nix and git, and the host's architecture)")
    parser.add_argument("--workdir", help="where the disk, logs and screenshots go (default: a temporary directory)")
    parser.add_argument("--arch", choices=["x86_64", "aarch64"], help="default: from the image's name")
    parser.add_argument("--accel", choices=["kvm", "tcg"], help="default: kvm when /dev/kvm is usable for this architecture")
    parser.add_argument("--memory", type=int, default=6144, help="MB (default 6144)")
    parser.add_argument("--cpus", type=int, default=min(4, os.cpu_count() or 2))
    parser.add_argument("--disk-size", default="32G")
    parser.add_argument("--encrypt", action="store_true", help="install on an encrypted disk and type its passphrase at boot")
    parser.add_argument("--offline", action="store_true", help="install without a network (the minimal application set promises to need none)")
    parser.add_argument("--live-agent", action="store_true", help="download Claude Code into the live session first, as a sign-in in the installer "
                        "does, then turn the network off: the installation takes the program from the live session")
    parser.add_argument("--patience", type=float, help="multiply every wait (default 1, or 8 under emulation)")
    parser.add_argument("--only", choices=["install", "boot"], help="one half: install only, or boot the disk a previous run left in --workdir")
    parser.add_argument("--keep-disk", action="store_true", help="keep the installed disk in --workdir")
    parser.add_argument("--quiet", dest="verbose", action="store_false", help="do not print the guest's console")
    args = parser.parse_args()

    name = re.match(r"slateos-(.+)-(x86_64|aarch64)-linux\.iso$", os.path.basename(os.path.realpath(args.iso)))
    args.version = name.group(1) if name else None
    args.arch = args.arch or (name.group(2) if name else platform.machine())
    if not args.firmware or not os.path.exists(args.firmware):
        parser.error("--firmware: a UEFI firmware image is needed (OVMF.fd, or QEMU_EFI.fd on aarch64)")
    if not args.accel:
        args.accel = "kvm" if platform.machine() == args.arch and os.access("/dev/kvm", os.R_OK | os.W_OK) else "tcg"
    if args.patience is None:
        args.patience = 1.0 if args.accel == "kvm" else 8.0
    for tool in (f"qemu-system-{args.arch}", "qemu-img"):
        if not shutil.which(tool):
            parser.error(f"{tool} is not on PATH")
    args.workdir = os.path.abspath(args.workdir or tempfile.mkdtemp(prefix="slateos-install-test-"))
    os.makedirs(args.workdir, exist_ok=True)
    args.disk = os.path.join(args.workdir, "disk.qcow2")
    args.iso = os.path.realpath(args.iso)
    # The variable store belongs to the machine like its disk: written by the
    # installation (the boot entry), read by every start after it.
    args.vars = os.path.join(args.workdir, "vars.fd") if args.firmware_vars else None
    if args.update:
        if not args.firmware_vars:
            parser.error("--update needs --firmware-vars: without a variable store the firmware forgets the trial start")
        if args.encrypt or args.only == "install" or not args.version:
            parser.error("--update goes with a plain installation that is booted, from an image whose name carries its version")
        args.update = os.path.abspath(args.update)

    log(f"image {args.iso} ({args.arch}, version {args.version or 'unknown'}); work in {args.workdir}")
    try:
        if args.only != "boot":
            subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", args.disk, args.disk_size], check=True)
            if args.vars:
                shutil.copyfile(args.firmware_vars, args.vars)
                os.chmod(args.vars, 0o644)
            install(args)
        if args.only != "install":
            boot(args)
    except Failure as failure:
        log(f"FAILED: {failure}")
        return 1
    finally:
        if not args.keep_disk and args.only != "install" and os.path.exists(args.disk):
            os.remove(args.disk)
    log("PASSED: the image installs" + (" on an encrypted disk" if args.encrypt else "") + " and the installed system boots into the desktop"
        + (", updates to a newer release on trial, survives one that does not start, and goes back" if args.update else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
