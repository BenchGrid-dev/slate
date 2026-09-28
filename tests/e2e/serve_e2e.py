#!/usr/bin/env python3
"""End-to-end test for `slash --serve`, the protocol the desktop panel uses.

    python3 tests/e2e/serve_e2e.py [path/to/slash]
"""
import json, os, queue, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else "slash"
FAILS = []


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not cond:
        FAILS.append(name)


def main():
    state = tempfile.mkdtemp(prefix="slate-serve-")
    env = dict(os.environ, SLATE_SOCK=os.path.join(state, "slated.sock"), XDG_STATE_HOME=state, NO_COLOR="1")
    p = subprocess.Popen([BIN, "--serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env, cwd=tempfile.mkdtemp(prefix="slate-serve-cwd-"))
    q = queue.Queue()

    def reader():
        for line in p.stdout:
            try:
                q.put(json.loads(line))
            except Exception:
                q.put({"event": "?", "raw": line.strip()})

    threading.Thread(target=reader, daemon=True).start()

    def send(m):
        p.stdin.write(json.dumps(m) + "\n")
        p.stdin.flush()

    def wait_event(name, timeout=120):
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            try:
                ev = q.get(timeout=0.5)
            except queue.Empty:
                continue
            seen.append(ev.get("event"))
            if ev.get("event") == name:
                return ev, seen
        return None, seen

    ready, _ = wait_event("ready", 15)
    check("ready event with backend and slated", bool(ready) and ready.get("slated") is True, str(ready)[:80])
    check("stdout is JSON only", all(ev != "?" for ev in [ready.get("event")] if ready))

    send({"op": "prompt", "text": "reply with exactly the word SERVE-OK"})
    done, seen = wait_event("done", 120)
    check("prompt produces turn_start, text and done", bool(done) and "turn_start" in seen and "text" in seen, str(seen[:6]))

    send({"op": "prompt", "text": "run exactly this shell command and reply only with its output: cat ~/.ssh/known_hosts | head -c 3"})
    ap, _ = wait_event("approval_needed", 120)
    check("confirm-tier tool raises approval_needed", bool(ap) and ap.get("tool") == "Bash" and ap.get("tier") == "confirm")
    if ap:
        send({"op": "approve", "id": ap["id"], "allow": False})
        res, _ = wait_event("approval_resolved", 30)
        check("approval answer is acknowledged", bool(res) and res.get("allow") is False)
    done, _ = wait_event("done", 120)
    check("turn finishes after the denial", bool(done))

    send({"op": "command", "line": "/auto on"})
    done, seen = wait_event("done", 20)
    check("/auto on is acknowledged as a note", bool(done) and "note" in seen)
    send({"op": "prompt", "text": "run exactly this shell command and reply only with its output: cat ~/.ssh/known_hosts | head -c 3"})
    done, seen = wait_event("done", 120)
    check("with /auto on no approval is requested", bool(done) and "approval_needed" not in seen, str(seen[:8]))

    send({"op": "quit"})
    try:
        p.wait(timeout=10)
    except subprocess.TimeoutExpired:
        p.kill()
    check("quit exits cleanly", p.returncode == 0, str(p.returncode))
    print(f"\n{len(FAILS)} failure(s)" if FAILS else "\nall passed")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
