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


def settled(m, app_id, exclude=(), timeout=8):
    """The window once its geometry stops changing (new windows get resized by the profile)."""
    last = None
    end = time.time() + timeout
    while time.time() < end:
        w = find(m, app_id, exclude)
        if w and last and (w["x"], w["y"], w["width"], w["height"]) == (last["x"], last["y"], last["width"], last["height"]):
            return w
        last = w
        time.sleep(0.4)
    return last

def png_size(b64):
    raw = base64.b64decode(b64)
    return int.from_bytes(raw[16:20], "big"), int.from_bytes(raw[20:24], "big")

def main():
    m = Mcp()
    before = {w["id"] for w in m.windows()}

    # 1. launch a terminal, type a command that creates a file, verify the file.
    marker = os.path.join(tempfile.gettempdir(), f"slate-e2e-{int(time.time())}.txt")
    # An explicit shell: on Slate the login shell is slash, which would send typed
    # text to the agent instead of running it.
    m.tool("desktop_launch", command="foot", args=["bash"], where="here")
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
        # screenshot size matches reported geometry (crop); wait for the window to settle first
        w = settled(m, "foot", before)
        text, images = m.tool("desktop_screenshot", window=foot["id"])
        if images and w:
            pw, ph = png_size(images[0])
            check("window screenshot size == reported window size", (pw, ph) == (w["width"], w["height"]), f"{pw}x{ph} vs {w['width']}x{w['height']}")
        # close it properly
        m.tool("desktop_close", window=foot["id"])
        gone = wait_for(lambda: find(m, "foot", before) is None, 10)
        check("desktop_close closes the terminal", bool(gone))

    # 1b. window management: two terminals side by side, non-overlapping, inside the screen.
    before2 = {w["id"] for w in m.windows()}
    m.tool("desktop_launch", command="foot", args=["bash"], where="here")
    m.tool("desktop_launch", command="foot", args=["bash"], where="here")
    two = wait_for(lambda: (lambda ws: ws if len(ws) >= 2 else None)([w for w in m.windows() if w["app_id"] == "foot" and w["id"] not in before2]), 15)
    check("two terminals launched", bool(two))
    if two:
        a, b = two[0]["id"], two[1]["id"]
        m.tool("desktop_arrange", layout="side_by_side", windows=[a, b])
        time.sleep(1)
        ws = {w["id"]: w for w in m.windows()}
        wa, wb = ws[a], ws[b]
        no_overlap = wa["x"] + wa["width"] <= wb["x"] or wb["x"] + wb["width"] <= wa["x"]
        same_row = abs(wa["y"] - wb["y"]) < 10
        check("side_by_side: windows do not overlap and share a row", no_overlap and same_row, f"{wa['x']},{wa['y']} {wa['width']}x{wa['height']} | {wb['x']},{wb['y']} {wb['width']}x{wb['height']}")
        on_screen = all(w["y"] < 80 and w["y"] + w["height"] <= 800 and w["x"] >= 0 and w["x"] + w["width"] <= 1280 for w in (wa, wb))
        check("side_by_side: windows fill the screen below the panel without overflowing", on_screen, f"y={wa['y']} bottom={wa['y']+wa['height']}")
        # typing with `window` goes to that window even though another one was clicked last
        m.tool("desktop_click", window=a, x=100, y=100, verify=False)
        marker_b = os.path.join(tempfile.gettempdir(), f"slate-e2e-b-{int(time.time())}.txt")
        time.sleep(1)  # let the second shell finish starting
        r, _ = m.tool("desktop_type", window=b, text=f"echo into-b > {marker_b}\n", verify=False)
        okb = wait_for(lambda: os.path.exists(marker_b), 12)
        check("desktop_type with window focuses that window first", bool(okb) and "into foot" in r, f"{r[:60]} file={bool(okb)}")
        m.tool("desktop_window_set", window=a, x=100, y=100, width=500, height=400)
        time.sleep(0.5)
        wa = {w["id"]: w for w in m.windows()}[a]
        check("desktop_window_set moves and resizes", abs(wa["x"] - 100) < 6 and abs(wa["y"] - 100) < 6 and abs(wa["width"] - 500) < 6, f"{wa['x']},{wa['y']} {wa['width']}x{wa['height']}")
        m.tool("desktop_close", window=a)
        m.tool("desktop_close", window=b)
        wait_for(lambda: not [w for w in m.windows() if w["id"] in (a, b)], 10)

    # 2. firefox: navigate to a local page and check the compositor-reported title.
    page = os.path.join(tempfile.gettempdir(), "slate-e2e-page.html")
    page2 = os.path.join(tempfile.gettempdir(), "slate-e2e-page2.html")
    open(page2, "w").write("<html><head><title>SLATE-E2E-PAGE2</title></head><body><p>second</p></body></html>")
    open(page, "w").write(
        "<html><head><title>SLATE-E2E-TITLE</title></head><body><h1>SLATE-E2E-HEADING</h1>"
        f"<p><a href=\"file://{page2}\">SLATE-E2E-LINK</a></p>"
        "<p><label>Name <input id=\"name\" type=\"text\"></label></p></body></html>"
    )
    before_ff = {w["id"] for w in m.windows()}
    # A private profile and --no-remote guarantee a fresh instance: plain `firefox` would
    # hand the request to an already running Firefox and exit.
    profile = tempfile.mkdtemp(prefix="slate-e2e-ff-")
    m.tool("desktop_launch", command="firefox", args=["--no-remote", "--profile", profile, "about:blank"], where="here")
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
        w = settled(m, "firefox", before_ff)
        text, images = m.tool("desktop_screenshot", window=ff["id"])
        if images and w:
            pw, ph = png_size(images[0])
            check("firefox screenshot cropped to window geometry", (pw, ph) == (w["width"], w["height"]), f"{pw}x{ph} vs {w['width']}x{w['height']}")
        # The accessibility tree: text without OCR, elements by name, actions without a pointer.
        try:
            body, _ = m.tool("desktop_read", window=ff["id"])
            check("desktop_read returns the page heading from the accessibility tree", "SLATE-E2E-HEADING" in body, body[:80].replace("\n", " | "))
            def find_link():
                elems, _ = m.tool("desktop_elements", window=ff["id"], query="SLATE-E2E")
                return next((l for l in elems.splitlines() if "link" in l and "SLATE-E2E-LINK" in l), None)
            # Browsers fill their accessibility cache lazily after a load: retry briefly.
            link = wait_for(find_link, 10, 1)
            elems = link or ""
            check("desktop_elements lists the page link with window-relative extents", link is not None and "@" in (link or ""), (link or elems[:80]))
            def find_entry():
                entries, _ = m.tool("desktop_elements", window=ff["id"], query="entry")
                return next((l for l in entries.splitlines() if l.startswith("e") and " entry " in l), None)
            entry = wait_for(find_entry, 10, 1)
            entries = entry or ""
            if entry:
                eid = entry.split()[0]
                m.tool("desktop_element_set_text", id=eid, text="SLATE-E2E-VALUE", verify=False)
                after = wait_for(lambda: (lambda t: t if "SLATE-E2E-VALUE" in t else None)(m.tool("desktop_elements", window=ff["id"], query="entry")[0]), 8, 1)
                check("desktop_element_set_text fills the input (value oracle)", bool(after), (after or "")[:100].replace("\n", " | "))
            else:
                check("desktop_elements finds the text input", False, entries[:80])
            if link:
                m.tool("desktop_element_click", id=link.split()[0], verify=False)
                navigated = wait_for(lambda: (lambda w: w and "SLATE-E2E-PAGE2" in w["title"])(find(m, "firefox", before_ff)), 15)
                check("desktop_element_click follows the link through its accessibility action (title oracle)", bool(navigated))
        except RuntimeError as e:
            check("accessibility tree available for firefox", False, str(e)[:120])
        m.tool("desktop_close", window=ff["id"])
        gone = wait_for(lambda: find(m, "firefox", before_ff) is None, 15)
        check("desktop_close closes the firefox window", bool(gone))

    # The agent's background screen: launch there, show, hide.
    status = json.loads(m.tool("desktop_status")[0])
    if status.get("background_screen"):
        before_bg = {w["id"] for w in m.windows()}
        r, _ = m.tool("desktop_launch", command="foot", args=["-e", "sh", "-c", "sleep 60"], where="background")
        bgw = wait_for(lambda: find(m, "foot", before_bg), 15)
        check("desktop_launch (background) opens the window on the agent's screen", bgw is not None and bgw["location"] == "background", (bgw or {}).get("location", r[:60]))
        if bgw:
            m.tool("desktop_show", window=bgw["id"])
            shown = wait_for(lambda: (lambda w: w and w["location"] == "screen")(find(m, "foot", before_bg)), 10)
            check("desktop_show brings it to the user's screen", bool(shown))
            m.tool("desktop_hide", window=bgw["id"])
            hidden = wait_for(lambda: (lambda w: w and w["location"] == "background")(find(m, "foot", before_bg)), 10)
            check("desktop_hide sends it back", bool(hidden))
            m.tool("desktop_close", window=bgw["id"])
            wait_for(lambda: find(m, "foot", before_bg) is None, 10)
    else:
        print("SKIP background screen: this compositor has no headless output")

    # GTK3 through the tree: Thunar's location entry, set and activated over the bus.
    before_th = {w["id"] for w in m.windows()}
    m.tool("desktop_launch", command="thunar", args=["/tmp"], where="here")
    th = wait_for(lambda: find(m, "thunar", before_th), 20)
    check("launch thunar appears in windows", th is not None)
    if th:
        time.sleep(2)
        try:
            elems, _ = m.tool("desktop_elements", window=th["id"])
            check("thunar exposes named buttons through the accessibility tree", '"Home"' in elems or '"Open Parent"' in elems, elems.splitlines()[0][:80])
            loc = next((l for l in elems.splitlines() if l.startswith("e") and " text " in l and "editable" in l), None)
            if loc:
                eid = loc.split()[0]
                m.tool("desktop_element_set_text", id=eid, text=tempfile.gettempdir(), verify=False)
                m.tool("desktop_element_click", id=eid, action="Activate", verify=False)
                base = os.path.basename(tempfile.gettempdir())
                titled = wait_for(lambda: (lambda w: w and w["title"].startswith(base))(find(m, "thunar", before_th)), 10)
                check("thunar navigates after set_text + Activate on the location entry (title oracle)", bool(titled))
            else:
                check("thunar location entry found in elements", False)
        except RuntimeError as e:
            check("accessibility tree available for thunar", False, str(e)[:120])
        m.tool("desktop_close", window=th["id"])
        wait_for(lambda: find(m, "thunar", before_th) is None, 10)

    m.close()
    print(f"\n{len(FAILS)} failure(s)" if FAILS else "\nall passed")
    sys.exit(1 if FAILS else 0)

if __name__ == "__main__":
    main()
