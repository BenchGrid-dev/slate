#!/usr/bin/env python3
"""Install SlateOS from its image in QEMU, then boot what was installed.

    python3 tests/image/install_test.py --iso result/iso/slateos-*.iso --firmware OVMF.fd [--encrypt]

The image starts as it does for a person: firmware, boot menu, live desktop. The test
logs in on a console of the live system (a virtio console; root has no password there),
runs `slateos-install` on an empty disk and powers off. It then boots the disk without
the image, types the disk's passphrase on the keyboard when the installation is
encrypted, logs in as the new user on the console (their login shell is slash) and
checks that the desktop runs. Logs and a screenshot of the desktop stay in --workdir.

Needs qemu-system-<arch> and qemu-img on PATH and a UEFI firmware image (OVMF.fd for
x86_64, QEMU_EFI.fd for aarch64). With /dev/kvm it takes about seven minutes, without
it (emulation) more than an hour. .github/workflows/image.yml runs it for minor releases.
"""
import argparse
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
import tempfile
import time
import zlib

USER = "tester"
PASSWORD = "slateos-ci"
# mkpasswd -m sha-512 slateos-ci
PASSWORD_HASH = "$6$slateosci2026x$0X7tTXEQJ.90vQYTMuw/I5G3XMyoKC3ibm1QJVXMHncQfP0kDXuSwrQdlVCaqG7RurDtRw19hpb0tXk9iD.ff/"
PASSPHRASE = "slateos-ci-disk"
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
            "-bios", args.firmware,
            "-display", "none",
            "-device", "qemu-xhci", "-device", "usb-kbd", "-device", "usb-tablet",
            "-device", "virtio-rng-pci",
            "-nic", "user,model=virtio-net-pci",
            "-drive", f"file={args.disk},if=none,id=target,format=qcow2,discard=unmap",
            "-device", "virtio-blk-pci,drive=target,serial=slateos-target,bootindex=1",
            "-device", "virtio-serial-pci",
            "-chardev", f"socket,id=console,path={self.console_path},server=on,wait=off",
            "-device", "virtconsole,chardev=console",
            "-qmp", f"unix:{self.qmp_path},server=on,wait=off",
            "-serial", f"file:{os.path.join(args.workdir, name + '-serial.log')}",
        ]
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
        return int(match.group(1)), self.buffer[start:match.start()]

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
        options = f"--disk {disk} --user {USER} --host slateos-test --tz UTC --apps minimal --yes --password-hash '{PASSWORD_HASH}'"
        if args.encrypt:
            vm.run(f"printf %s '{PASSPHRASE}' > /tmp/passphrase", 30)
            options += " --passphrase-file /tmp/passphrase"
        log("install: running slateos-install" + (" --encrypt" if args.encrypt else ""))
        started = time.monotonic()
        status, out = vm.run(f"slateos-install {options} 2>&1", 3000)
        if status != 0:
            raise Failure(f"slateos-install exited with {status}. Its last output:\n{out[-3000:]}")
        log(f"install: installed in {int(time.monotonic() - started)} s; powering off")
        vm.run("sync", 120)
        vm.send("poweroff\n")
        vm.stop(180)
    except BaseException:
        vm.stop()
        raise


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


def boot(args):
    """Boot the installed disk without the image and check the desktop."""
    vm = Machine(args, "boot", with_image=False)
    try:
        if args.encrypt:
            unlock(vm)
        vm.expect(r"login: ", 600, "the installed system's login prompt")
        log("boot: the installed system is up; logging in as the new user")
        vm.send(f"{USER}\n")
        vm.expect(r"Password: ", 60, "the password prompt")
        vm.send(f"{PASSWORD}\n")
        time.sleep(8 * args.patience)
        # The login shell is slash: `!` hands a line to the real shell. slash shows the
        # line as it is typed, so nothing a check looks for is spelled out in it.
        checks = (
            "systemctl --user is-active slate-desktop slate-dock slate-panel slated | tr '\\n' ' '; echo; "
            "echo sway=$(pgrep -cx sway); slateos-version; "
            "echo root=$(findmnt -no FSTYPE /); echo disks=$(lsblk -rno TYPE | sort -u | tr '\\n' ,)"
        )
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
            if not missing:
                break
            if time.monotonic() > deadline:
                raise Failure("boot: the installed system lacks " + "; ".join(missing) + f". The checks printed:\n{out}")
            time.sleep(10 * args.patience)
        results = [line.strip() for line in out.splitlines() if line.strip()][-5:]  # after the echoed command
        log("boot: the desktop runs: " + " | ".join(results))
        time.sleep(10 * args.patience)
        vm.screenshot("desktop.png")
        vm.stop()
    except BaseException:
        try:
            vm.screenshot("failure.png")
        except Exception:  # noqa: BLE001
            pass
        vm.stop()
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--iso", required=True, help="the image to install from")
    parser.add_argument("--firmware", default=os.environ.get("OVMF_FD"), help="UEFI firmware image (or $OVMF_FD)")
    parser.add_argument("--workdir", help="where the disk, logs and screenshots go (default: a temporary directory)")
    parser.add_argument("--arch", choices=["x86_64", "aarch64"], help="default: from the image's name")
    parser.add_argument("--accel", choices=["kvm", "tcg"], help="default: kvm when /dev/kvm is usable for this architecture")
    parser.add_argument("--memory", type=int, default=6144, help="MB (default 6144)")
    parser.add_argument("--cpus", type=int, default=min(4, os.cpu_count() or 2))
    parser.add_argument("--disk-size", default="32G")
    parser.add_argument("--encrypt", action="store_true", help="install on an encrypted disk and type its passphrase at boot")
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

    log(f"image {args.iso} ({args.arch}, version {args.version or 'unknown'}); work in {args.workdir}")
    try:
        if args.only != "boot":
            subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", args.disk, args.disk_size], check=True)
            install(args)
        if args.only != "install":
            boot(args)
    except Failure as failure:
        log(f"FAILED: {failure}")
        return 1
    finally:
        if not args.keep_disk and args.only != "install" and os.path.exists(args.disk):
            os.remove(args.disk)
    log("PASSED: the image installs" + (" on an encrypted disk" if args.encrypt else "") + " and the installed system boots into the desktop")
    return 0


if __name__ == "__main__":
    sys.exit(main())
