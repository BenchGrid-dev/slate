#!/usr/bin/env python3
"""End-to-end tests for slash + slated with a real Claude Code backend.

Drives slash through a pty like a human would. Uses an isolated slated state
and socket so the user's own memories and audit log are untouched.

    python3 tests/e2e/slash_e2e.py [path/to/slash]
"""
import os, pty, re, select, sys, tempfile, time

BIN = sys.argv[1] if len(sys.argv) > 1 else "slash"
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
            os.execv(BIN, [BIN])
        self.out = b""
        self.answers = 0
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
                    os.write(self.fd, b"y\n")
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
    work = tempfile.mkdtemp(prefix="slate-e2e-")
    state = tempfile.mkdtemp(prefix="slate-e2e-state-")
    env = {"NO_COLOR": "1", "TERM": "dumb", "SLATE_SOCK": os.path.join(state, "slated.sock"), "XDG_STATE_HOME": state}
    s = Slash(work, env)
    check("banner shows slated connected", "slated:" in s.out.decode(errors="replace") and "slated: off" not in s.out.decode(errors="replace"))

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

    r = s.send("run exactly this shell command and reply only with its output: cat ~/.ssh/known_hosts | head -c 5", wait=120)
    check("confirm-tier command triggers an approval prompt", s.answers >= 1, f"answered {s.answers}")

    r = s.send("/audit 5")
    check("/audit lists tool checks", "toolcheck" in r or "tool_check" in r)
    s.close()
    print(f"\n{len(FAILS)} failure(s)" if FAILS else "\nall passed")
    sys.exit(1 if FAILS else 0)

if __name__ == "__main__":
    main()
