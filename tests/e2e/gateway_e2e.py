#!/usr/bin/env python3
"""End-to-end test for an API connection of the person's own: `slash --serve`, the real
agent CLI, slash's gateway and a scripted OpenAI-format endpoint (mock_openai.py).

    python3 tests/e2e/gateway_e2e.py [path/to/slash]

Needs Claude Code on PATH, but no sign-in and no key: the endpoint is the mock. The
suite uses a configuration directory of its own, so the person's provider.toml and
slash.toml are neither read nor changed.
"""
import json, os, queue, socket, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else "slash"
HERE = os.path.dirname(os.path.abspath(__file__))
FAILS = []


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not cond:
        FAILS.append(name)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Slash:
    """`slash --serve` with a provider.toml that names the mock endpoint."""

    def __init__(self, port, key="sk-mock"):
        self.config = tempfile.mkdtemp(prefix="slate-gateway-config-")
        state = tempfile.mkdtemp(prefix="slate-gateway-state-")
        # Where slash looks for its configuration: XDG on Linux, the home's Library on macOS.
        mac = sys.platform == "darwin"
        base = os.path.join(self.config, "Library", "Application Support", "slate") if mac else os.path.join(self.config, "slate")
        os.makedirs(base)
        path = os.path.join(base, "provider.toml")
        with open(os.open(path, os.O_WRONLY | os.O_CREAT, 0o600), "w") as f:
            f.write(f'connection = "api"\n\n[api]\nformat = "openai"\nbase_url = "http://127.0.0.1:{port}/v1"\n'
                    f'api_key = "{key}"\nmodel = "mock-large"\nsmall_model = "mock-small"\n')
        # A sign-in of the person's own must not be what answers: no ANTHROPIC_* from outside.
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANTHROPIC_")}
        env.update(SLATE_SOCK=os.path.join(state, "slated.sock"), XDG_STATE_HOME=state, XDG_CONFIG_HOME=self.config, NO_COLOR="1")
        if mac:
            env["HOME"] = self.config
        self.p = subprocess.Popen([BIN, "--serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env,
                                  cwd=tempfile.mkdtemp(prefix="slate-gateway-cwd-"))
        self.q = queue.Queue()
        threading.Thread(target=self.reader, daemon=True).start()

    def reader(self):
        for line in self.p.stdout:
            try:
                self.q.put(json.loads(line))
            except Exception:
                self.q.put({"event": "?", "raw": line.strip()})

    def send(self, m):
        self.p.stdin.write(json.dumps(m) + "\n")
        self.p.stdin.flush()

    def until(self, name, timeout=120):
        """Events up to and including the first `name`."""
        end, seen = time.time() + timeout, []
        while time.time() < end:
            try:
                ev = self.q.get(timeout=0.5)
            except queue.Empty:
                continue
            seen.append(ev)
            if ev.get("event") == name:
                break
            if ev.get("event") == "approval":      # nothing here should need one; say yes and go on
                self.send({"op": "approve", "id": ev.get("id"), "allow": True})
        return seen

    def quit(self):
        try:
            self.send({"op": "quit"})
            self.p.wait(timeout=15)
        except Exception:
            self.p.kill()


def main():
    port = free_port()
    log = os.path.join(tempfile.mkdtemp(prefix="slate-gateway-mock-"), "requests.jsonl")
    mock = subprocess.Popen([sys.executable, os.path.join(HERE, "mock_openai.py"), str(port), log])
    time.sleep(1.0)
    try:
        s = Slash(port)
        ready = next((e for e in s.until("ready", 20) if e.get("event") == "ready"), {})
        check("slash starts with the connection's model", ready.get("model") == "mock-large", str(ready)[:120])
        if ready.get("model") != "mock-large":
            # Not our connection: do not send a request to whatever the person is signed in to.
            s.quit()
            raise SystemExit("slash did not read the test's provider.toml; stopping")

        s.send({"op": "prompt", "text": "Run the test command with the Bash tool and tell me what it printed."})
        events = s.until("done", 180)
        kinds = [e.get("event") for e in events]
        tool = next((e for e in events if e.get("event") == "tool_start"), {})
        result = next((e for e in events if e.get("event") == "tool_end"), {})
        done = events[-1] if events and events[-1].get("event") == "done" else {}
        check("the endpoint's tool call runs through Claude Code", tool.get("name") == "Bash" and tool.get("detail") == "echo gateway-works", str(tool)[:120])
        check("its result goes back and the answer quotes it", result.get("ok") is True and "gateway-works" in (done.get("summary") or ""), str(done)[:160])
        check("the turn ends well", done.get("ok") is True, str(kinds)[:200])

        requests = [json.loads(line) for line in open(log)]
        first = requests[0] if requests else {}
        body = first.get("body", {})
        names = [t.get("function", {}).get("name") for t in body.get("tools", [])]
        check("the endpoint is asked in its own format, with the person's key", first.get("path", "").endswith("/chat/completions")
              and first.get("authorization") == "Bearer sk-mock", str(first.get("path")))
        check("for the connection's model", body.get("model") == "mock-large", str(body.get("model")))
        check("with the system prompt first and the tools as functions", body.get("messages", [{}])[0].get("role") == "system" and "Bash" in names, str(names)[:120])
        check("without what only Anthropic's servers can do", "WebSearch" not in names and "ToolSearch" not in names, str(names)[:160])
        roles = [m.get("role") for m in requests[-1]["body"]["messages"]] if requests else []
        check("the tool result follows the call", roles[-2:] == ["assistant", "tool"], str(roles))

        s.send({"op": "prompt", "text": "Check the environment with the Bash tool."})
        events = s.until("done", 180)
        # The endpoint's answer quotes the command's output: the gateway's address, then
        # the number of times its key occurs in the environment.
        done = events[-1] if events and events[-1].get("event") == "done" else {}
        lines = (done.get("summary") or "").split()
        check("the agent's commands see the gateway, not the key", "http://127.0.0.1:" in lines and lines[-1:] == ["0"], str(done)[:160])
        s.quit()

        wrong = Slash(port, key="sk-wrong")
        wrong.until("ready", 20)
        wrong.send({"op": "prompt", "text": "hello"})
        events = wrong.until("done", 120)
        said = json.dumps(events)
        done = events[-1] if events else {}
        check("a refused key is reported with the endpoint's own words", done.get("ok") is False and "Incorrect API key" in said, said[-300:])
        wrong.quit()
    finally:
        mock.terminate()
    print()
    if FAILS:
        print(f"{len(FAILS)} failed: {FAILS}")
        sys.exit(1)
    print("all passed")


if __name__ == "__main__":
    main()
