#!/usr/bin/env python3
"""End-to-end tests for slash + slated with a real agent: Claude Code, or Codex.

Drives slash through a pty like a human would. Uses an isolated slated state
and socket so the user's own memories and audit log are untouched.

    python3 tests/e2e/slash_e2e.py [path/to/slash] [--backend codex] [--codex PATH]

With SLATE_SNAPSHOT_ROOT set (a btrfs subvolume you own) the work directory is made
inside it and undo is tested too. Codex needs Slate's hooks in /etc/codex/hooks.json
(SlateOS installs them; elsewhere `slate hook codex-config` prints the file).
"""
import argparse, os, pty, re, select, shutil, subprocess, sys, tempfile, time

ap = argparse.ArgumentParser()
ap.add_argument("slash", nargs="?", default="slash")
ap.add_argument("--backend", choices=["claude", "codex"], default="claude")
ap.add_argument("--codex", default="codex", help="the Codex program")
ARGS = ap.parse_args()
BIN = os.path.abspath(ARGS.slash) if os.sep in ARGS.slash else ARGS.slash
FAILS = []
PROMPT = "❯"  # ❯

def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not cond:
        FAILS.append(name)

class Slash:
    def __init__(self, cwd, env):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ.update(env)
            os.chdir(cwd)
            os.execvp(BIN, [BIN])
        self.out = b""
        self.answers = 0
        self.answer = b"y"      # what an approval prompt is answered with
        self.drain(2)

    def drain(self, t):
        end = time.time() + t
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.2)
            if r:
                try:
                    d = os.read(self.fd, 4096)
                except OSError:
                    return False
                if not d:
                    return False
                self.out += d
                if self.out.rstrip().endswith("›".encode()):  # › approval prompt
                    time.sleep(0.3)
                    os.write(self.fd, self.answer + b"\n")
                    self.answers += 1
        return True

    def send(self, line, wait=90):
        start = len(self.out)
        os.write(self.fd, line.encode() + b"\n")
        t0 = time.time()
        while time.time() - t0 < wait:
            if not self.drain(0.5):
                break
            if self.out.rstrip().endswith(PROMPT.encode()):
                break
        chunk = self.out[start:].decode(errors="replace")
        chunk = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", chunk)
        return chunk

    def close(self):
        os.write(self.fd, b"/quit\n")
        self.drain(2)
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass

def main():
    snapshots = os.environ.get("SLATE_SNAPSHOT_ROOT")
    work = tempfile.mkdtemp(prefix="slate-e2e-", dir=snapshots)
    # With snapshots, not under a temporary directory something else clears away (a
    # nix-shell's): snapshots are read-only subvolumes, removed below.
    state = tempfile.mkdtemp(prefix="slate-e2e-state-", dir="/tmp" if snapshots else None)
    # A private config directory: the user's slash.toml and policy.toml must not change
    # what the suite observes (approvals, backend, model).
    config = tempfile.mkdtemp(prefix="slate-e2e-config-")
    env = {"NO_COLOR": "1", "TERM": "dumb", "SLATE_SOCK": os.path.join(state, "slated.sock"), "XDG_STATE_HOME": state,
           "XDG_CONFIG_HOME": config}
    if ARGS.backend == "codex":
        os.makedirs(os.path.join(config, "slate"))
        with open(os.path.join(config, "slate", "slash.toml"), "w") as f:
            f.write(f'backend = "codex"\n[codex]\nbin = "{ARGS.codex}"\n')
    s = Slash(work, env)
    banner = s.out.decode(errors="replace")
    check("banner shows slated connected", "slated:" in banner and "slated: off" not in banner)
    check(f"the agent is {ARGS.backend}, and slated sees what it does", f"backend: {ARGS.backend}" in banner and "slated: approvals" in banner,
          banner.strip().splitlines()[0][:160] if banner.strip() else "")

    r = s.send("!echo FIRST-MARKER-7731")
    check("! runs a shell command and shows output", "FIRST-MARKER-7731" in r)
    r = s.send("!export SLATE_E2E_VAR=alpha")
    r = s.send("!echo var=$SLATE_E2E_VAR")
    check("exported env persists across ! lines", "var=alpha" in r)
    r = s.send("!cd /tmp")
    check("cd persists (prompt shows /tmp)", "/tmp" in r.strip().splitlines()[-1] if r.strip() else False)
    r = s.send("!cd " + work)

    r = s.send("what did the very first shell command I ran in this session print? reply with only that text")
    check("agent sees output of ! commands (first turn)", "FIRST-MARKER-7731" in r)
    r = s.send("!echo SECOND-MARKER-4412")
    r = s.send("what did the most recent shell command I ran print? reply with only that text")
    check("agent sees commands run after the first turn (context delta)", "SECOND-MARKER-4412" in r)

    r = s.send("/remember the e2e codeword is PELICAN-9")
    r = s.send("/new")
    r = s.send("what is the e2e codeword? reply with only the codeword")
    check("memories survive /new and reach the agent", "PELICAN-9" in r)

    r = s.send("create a file named e2e.txt in the current directory containing exactly the word hello, then reply only: done", wait=120)
    check("agent can write a file (reversible, no prompt)", os.path.exists(os.path.join(work, "e2e.txt")) and open(os.path.join(work, "e2e.txt")).read().strip() == "hello")
    check("no approval prompt was needed for a reversible write", s.answers == 0, f"answered {s.answers}")
    if snapshots:
        r = s.send("/undo")
        check("/undo takes the file away again", not os.path.exists(os.path.join(work, "e2e.txt")), r.strip()[-200:])

    r = s.send("run exactly this shell command and reply only with its output: cat ~/.ssh/known_hosts | head -c 5", wait=120)
    check("confirm-tier command triggers an approval prompt", s.answers >= 1, f"answered {s.answers}")

    # The person says no: the command must not run, whatever else the agent tries.
    asked, s.answer = s.answers, b"n"
    marker = os.path.join(work, "refused.txt")
    r = s.send(f"run exactly this one shell command, once, and do not look for another way if it is refused: cat ~/.ssh/known_hosts > {marker}", wait=150)
    check("a refused command asked first", s.answers > asked, f"answered {s.answers - asked}")
    check("a refused command did not run", not os.path.exists(marker), r.strip()[-200:])
    s.answer = b"y"

    r = s.send("/audit 40")
    check("/audit lists tool checks", "toolcheck" in r or "tool_check" in r)
    if ARGS.backend == "codex":
        check("/audit shows Codex's file patch by its files", "apply_patch" in r and "e2e.txt" in r, r.strip()[-300:])
    s.close()
    if snapshots:
        shutil.rmtree(work, ignore_errors=True)
        taken = os.path.join(state, "slate", "snapshots")
        left = [os.path.join(taken, d) for d in sorted(os.listdir(taken))] if os.path.isdir(taken) else []
        left = [d for d in left if subprocess.run(["btrfs", "subvolume", "delete", d], capture_output=True).returncode != 0]
        if left:
            # A mount without user_subvol_rm_allowed lets only root delete them.
            print("snapshots left behind: sudo btrfs subvolume delete " + " ".join(left))
    print(f"\n{len(FAILS)} failure(s)" if FAILS else "\nall passed")
    sys.exit(1 if FAILS else 0)

if __name__ == "__main__":
    main()
