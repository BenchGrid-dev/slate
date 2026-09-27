#!/usr/bin/env python3
"""End-to-end tests for slate-desktop on a live wlroots desktop.

Drives `slate-desktop serve` over MCP exactly like an agent would, and checks
outcomes through oracles an agent cannot fake: window titles reported by the
compositor and files created by commands typed into a terminal.

Run on the desktop machine (needs WAYLAND_DISPLAY, sway IPC, foot, firefox):
    python3 tests/e2e/desktop_e2e.py [path/to/slate-desktop]
"""
import base64, json, os, subprocess, sys, tempfile, time

BIN = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else "slate-desktop"
FAILS = []

class Mcp:
    def __init__(self):
        self.p = subprocess.Popen([BIN, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        self.call("initialize")
    def call(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("server exited")
            m = json.loads(line)
            if m.get("id") == self.n:
                return m
    def tool(self, name, **args):
        r = self.call("tools/call", {"name": name, "arguments": args})["result"]
        texts = [c["text"] for c in r["content"] if c["type"] == "text"]
        images = [c["data"] for c in r["content"] if c["type"] == "image"]
        if r.get("isError"):
            raise RuntimeError(f"{name}: {texts}")
        return "\n".join(texts), images
    def windows(self):
        return json.loads(self.tool("desktop_windows")[0])
    def close(self):
        self.p.stdin.close(); self.p.wait(timeout=5)

def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    if not cond:
        FAILS.append(name)

def wait_for(pred, timeout=15, step=0.5):
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v:
            return v
        time.sleep(step)
    return None

def find(m, app_id, exclude=()):
    return next((w for w in m.windows() if w["app_id"] == app_id and w["id"] not in exclude), None)

def png_size(b64):
    raw = base64.b64decode(b64)
    return int.from_bytes(raw[16:20], "big"), int.from_bytes(raw[20:24], "big")

def main():
    m = Mcp()
    before = {w["id"] for w in m.windows()}

    # 1. launch a terminal, type a command that creates a file, verify the file.
    marker = os.path.join(tempfile.gettempdir(), f"slate-e2e-{int(time.time())}.txt")
    m.tool("desktop_launch", command="foot")
    foot = wait_for(lambda: find(m, "foot", before))
    check("launch foot appears in windows", foot is not None)
    if foot:
        m.tool("desktop_click", window=foot["id"], x=100, y=100)
        m.tool("desktop_type", text=f"echo agent-typed > {marker}")
        m.tool("desktop_key", combo="Return")
        ok = wait_for(lambda: os.path.exists(marker) and open(marker).read().strip() == "agent-typed", 10)
        check("type + Return in foot creates the file with the right content", bool(ok))
        # unicode typing
        m.tool("desktop_type", text=f"echo 你好-ünïcode >> {marker}")
        m.tool("desktop_key", combo="Return")
        ok = wait_for(lambda: os.path.exists(marker) and "你好-ünïcode" in open(marker).read(), 10)
        check("unicode text arrives intact", bool(ok))
        # key combo: ctrl+u clears the line, so a following Return runs nothing new
        m.tool("desktop_type", text="echo SHOULD-NOT-RUN >> " + marker)
        m.tool("desktop_key", combo="ctrl+u")
        m.tool("desktop_key", combo="Return")
        time.sleep(1)
        check("ctrl+u combo cancels the typed line", "SHOULD-NOT-RUN" not in open(marker).read())
        # screenshot size matches reported geometry (crop)
        text, images = m.tool("desktop_screenshot", window=foot["id"])
        w = find(m, "foot", before)
        if images and w:
            pw, ph = png_size(images[0])
            check("window screenshot size == reported window size", (pw, ph) == (w["width"], w["height"]), f"{pw}x{ph} vs {w['width']}x{w['height']}")
        # close it through the seat
        m.tool("desktop_type", text="exit")
        m.tool("desktop_key", combo="Return")
        gone = wait_for(lambda: find(m, "foot", before) is None, 10)
        check("typed exit closes the terminal", bool(gone))

    # 2. firefox: navigate to a local page and check the compositor-reported title.
    page = os.path.join(tempfile.gettempdir(), "slate-e2e-page.html")
    open(page, "w").write("<html><head><title>SLATE-E2E-TITLE</title></head><body><h1>ok</h1></body></html>")
    before_ff = {w["id"] for w in m.windows()}
    # A private profile and --no-remote guarantee a fresh instance: plain `firefox` would
    # hand the request to an already running Firefox and exit.
    profile = tempfile.mkdtemp(prefix="slate-e2e-ff-")
    m.tool("desktop_launch", command="firefox", args=["--no-remote", "--profile", profile, "about:blank"])
    ff = wait_for(lambda: find(m, "firefox", before_ff), 40)
    check("launch firefox appears in windows", ff is not None)
    if ff:
        time.sleep(2)  # let it finish starting
        m.tool("desktop_click", window=ff["id"], x=ff["width"] // 2, y=ff["height"] // 2)
        if ff["width"] < 700:
            m.tool("desktop_key", combo="super+f")  # sway: fullscreen, so the UI is not clipped
            time.sleep(1)
            ff = find(m, "firefox", before_ff) or ff
        m.tool("desktop_key", combo="ctrl+l")
        m.tool("desktop_type", text=f"file://{page}")
        m.tool("desktop_key", combo="Return")
        titled = wait_for(lambda: (lambda w: w and "SLATE-E2E-TITLE" in w["title"])(find(m, "firefox", before_ff)), 20)
        check("firefox navigates to the typed URL (title oracle)", bool(titled))
        text, images = m.tool("desktop_screenshot", window=ff["id"])
        w = find(m, "firefox", before_ff)
        if images and w:
            pw, ph = png_size(images[0])
            check("firefox screenshot cropped to window geometry", (pw, ph) == (w["width"], w["height"]), f"{pw}x{ph} vs {w['width']}x{w['height']}")
        m.tool("desktop_key", combo="ctrl+q")
        time.sleep(1.5)
        m.tool("desktop_key", combo="Return")  # confirm "Quit Firefox" if it asks
        gone = wait_for(lambda: find(m, "firefox", before_ff) is None, 15)
        check("ctrl+q (+Return) quits the firefox instance", bool(gone))

    m.close()
    print(f"\n{len(FAILS)} failure(s)" if FAILS else "\nall passed")
    sys.exit(1 if FAILS else 0)

if __name__ == "__main__":
    main()
