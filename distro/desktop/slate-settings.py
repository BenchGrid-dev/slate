#!/usr/bin/env python3
"""Slate Settings: the settings app of the Slate desktop.

Pages: Network (Wi-Fi), Bluetooth, Display (resolution, scale, HiDPI, with a
confirm-or-revert countdown), Appearance, Sound, Keyboard (input methods), Power
(battery, power mode, screen lock), Printers, Slate (agent status, memories) and AI.
Display changes are applied live through sway and persisted to
~/.config/slate/sway.d/output.conf, which the Slate sway profile includes.

    slate-settings [PAGE]    open on a page: network, bluetooth, display, power, ...
"""
import glob
import json
import os
import re
import subprocess
import sys
import threading
import time
import tomllib

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

SWAYD = os.path.expanduser("~/.config/slate/sway.d")
OUTPUT_CONF = os.path.join(SWAYD, "output.conf")
SCALES = ["1", "1.25", "1.5", "1.75", "2"]


def swaysock():
    s = os.environ.get("SWAYSOCK")
    if s and os.path.exists(s):
        return s
    runtime = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    cands = glob.glob(os.path.join(runtime, "sway-ipc.*"))
    return cands[0] if cands else None


def swaymsg(*args, as_json=False):
    """Run a sway IPC query (`-t ...`) or command. Commands are passed as one string so
    that words like `--custom` reach sway instead of being parsed by swaymsg."""
    sock = swaysock()
    args = list(args)
    if args and args[0] != "-t":
        args = [" ".join(args)]
    cmd = ["swaymsg"] + (["-s", sock] if sock else []) + args
    out = subprocess.run(cmd, capture_output=True, text=True)
    if as_json:
        return json.loads(out.stdout or "null")
    return out


def run(cmd, timeout=10):
    # Agent CLIs installed per user (npm -g, ~/.local/bin) are not on the session PATH.
    env = dict(os.environ)
    extra = [os.path.expanduser(p) for p in ("~/.npm-global/bin", "~/.local/bin", "~/.cargo/bin")]
    env["PATH"] = ":".join(extra + [env.get("PATH", "")])
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env).stdout.strip()
    except Exception as e:  # noqa: BLE001
        return f"({e})"


class DisplayPage(Gtk.Box):
    def __init__(self, win):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.win = win
        self.outputs = []
        self.previous = None  # (name, mode_str, scale) to revert to

        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Display", description="Changes apply immediately. If the screen looks wrong, wait 15 seconds and it reverts.")
        page.add(group)

        self.output_row = Adw.ComboRow(title="Output")
        self.output_model = Gtk.StringList()
        self.output_row.set_model(self.output_model)
        self.output_row.connect("notify::selected", lambda *_: self.fill_modes())
        group.add(self.output_row)

        self.mode_row = Adw.ComboRow(title="Resolution", subtitle="Physical pixels of the virtual or real screen")
        self.mode_model = Gtk.StringList()
        self.mode_row.set_model(self.mode_model)
        group.add(self.mode_row)

        self.scale_row = Adw.ComboRow(title="Scale", subtitle="1 = native; 2 = HiDPI (needs a 2x resolution to keep the same working area)")
        self.scale_model = Gtk.StringList.new(SCALES)
        self.scale_row.set_model(self.scale_model)
        group.add(self.scale_row)

        self.hidpi_row = Adw.ActionRow(title="HiDPI preset", subtitle="Double the resolution and use scale 2: same layout, twice the sharpness. Needs Retina mode in UTM.")
        btn = Gtk.Button(label="Use 2x", valign=Gtk.Align.CENTER)
        btn.connect("clicked", self.on_hidpi)
        self.hidpi_row.add_suffix(btn)
        group.add(self.hidpi_row)

        actions = Adw.PreferencesGroup()
        page.add(actions)
        row = Adw.ActionRow(title="Apply", subtitle="Try it now; a dialog asks whether to keep it")
        apply = Gtk.Button(label="Apply", valign=Gtk.Align.CENTER, css_classes=["suggested-action"])
        apply.connect("clicked", self.on_apply)
        row.add_suffix(apply)
        actions.add(row)
        row = Adw.ActionRow(title="Reset", subtitle="Back to the screen's native mode at scale 1 and forget the saved setting")
        reset = Gtk.Button(label="Reset", valign=Gtk.Align.CENTER)
        reset.connect("clicked", self.on_reset)
        row.add_suffix(reset)
        actions.add(row)

        self.status = Gtk.Label(xalign=0, margin_start=24, margin_bottom=12, css_classes=["dim-label"])
        self.append(page)
        self.append(self.status)
        self.refresh()

    # ---- data
    def refresh(self):
        # The agent's background screen (HEADLESS-*) is not a display the person can see.
        self.outputs = [o for o in (swaymsg("-t", "get_outputs", as_json=True) or []) if not o["name"].startswith("HEADLESS-")]
        self.output_model.splice(0, self.output_model.get_n_items(), [o["name"] for o in self.outputs])
        self.fill_modes()

    def current_output(self):
        i = self.output_row.get_selected()
        return self.outputs[i] if 0 <= i < len(self.outputs) else None

    def fill_modes(self):
        o = self.current_output()
        if not o:
            return
        modes = []
        for m in o.get("modes", []):
            s = f'{m["width"]}x{m["height"]}'
            if s not in modes:
                modes.append(s)
        cur = o.get("current_mode") or {}
        cur_s = f'{cur.get("width", 0)}x{cur.get("height", 0)}'
        if cur_s not in modes:
            modes.insert(0, cur_s)
        self.mode_model.splice(0, self.mode_model.get_n_items(), modes)
        self.mode_row.set_selected(modes.index(cur_s))
        scale = str(o.get("scale", 1)).rstrip("0").rstrip(".")
        if scale in SCALES:
            self.scale_row.set_selected(SCALES.index(scale))
        self.status.set_text(f'{o["name"]}: {cur_s} at scale {scale} → working area {o["rect"]["width"]}x{o["rect"]["height"]}')

    # ---- actions
    def selected_mode(self):
        i = self.mode_row.get_selected()
        return self.mode_model.get_string(i) if i >= 0 else None

    def on_hidpi(self, *_):
        o = self.current_output()
        if not o:
            return
        w, h = o["rect"]["width"], o["rect"]["height"]
        custom = f"{w * 2}x{h * 2}"
        if custom not in [self.mode_model.get_string(i) for i in range(self.mode_model.get_n_items())]:
            self.mode_model.append(custom)
        self.mode_row.set_selected([self.mode_model.get_string(i) for i in range(self.mode_model.get_n_items())].index(custom))
        self.scale_row.set_selected(SCALES.index("2"))
        self.on_apply()

    def apply_cmd(self, name, mode, scale, custom=False):
        mode_arg = ["mode", "--custom", mode] if custom else ["mode", mode]
        r = swaymsg("output", name, *mode_arg, "scale", scale)
        ok = r.returncode == 0 and '"success": true' in r.stdout
        return ok, (r.stdout + r.stderr).strip()

    def on_apply(self, *_):
        o = self.current_output()
        mode = self.selected_mode()
        if not o or not mode:
            return
        scale = SCALES[self.scale_row.get_selected()]
        cur = o.get("current_mode") or {}
        self.previous = (o["name"], f'{cur.get("width")}x{cur.get("height")}', str(o.get("scale", 1)))
        listed = any(f'{m["width"]}x{m["height"]}' == mode for m in o.get("modes", []))
        ok, msg = self.apply_cmd(o["name"], mode, scale, custom=not listed)
        if not ok:
            self.status.set_text(f"sway refused: {msg[:200]}")
            return
        self.confirm(o["name"], mode, scale, custom=not listed)

    def confirm(self, name, mode, scale, custom):
        dialog = Adw.MessageDialog(transient_for=self.win, heading="Keep these display settings?", body=f"{name}: {mode} at scale {scale}. Reverting in 15 seconds unless you keep them.")
        dialog.add_response("revert", "Revert")
        dialog.add_response("keep", "Keep")
        dialog.set_response_appearance("keep", Adw.ResponseAppearance.SUGGESTED)
        dialog.set_default_response("keep")
        remaining = {"n": 15}

        def tick():
            if not dialog.get_visible():
                return False
            remaining["n"] -= 1
            dialog.set_body(f"{name}: {mode} at scale {scale}. Reverting in {remaining['n']} seconds unless you keep them.")
            if remaining["n"] <= 0:
                dialog.response("revert")
                return False
            return True

        def on_response(_d, resp):
            if resp == "keep":
                self.persist(name, mode, scale, custom)
                self.status.set_text(f"Saved to {OUTPUT_CONF}")
            else:
                self.revert()
            self.refresh()

        dialog.connect("response", on_response)
        GLib.timeout_add_seconds(1, tick)
        dialog.present()

    def revert(self):
        if not self.previous:
            return
        name, mode, scale = self.previous
        self.apply_cmd(name, mode, scale)
        self.status.set_text(f"Reverted to {mode} at scale {scale}")

    def persist(self, name, mode, scale, custom):
        os.makedirs(SWAYD, exist_ok=True)
        mode_arg = f"mode --custom {mode}" if custom else f"mode {mode}"
        with open(OUTPUT_CONF, "w") as f:
            f.write(f"# written by Slate Settings\noutput {name} {mode_arg} scale {scale}\n")

    def on_reset(self, *_):
        o = self.current_output()
        if not o:
            return
        modes = o.get("modes", [])
        native = f'{modes[0]["width"]}x{modes[0]["height"]}' if modes else "1280x800"
        self.apply_cmd(o["name"], native, "1")
        try:
            os.remove(OUTPUT_CONF)
        except FileNotFoundError:
            pass
        self.status.set_text(f"Reset to {native} at scale 1; saved setting removed")
        self.refresh()


class SoundPage(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Sound")
        page.add(group)
        row = Adw.ActionRow(title="Output volume")
        self.scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0, 100, 5)
        self.scale.set_hexpand(True)
        self.scale.set_size_request(240, -1)
        vol = run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"])
        try:
            self.scale.set_value(float(vol.split()[1]) * 100)
        except Exception:  # noqa: BLE001
            pass
        self.scale.connect("value-changed", lambda s: run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", f"{int(s.get_value())}%"]))
        row.add_suffix(self.scale)
        group.add(row)
        self.append(page)


class SlatePage(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        page = Adw.PreferencesPage()
        status = Adw.PreferencesGroup(title="Slate")
        page.add(status)
        for line in run(["slate", "status"]).splitlines():
            status.add(Adw.ActionRow(title=line))
        try:
            st = json.loads(run(["slate", "agent-status"]))
            status.add(Adw.ActionRow(title="Agent", subtitle=f'{st.get("text", "")} · {st.get("tooltip", "")}'))
        except Exception:  # noqa: BLE001
            pass
        self.mem_group = Adw.PreferencesGroup(title="Memories", description="Things you asked Slate to remember. Every task starts with these.")
        page.add(self.mem_group)
        self.fill_memories()
        self.append(page)

    def fill_memories(self):
        for line in run(["slate", "memories"]).splitlines():
            parts = line.split(None, 2)
            if len(parts) < 3:
                continue
            _ts, mid, text = parts
            row = Adw.ActionRow(title=text)
            btn = Gtk.Button(icon_name="user-trash-symbolic", valign=Gtk.Align.CENTER, css_classes=["flat"])

            def forget(_b, mid=mid, row=row):
                run(["slate", "forget", mid])
                self.mem_group.remove(row)

            btn.connect("clicked", forget)
            row.add_suffix(btn)
            self.mem_group.add(row)


SLASH_CONFIG = os.path.expanduser("~/.config/slate/slash.toml")


def load_slash_config():
    try:
        with open(SLASH_CONFIG, "rb") as f:
            return tomllib.load(f)
    except Exception:  # noqa: BLE001
        return {}


def toml_value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    return json.dumps(str(v))


def save_slash_config(cfg):
    """slash.toml is small and flat (scalars at the top, one level of tables): write it by hand."""
    os.makedirs(os.path.dirname(SLASH_CONFIG), exist_ok=True)
    lines = ["# written by Slate Settings; slash reads this at start"]
    for k, v in cfg.items():
        if not isinstance(v, dict):
            lines.append(f"{k} = {toml_value(v)}")
    for k, v in cfg.items():
        if isinstance(v, dict):
            lines.append(f"\n[{k}]")
            for k2, v2 in v.items():
                lines.append(f"{k2} = {toml_value(v2)}")
    with open(SLASH_CONFIG, "w") as f:
        f.write("\n".join(lines) + "\n")


class AIPage(Gtk.Box):
    """Slate AI: which agent runs slash, how it is signed in, how chatty it is."""

    BACKENDS = ["claude", "codex"]

    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.cfg = load_slash_config()
        page = Adw.PreferencesPage()
        self.append(page)

        agent = Adw.PreferencesGroup(title="Agent", description="Slate drives the official Claude Code or Codex CLI with your own subscription. Changes apply to new sessions.")
        page.add(agent)
        self.backend = Adw.ComboRow(title="Backend", model=Gtk.StringList.new(["Claude Code", "Codex"]))
        self.backend.set_selected(self.BACKENDS.index(self.cfg.get("backend", "claude")) if self.cfg.get("backend", "claude") in self.BACKENDS else 0)
        self.backend.connect("notify::selected", lambda *_: self.set_value(["backend"], self.BACKENDS[self.backend.get_selected()]))
        agent.add(self.backend)
        self.model = Adw.EntryRow(title="Claude model (sonnet, opus, haiku or a model id)")
        self.model.set_text(str(self.cfg.get("claude", {}).get("model") or ""))
        self.model.connect("apply", lambda *_: self.set_value(["claude", "model"], self.model.get_text().strip() or None))
        self.model.set_show_apply_button(True)
        agent.add(self.model)
        self.codex_model = Adw.EntryRow(title="Codex model (empty keeps Codex's default)")
        self.codex_model.set_text(str(self.cfg.get("codex", {}).get("model") or ""))
        self.codex_model.connect("apply", lambda *_: self.set_value(["codex", "model"], self.codex_model.get_text().strip() or None))
        self.codex_model.set_show_apply_button(True)
        agent.add(self.codex_model)

        signin = Adw.PreferencesGroup(title="Sign in", description="Sign-in happens in the agent's own CLI (a terminal opens). Slate never sees your credentials.")
        page.add(signin)
        self.claude_row = Adw.ActionRow(title="Claude Code")
        self.add_auth_buttons(self.claude_row, ["claude", "auth", "login"], ["claude", "auth", "logout"])
        signin.add(self.claude_row)
        self.codex_row = Adw.ActionRow(title="Codex")
        self.add_auth_buttons(self.codex_row, ["codex", "login"], ["codex", "logout"])
        signin.add(self.codex_row)
        self.refresh_auth()

        behaviour = Adw.PreferencesGroup(title="Behaviour")
        page.add(behaviour)
        self.verbose = Adw.SwitchRow(title="Verbose log", subtitle="Show raw agent events, tool details and session ids in slash (also /verbose)")
        self.verbose.set_active(bool(self.cfg.get("verbose", False)))
        self.verbose.connect("notify::active", lambda *_: self.set_value(["verbose"], self.verbose.get_active()))
        behaviour.add(self.verbose)
        self.auto = Adw.SwitchRow(title="Bypass approvals by default", subtitle="Start every session as /auto on: actions that would ask for confirmation just run (still audited, still undoable)")
        self.auto.set_active(bool(self.cfg.get("auto_approve", False)))
        self.auto.connect("notify::active", lambda *_: self.set_value(["auto_approve"], self.auto.get_active()))
        behaviour.add(self.auto)
        restart = Adw.ActionRow(title="Restart the Slate prompt", subtitle="Applies the settings above to the prompt in the top-right corner")
        btn = Gtk.Button(label="Restart", valign=Gtk.Align.CENTER)
        btn.connect("clicked", self.restart_prompt)
        restart.add_suffix(btn)
        behaviour.add(restart)
        config_row = Adw.ActionRow(title="Configuration file", subtitle=SLASH_CONFIG)
        behaviour.add(config_row)

    def set_value(self, path, value):
        node = self.cfg
        for k in path[:-1]:
            node = node.setdefault(k, {})
        if value is None:
            node.pop(path[-1], None)
        else:
            node[path[-1]] = value
        save_slash_config(self.cfg)

    def add_auth_buttons(self, row, login_cmd, logout_cmd):
        login = Gtk.Button(label="Sign in…", valign=Gtk.Align.CENTER)
        login.connect("clicked", lambda *_: self.run_in_terminal(login_cmd))
        logout = Gtk.Button(label="Sign out", valign=Gtk.Align.CENTER, css_classes=["flat"])
        logout.connect("clicked", lambda *_: (run(logout_cmd), self.refresh_auth()))
        row.add_suffix(login)
        row.add_suffix(logout)

    def run_in_terminal(self, cmd):
        # The CLI opens a browser and waits for the code; leave a shell so the window stays readable.
        subprocess.Popen(["foot", "-e", "sh", "-c", " ".join(cmd) + '; echo; echo "Done. You can close this window."; exec ${SHELL:-sh}'])
        GLib.timeout_add_seconds(15, lambda: (self.refresh_auth(), False)[1])

    def refresh_auth(self):
        def work():
            claude = run(["claude", "auth", "status"])
            try:
                st = json.loads(claude)
                who = st.get("email") or st.get("account") or st.get("authMethod") or ""
                claude_text = f"Signed in ({who})" if st.get("loggedIn") else "Not signed in"
            except Exception:  # noqa: BLE001
                claude_text = "claude not installed" if not claude else claude.splitlines()[0]
            codex = run(["codex", "login", "status"])
            codex_text = codex.splitlines()[0] if codex else "codex not installed"
            GLib.idle_add(self.claude_row.set_subtitle, claude_text)
            GLib.idle_add(self.codex_row.set_subtitle, codex_text)

        threading.Thread(target=work, daemon=True).start()

    def restart_prompt(self, *_):
        subprocess.run(["pkill", "-f", "bin/.slate-shell-wrapped"], check=False)
        subprocess.Popen(["slate-shell", "--hidden"])


class AppearancePage(Gtk.Box):
    """Dark or light, and the desktop background; applied through slate-theme."""

    def __init__(self, window):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.window = window
        page = Adw.PreferencesPage()
        self.append(page)
        try:
            self.state = json.loads(run(["slate-theme", "status"]) or "{}")
        except Exception:  # noqa: BLE001
            self.state = {}

        theme = Adw.PreferencesGroup(title="Theme", description="Applies to windows, the panel, terminals, notifications and the launcher at once.")
        page.add(theme)
        self.theme_row = Adw.ComboRow(title="Appearance", model=Gtk.StringList.new(["Dark", "Light"]))
        self.theme_row.set_selected(1 if self.state.get("theme") == "light" else 0)
        self.theme_row.connect("notify::selected", self.on_theme)
        theme.add(self.theme_row)
        self.clear_row = Adw.SwitchRow(
            title="Reduce transparency",
            subtitle="Solid panels, dock and menus instead of frosted glass. Easier to read, and lighter on machines without a graphics card.",
        )
        self.clear_row.set_active(self.state.get("transparency", "on") == "reduced")
        self.clear_row.connect("notify::active", self.on_transparency)
        theme.add(self.clear_row)

        wall = Adw.PreferencesGroup(title="Desktop background")
        page.add(wall)
        self.wall_row = Adw.ActionRow(title="Image", subtitle=self.wallpaper_label())
        choose = Gtk.Button(label="Choose…", valign=Gtk.Align.CENTER)
        choose.connect("clicked", self.on_choose)
        reset = Gtk.Button(label="Default", valign=Gtk.Align.CENTER, css_classes=["flat"])
        reset.connect("clicked", lambda *_: self.set_wallpaper("default"))
        self.wall_row.add_suffix(choose)
        self.wall_row.add_suffix(reset)
        wall.add(self.wall_row)

    def wallpaper_label(self):
        w = self.state.get("wallpaper", "default")
        if w in ("default", "/etc/slate/wallpaper.png"):
            return "SlateOS (follows the appearance)"
        return w

    def on_theme(self, *_):
        name = "light" if self.theme_row.get_selected() == 1 else "dark"
        run(["slate-theme", name], timeout=20)

    def on_transparency(self, *_):
        run(["slate-theme", "transparency", "reduced" if self.clear_row.get_active() else "on"], timeout=20)

    def on_choose(self, *_):
        dialog = Gtk.FileDialog(title="Choose a background image")
        f = Gtk.FileFilter()
        f.set_name("Images")
        for m in ("image/png", "image/jpeg", "image/webp"):
            f.add_mime_type(m)
        filters = Gio.ListStore.new(Gtk.FileFilter)
        filters.append(f)
        dialog.set_filters(filters)
        pictures = os.path.expanduser("~/Pictures")
        if os.path.isdir(pictures):
            dialog.set_initial_folder(Gio.File.new_for_path(pictures))

        def done(d, result):
            try:
                file = d.open_finish(result)
            except Exception:  # noqa: BLE001
                return
            if file:
                self.set_wallpaper(file.get_path())

        dialog.open(self.window, None, done)

    def set_wallpaper(self, path):
        out = run(["slate-theme", "wallpaper", path], timeout=20)
        try:
            self.state = json.loads(out or "{}")
        except Exception:  # noqa: BLE001
            pass
        self.wall_row.set_subtitle(self.wallpaper_label())


# ---- shared helpers for the hardware pages ----------------------------------------

def system_bus():
    return Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def dbus_call(bus, name, path, iface, method, args=None, reply=None, timeout=10000):
    r = bus.call_sync(name, path, iface, method, args,
                      GLib.VariantType(reply) if reply else None, Gio.DBusCallFlags.NONE, timeout, None)
    return r.unpack() if r is not None else None


def dbus_get(bus, name, path, iface, prop):
    try:
        return dbus_call(bus, name, path, "org.freedesktop.DBus.Properties", "Get",
                         GLib.Variant("(ss)", (iface, prop)), "(v)")[0]
    except GLib.Error:
        return None


def dbus_error(e):
    """The readable part of a GLib.Error from D-Bus."""
    msg = getattr(e, "message", str(e))
    return msg.split(": ", 2)[-1] if msg.startswith("GDBus.Error") else msg


def boxed_list(*rows):
    lb = Gtk.ListBox(css_classes=["boxed-list"], selection_mode=Gtk.SelectionMode.NONE)
    for r in rows:
        lb.append(r)
    return lb


class Page(Gtk.Box):
    """A preferences page with toasts and rows that can be emptied and refilled."""

    def __init__(self, win):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.win = win
        self.page = Adw.PreferencesPage()
        self.toasts = Adw.ToastOverlay(child=self.page, vexpand=True)
        self.append(self.toasts)
        self.timer = None
        self.connect("map", lambda *_: self.on_shown())
        self.connect("unmap", lambda *_: self.on_hidden())

    def toast(self, text):
        self.toasts.add_toast(Adw.Toast(title=GLib.markup_escape_text(text), timeout=4))

    def every(self, seconds, fn):
        """Call fn now and every `seconds` while the page is on screen."""
        self.periodic = (seconds, fn)

    def on_shown(self):
        p = getattr(self, "periodic", None)
        if p and self.timer is None:
            p[1]()
            self.timer = GLib.timeout_add_seconds(p[0], lambda: (p[1](), True)[1])

    def on_hidden(self):
        if self.timer is not None:
            GLib.source_remove(self.timer)
            self.timer = None

    @staticmethod
    def clear(group, rows):
        for r in rows:
            group.remove(r)
        rows.clear()


def in_thread(work, done):
    """Run work() off the main loop, then done(result) on it."""
    def go():
        try:
            res = work()
        except Exception as e:  # noqa: BLE001
            res = e
        GLib.idle_add(lambda: (done(res), False)[1])
    threading.Thread(target=go, daemon=True).start()


# ---- Network ----------------------------------------------------------------------

def split_terse(line):
    """The fields of an `nmcli -t` line: ':' separates them, '\\:' and '\\\\' are escapes."""
    out, cur, esc = [], [], False
    for ch in line:
        if esc:
            cur.append(ch)
            esc = False
        elif ch == "\\":
            esc = True
        elif ch == ":":
            out.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    out.append("".join(cur))
    return out


def nmcli(*args, timeout=10):
    try:
        out = subprocess.run(["nmcli", "-t", *args], capture_output=True, text=True, timeout=timeout).stdout
    except Exception:  # noqa: BLE001
        return []
    return [split_terse(line) for line in out.splitlines() if line]


def signal_icon(strength):
    for level, name in ((75, "excellent"), (50, "good"), (25, "ok"), (1, "weak")):
        if strength >= level:
            return f"network-wireless-signal-{name}-symbolic"
    return "network-wireless-signal-none-symbolic"


NM = "org.freedesktop.NetworkManager"


class NetworkPage(Page):
    """Wi-Fi (choose a network, type its password), wired connections, and the full
    NetworkManager editor for the rest (VPN, static addresses, enterprise Wi-Fi)."""

    def __init__(self, win):
        super().__init__(win)
        self.busy = False
        self.wifi_dev = None

        self.wifi = Adw.PreferencesGroup(title="Wi-Fi")
        self.radio = Gtk.Switch(valign=Gtk.Align.CENTER)
        self.radio_handler = self.radio.connect("notify::active", self.on_radio)
        self.wifi.set_header_suffix(self.radio)
        self.page.add(self.wifi)
        self.wifi_rows = []

        self.nets = Adw.PreferencesGroup(title="Other Networks")
        head = Gtk.Box(spacing=6)
        self.scan_spinner = Adw.Spinner(visible=False)
        rescan = Gtk.Button(icon_name="view-refresh-symbolic", css_classes=["flat"], tooltip_text="Look for networks")
        rescan.connect("clicked", lambda *_: self.rescan())
        head.append(self.scan_spinner)
        head.append(rescan)
        self.nets.set_header_suffix(head)
        self.page.add(self.nets)
        self.net_rows = []
        self.other = Adw.ButtonRow(title="Other Network…", start_icon_name="list-add-symbolic")
        self.other.connect("activated", lambda *_: self.ask_hidden())

        self.wired = Adw.PreferencesGroup(title="Wired")
        self.page.add(self.wired)
        self.wired_rows = []

        adv = Adw.PreferencesGroup()
        row = Adw.ActionRow(title="Network Connections", subtitle="VPN, fixed addresses, proxies and enterprise Wi-Fi", activatable=True)
        row.add_suffix(Gtk.Image.new_from_icon_name("go-next-symbolic"))
        row.connect("activated", lambda *_: subprocess.Popen(["nm-connection-editor"]))
        adv.add(row)
        self.page.add(adv)

        self.install_actions()
        self.every(8, self.refresh)

    def on_shown(self):
        super().on_shown()
        self.rescan()

    # -- reading
    def saved_ssids(self):
        ssids = {}
        for f in nmcli("-f", "NAME,UUID,TYPE", "connection", "show"):
            if len(f) >= 3 and f[2] == "802-11-wireless":
                ssid, _, mode = run(["nmcli", "-g", "802-11-wireless.ssid,802-11-wireless.mode", "connection", "show", "uuid", f[1]]).partition("\n")
                if mode.strip() != "ap":  # a hotspot this computer serves
                    ssids[ssid or f[0]] = f[1]
        return ssids

    def refresh(self):
        if self.busy:
            return
        devices = nmcli("-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status")
        wifi = [d for d in devices if len(d) >= 4 and d[1] == "wifi"]
        # A card that serves a hotspot is not the one to join networks with.
        clients = [d for d in wifi if not d[3] or run(["nmcli", "-g", "802-11-wireless.mode", "connection", "show", "id", d[3]]) != "ap"]
        self.wifi_dev = (clients or wifi)[0][0] if wifi else None
        radio_on = run(["nmcli", "radio", "wifi"]) == "enabled"
        self.radio.handler_block(self.radio_handler)
        self.radio.set_active(radio_on)
        self.radio.handler_unblock(self.radio_handler)
        self.radio.set_sensitive(self.wifi_dev is not None)

        self.clear(self.wifi, self.wifi_rows)
        self.clear(self.nets, self.net_rows)
        if self.other.get_parent():
            self.nets.remove(self.other)
        self.nets.set_visible(bool(self.wifi_dev) and radio_on)
        if not self.wifi_dev:
            self.add_row(self.wifi, self.wifi_rows, Adw.ActionRow(title="No Wi-Fi adapter", subtitle="This computer has no wireless network card, or it needs firmware"))
        elif not radio_on:
            self.add_row(self.wifi, self.wifi_rows, Adw.ActionRow(title="Wi-Fi is off"))
        else:
            saved = self.saved_ssids()
            seen = {}
            for f in nmcli("-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "ifname", self.wifi_dev):
                if len(f) < 4 or not f[1]:
                    continue
                inuse, ssid, sig, sec = f[0] == "*", f[1], int(f[2] or 0), f[3]
                cur = seen.get(ssid)
                if not cur or inuse or (sig > cur["signal"] and not cur["inuse"]):
                    seen[ssid] = {"inuse": inuse or (cur or {}).get("inuse", False), "signal": sig, "security": sec}
            current = [s for s, n in seen.items() if n["inuse"]]
            for ssid in current:
                n = seen[ssid]
                row = Adw.ActionRow(title=GLib.markup_escape_text(ssid), subtitle="Connected")
                row.add_prefix(Gtk.Image.new_from_icon_name(signal_icon(n["signal"])))
                menu = Gio.Menu()
                menu.append("Disconnect", f"net.disconnect::{ssid}")
                if ssid in saved:
                    menu.append("Forget This Network", f"net.forget::{saved[ssid]}")
                btn = Gtk.MenuButton(icon_name="view-more-symbolic", menu_model=menu, valign=Gtk.Align.CENTER, css_classes=["flat"])
                row.add_suffix(btn)
                self.add_row(self.wifi, self.wifi_rows, row)
            if not current:
                self.add_row(self.wifi, self.wifi_rows, Adw.ActionRow(title="Not connected", subtitle="Choose a network below"))
            for ssid, n in sorted(seen.items(), key=lambda kv: -kv[1]["signal"]):
                if n["inuse"]:
                    continue
                secured = bool(n["security"]) and n["security"] != "--"
                row = Adw.ActionRow(title=GLib.markup_escape_text(ssid), activatable=True,
                                    subtitle="Saved" if ssid in saved else "")
                row.add_prefix(Gtk.Image.new_from_icon_name(signal_icon(n["signal"])))
                if secured:
                    row.add_suffix(Gtk.Image.new_from_icon_name("channel-secure-symbolic"))
                row.connect("activated", lambda _r, s=ssid, sec=n["security"], u=saved.get(ssid): self.connect_to(s, sec, u))
                self.add_row(self.nets, self.net_rows, row)
            self.nets.add(self.other)

        self.clear(self.wired, self.wired_rows)
        for d in devices:
            if len(d) >= 4 and d[1] == "ethernet":
                state = "Connected" if d[2] == "connected" else d[2].capitalize()
                addr = run(["nmcli", "-g", "IP4.ADDRESS", "device", "show", d[0]]).split("|")[0].split("/")[0]
                row = Adw.ActionRow(title=d[0], subtitle=f"{state} · {addr}" if addr and d[2] == "connected" else state)
                row.add_prefix(Gtk.Image.new_from_icon_name("network-wired-symbolic"))
                self.add_row(self.wired, self.wired_rows, row)
        self.wired.set_visible(bool(self.wired_rows))

    @staticmethod
    def add_row(group, rows, row):
        group.add(row)
        rows.append(row)

    def rescan(self):
        if not self.wifi_dev:
            self.refresh()
            return
        self.scan_spinner.set_visible(True)
        in_thread(lambda: run(["nmcli", "device", "wifi", "rescan", "ifname", self.wifi_dev], timeout=15),
                  lambda _r: (self.scan_spinner.set_visible(False), self.refresh()))

    # -- actions
    def on_radio(self, sw, *_):
        run(["nmcli", "radio", "wifi", "on" if sw.get_active() else "off"])
        GLib.timeout_add(800, lambda: (self.refresh(), False)[1])

    def disconnect_wifi(self):
        if self.wifi_dev:
            run(["nmcli", "device", "disconnect", self.wifi_dev])
        self.refresh()

    def forget(self, uuid):
        run(["nmcli", "connection", "delete", "uuid", uuid])
        self.refresh()

    def connect_to(self, ssid, security, saved_uuid):
        secured = bool(security) and security != "--"
        if "802.1X" in security and not saved_uuid:
            self.toast(f"{ssid} uses enterprise sign-in: set it up in Network Connections")
            return
        if saved_uuid:
            self.run_connect(ssid, ["nmcli", "--wait", "40", "connection", "up", "uuid", saved_uuid, "ifname", self.wifi_dev],
                             ask_on_secrets=secured, security=security)
        elif not secured:
            self.run_connect(ssid, ["nmcli", "--wait", "40", "device", "wifi", "connect", ssid, "ifname", self.wifi_dev])
        else:
            self.ask_password(ssid, security)

    def run_connect(self, ssid, cmd, ask_on_secrets=False, security=""):
        self.busy = True
        self.toast(f"Connecting to {ssid}…")

        def work():
            return subprocess.run(cmd, capture_output=True, text=True, timeout=60)

        def done(r):
            self.busy = False
            if isinstance(r, Exception) or r.returncode != 0:
                err = (str(r) if isinstance(r, Exception) else (r.stderr or r.stdout)).strip()
                if ask_on_secrets and ("ecret" in err or "password" in err.lower()):
                    self.ask_password(ssid, security)
                else:
                    self.toast(f"Could not connect to {ssid}: {err.splitlines()[-1] if err else 'failed'}")
            else:
                self.toast(f"Connected to {ssid}")
            self.refresh()

        in_thread(work, done)

    def ask_password(self, ssid, security, hidden=False):
        dialog = Adw.AlertDialog(heading=f"Join “{ssid}”" if ssid else "Join a Network",
                                 body="Enter the network's password.")
        name = Adw.EntryRow(title="Network name", activates_default=True)
        pw = Adw.PasswordEntryRow(title="Password", activates_default=True)
        rows = [name, pw] if hidden else [pw]
        dialog.set_extra_child(boxed_list(*rows))
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("join", "Join")
        dialog.set_response_appearance("join", Adw.ResponseAppearance.SUGGESTED)
        dialog.set_default_response("join")
        dialog.set_close_response("cancel")

        def on_response(_d, resp):
            if resp != "join":
                return
            target = name.get_text().strip() if hidden else ssid
            if not target:
                return
            self.join(target, pw.get_text(), hidden, security)

        dialog.connect("response", on_response)
        dialog.present(self.win)
        (name if hidden else pw).grab_focus()

    def ask_hidden(self):
        self.ask_password("", "", hidden=True)

    def join(self, ssid, password, hidden, security=""):
        """A new network with a password, through NetworkManager's D-Bus API: the password
        never appears on a command line. The security type comes from what the access
        point advertises (nmcli's SECURITY field); a failed attempt is not kept."""
        self.busy = True
        self.toast(f"Connecting to {ssid}…")
        dev = self.wifi_dev
        advertised = security

        def work():
            bus = system_bus()
            dev_path = dbus_call(bus, NM, "/org/freedesktop/NetworkManager", NM, "GetDeviceByIpIface",
                                 GLib.Variant("(s)", (dev,)), "(o)")[0]
            wireless = {"ssid": GLib.Variant("ay", ssid.encode()), "mode": GLib.Variant("s", "infrastructure")}
            if hidden:
                wireless["hidden"] = GLib.Variant("b", True)
            sec = {}
            words = advertised.split()
            if "OWE" in words:
                sec["key-mgmt"] = GLib.Variant("s", "owe")
            elif "WEP" in words:
                sec.update({"key-mgmt": GLib.Variant("s", "none"), "wep-key0": GLib.Variant("s", password),
                            "wep-key-type": GLib.Variant("u", 2)})  # 2: a passphrase
            elif password:
                # WPA3 only: SAE; WPA2 (alone or with WPA3) and older: a pre-shared key.
                only_wpa3 = "WPA3" in words and not any(w in words for w in ("WPA1", "WPA2", "WPA"))
                sec.update({"key-mgmt": GLib.Variant("s", "sae" if only_wpa3 else "wpa-psk"),
                            "psk": GLib.Variant("s", password)})
            settings = {"connection": {"type": GLib.Variant("s", "802-11-wireless"), "id": GLib.Variant("s", ssid)},
                        "802-11-wireless": wireless}
            if sec:
                settings["802-11-wireless-security"] = sec
            conn, active = dbus_call(bus, NM, "/org/freedesktop/NetworkManager", NM, "AddAndActivateConnection",
                                     GLib.Variant("(a{sa{sv}}oo)", (settings, dev_path, "/")), "(oo)", timeout=30000)
            # NM_ACTIVE_CONNECTION_STATE: 2 activated, 3 deactivating, 4 deactivated
            for _ in range(90):
                time.sleep(0.5)
                state = dbus_get(bus, NM, active, NM + ".Connection.Active", "State")
                if state == 2:
                    return None
                if state is None or state >= 3:
                    break
            try:
                dbus_call(bus, NM, conn, NM + ".Settings.Connection", "Delete")
            except GLib.Error:
                pass
            return "wrong password, or the network did not accept this computer"

        def done(res):
            self.busy = False
            if res is None:
                self.toast(f"Connected to {ssid}")
            else:
                self.toast(f"Could not connect to {ssid}: {dbus_error(res) if isinstance(res, Exception) else res}")
            self.refresh()

        in_thread(work, done)

    def install_actions(self):
        group = Gio.SimpleActionGroup()
        for name, fn in (("disconnect", lambda _a, _p: self.disconnect_wifi()),
                         ("forget", lambda _a, p: self.forget(p.get_string()))):
            act = Gio.SimpleAction.new(name, GLib.VariantType("s"))
            act.connect("activate", fn)
            group.add_action(act)
        self.insert_action_group("net", group)


# ---- Bluetooth --------------------------------------------------------------------

BLUEZ = "org.bluez"
AGENT_PATH = "/dev/benchgrid/slate/settings/agent"
AGENT_XML = """<node><interface name="org.bluez.Agent1">
<method name="Release"/>
<method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
<method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
<method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
<method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/><arg type="q" direction="in"/></method>
<method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
<method name="RequestAuthorization"><arg type="o" direction="in"/></method>
<method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
<method name="Cancel"/>
</interface></node>"""
DEVICE_ICONS = {
    "audio-headset": "audio-headset-symbolic", "audio-headphones": "audio-headphones-symbolic",
    "audio-card": "audio-speakers-symbolic", "input-keyboard": "input-keyboard-symbolic",
    "input-mouse": "input-mouse-symbolic", "input-tablet": "input-tablet-symbolic",
    "input-gaming": "input-gaming-symbolic", "phone": "phone-symbolic", "computer": "computer-symbolic",
    "video-display": "video-display-symbolic", "printer": "printer-symbolic", "camera-photo": "camera-photo-symbolic",
}


class BluetoothPage(Page):
    """Turn Bluetooth on or off, pair with nearby devices, connect and forget paired ones.
    While the page is open the computer looks for devices and can be found by them, and
    Settings answers pairing requests (codes to confirm or type)."""

    def __init__(self, win):
        super().__init__(win)
        self.bus = None
        self.adapter = None
        self.dialog = None
        self.pending = set()
        self.refresh_queued = False
        try:
            self.bus = system_bus()
        except GLib.Error:
            pass

        self.main = Adw.PreferencesGroup(title="Bluetooth")
        self.power = Gtk.Switch(valign=Gtk.Align.CENTER)
        self.power_handler = self.power.connect("notify::active", self.on_power)
        self.main.set_header_suffix(self.power)
        self.page.add(self.main)
        self.status = Adw.ActionRow(title="Bluetooth")
        self.main.add(self.status)

        self.mine = Adw.PreferencesGroup(title="My Devices")
        self.page.add(self.mine)
        self.mine_rows = []
        self.nearby = Adw.PreferencesGroup(title="Nearby Devices", description="Make the device you want to pair discoverable (often by holding its pairing button).")
        self.spinner = Adw.Spinner()
        self.nearby.set_header_suffix(self.spinner)
        self.page.add(self.nearby)
        self.nearby_rows = []

        if self.bus:
            for iface, member in (("org.freedesktop.DBus.ObjectManager", None), ("org.freedesktop.DBus.Properties", "PropertiesChanged")):
                self.bus.signal_subscribe(BLUEZ, iface, member, None, None, Gio.DBusSignalFlags.NONE,
                                          lambda *_: self.queue_refresh())
            self.register_agent()
        self.refresh()

    # -- BlueZ
    def objects(self):
        try:
            return dbus_call(self.bus, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects",
                             None, "(a{oa{sa{sv}}})")[0]
        except (GLib.Error, TypeError):
            return {}

    def call(self, path, iface, method, args=None, done=None, timeout=90000):
        def finish(bus, res):
            try:
                bus.call_finish(res)
                err = None
            except GLib.Error as e:
                err = e
            if done:
                done(err)
        self.bus.call(BLUEZ, path, iface, method, args, None, Gio.DBusCallFlags.NONE, timeout, None, finish)

    def set_prop(self, path, iface, prop, value, done=None):
        self.call(path, "org.freedesktop.DBus.Properties", "Set", GLib.Variant("(ssv)", (iface, prop, value)), done, 10000)

    def register_agent(self):
        try:
            node = Gio.DBusNodeInfo.new_for_xml(AGENT_XML)
            self.bus.register_object(AGENT_PATH, node.interfaces[0], self.on_agent_call, None, None)
            dbus_call(self.bus, BLUEZ, "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent",
                      GLib.Variant("(os)", (AGENT_PATH, "KeyboardDisplay")))
            dbus_call(self.bus, BLUEZ, "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent",
                      GLib.Variant("(o)", (AGENT_PATH,)))
        except GLib.Error:
            pass  # no bluetoothd: the page says so

    def device_name(self, path):
        d = self.objects().get(path, {}).get("org.bluez.Device1", {})
        return GLib.markup_escape_text(d.get("Alias") or d.get("Name") or "the device")

    def on_agent_call(self, _conn, _sender, _path, _iface, method, params, inv):
        args = params.unpack()
        reject = lambda: inv.return_dbus_error("org.bluez.Error.Rejected", "Rejected by the person")  # noqa: E731
        if method in ("Release", "Cancel"):
            if self.dialog:
                self.dialog.close()
            inv.return_value(None)
        elif method == "AuthorizeService":
            inv.return_value(None)  # services of a device the person paired
        elif method == "RequestConfirmation":
            dev, passkey = args
            self.agent_dialog(f"Pair with {self.device_name(dev)}?",
                              f"Check that {self.device_name(dev)} shows this code:\n\n<span size='xx-large' weight='bold'>{passkey:06d}</span>",
                              "Pair", lambda ok: inv.return_value(None) if ok else reject())
        elif method == "RequestAuthorization":
            dev, = args
            self.agent_dialog(f"Pair with {self.device_name(dev)}?", "The device asks to pair with this computer.",
                              "Pair", lambda ok: inv.return_value(None) if ok else reject())
        elif method in ("RequestPinCode", "RequestPasskey"):
            dev, = args
            entry = Adw.EntryRow(title="PIN" if method == "RequestPinCode" else "Passkey (digits)", activates_default=True)

            def answer(ok):
                text = entry.get_text().strip()
                if not ok or not text:
                    reject()
                elif method == "RequestPinCode":
                    inv.return_value(GLib.Variant("(s)", (text,)))
                elif text.isdigit():
                    inv.return_value(GLib.Variant("(u)", (int(text),)))
                else:
                    reject()
            self.agent_dialog(f"Pair with {self.device_name(dev)}", "Enter the code shown on the device, or its PIN (often 0000).",
                              "Pair", answer, extra=entry)
        elif method in ("DisplayPinCode", "DisplayPasskey"):
            dev, code = args[0], args[1]
            code = f"{code:06d}" if method == "DisplayPasskey" else code
            self.agent_dialog(f"Pair with {self.device_name(dev)}",
                              f"Type this code on {self.device_name(dev)}, then press Return:\n\n<span size='xx-large' weight='bold'>{code}</span>",
                              None, None)
            inv.return_value(None)
        else:
            inv.return_dbus_error("org.bluez.Error.Rejected", f"unknown method {method}")

    def agent_dialog(self, heading, body, accept, answer, extra=None):
        if self.dialog:
            self.dialog.close()
        d = Adw.AlertDialog(heading=heading, body=body, body_use_markup=True)
        if extra:
            d.set_extra_child(boxed_list(extra))
        d.add_response("cancel", "Cancel" if accept else "Close")
        if accept:
            d.add_response("ok", accept)
            d.set_response_appearance("ok", Adw.ResponseAppearance.SUGGESTED)
            d.set_default_response("ok")
        d.set_close_response("cancel")
        answered = {"done": False}

        def on_response(_d, resp):
            self.dialog = None
            if answer and not answered["done"]:
                answered["done"] = True
                answer(resp == "ok")

        d.connect("response", on_response)
        self.dialog = d
        d.present(self.win)

    # -- the page
    def queue_refresh(self):
        if not self.refresh_queued:
            self.refresh_queued = True
            GLib.timeout_add(300, lambda: (self.refresh(), False)[1])

    def refresh(self):
        self.refresh_queued = False
        objs = self.objects() if self.bus else {}
        adapters = [(p, i["org.bluez.Adapter1"]) for p, i in objs.items() if "org.bluez.Adapter1" in i]
        self.adapter = adapters[0][0] if adapters else None
        props = adapters[0][1] if adapters else {}
        powered = bool(props.get("Powered"))
        self.power.handler_block(self.power_handler)
        self.power.set_active(powered)
        self.power.handler_unblock(self.power_handler)
        self.power.set_sensitive(self.adapter is not None)
        if not self.adapter:
            self.status.set_title("No Bluetooth adapter")
            self.status.set_subtitle("This computer has no Bluetooth, or it needs firmware")
        elif not powered:
            self.status.set_title("Bluetooth is off")
            self.status.set_subtitle("")
        else:
            self.status.set_title(f"This computer is “{props.get('Alias') or props.get('Name') or 'SlateOS'}”")
            self.status.set_subtitle("Visible to nearby devices while this page is open" if props.get("Discoverable") else "")
        self.spinner.set_visible(bool(props.get("Discovering")))
        self.mine.set_visible(powered)
        self.nearby.set_visible(powered)

        devices = []
        for path, ifaces in objs.items():
            d = ifaces.get("org.bluez.Device1")
            if d and self.adapter and path.startswith(self.adapter + "/"):
                battery = ifaces.get("org.bluez.Battery1", {}).get("Percentage")
                devices.append((path, d, battery))
        self.clear(self.mine, self.mine_rows)
        self.clear(self.nearby, self.nearby_rows)
        for path, d, battery in sorted(devices, key=lambda t: (not t[1].get("Connected"), (t[1].get("Alias") or "").lower())):
            if d.get("Paired") or d.get("Bonded"):
                self.mine_rows.append(self.paired_row(path, d, battery))
                self.mine.add(self.mine_rows[-1])
        for path, d, _b in sorted(devices, key=lambda t: -(t[1].get("RSSI") or -200)):
            if not (d.get("Paired") or d.get("Bonded")) and d.get("Name"):
                self.nearby_rows.append(self.nearby_row(path, d))
                self.nearby.add(self.nearby_rows[-1])
        if not self.mine_rows:
            self.mine_rows.append(Adw.ActionRow(title="No paired devices"))
            self.mine.add(self.mine_rows[-1])
        if not self.nearby_rows:
            self.nearby_rows.append(Adw.ActionRow(title="Looking for devices…" if props.get("Discovering") else "No devices found"))
            self.nearby.add(self.nearby_rows[-1])

    def icon(self, d):
        return DEVICE_ICONS.get(d.get("Icon", ""), "bluetooth-symbolic")

    def paired_row(self, path, d, battery):
        state = "Connected" if d.get("Connected") else "Not connected"
        if battery is not None:
            state += f" · battery {battery} %"
        row = Adw.ActionRow(title=GLib.markup_escape_text(d.get("Alias") or d.get("Name") or d.get("Address", "")), subtitle=state)
        row.add_prefix(Gtk.Image.new_from_icon_name(self.icon(d)))
        if path in self.pending:
            row.add_suffix(Adw.Spinner())
        else:
            btn = Gtk.Button(label="Disconnect" if d.get("Connected") else "Connect", valign=Gtk.Align.CENTER)
            btn.connect("clicked", lambda *_: self.toggle_connection(path, d))
            row.add_suffix(btn)
        forget = Gtk.Button(icon_name="user-trash-symbolic", valign=Gtk.Align.CENTER, css_classes=["flat"], tooltip_text="Forget this device")
        forget.connect("clicked", lambda *_: self.forget(path, d))
        row.add_suffix(forget)
        return row

    def nearby_row(self, path, d):
        row = Adw.ActionRow(title=GLib.markup_escape_text(d.get("Alias") or d.get("Name")), activatable=path not in self.pending)
        row.add_prefix(Gtk.Image.new_from_icon_name(self.icon(d)))
        if path in self.pending:
            row.set_subtitle("Pairing…")
            row.add_suffix(Adw.Spinner())
        else:
            row.connect("activated", lambda *_: self.pair(path, d))
        return row

    def busy(self, path, on):
        (self.pending.add if on else self.pending.discard)(path)
        self.refresh()

    def toggle_connection(self, path, d):
        name = d.get("Alias") or d.get("Name") or "the device"
        method = "Disconnect" if d.get("Connected") else "Connect"
        self.busy(path, True)

        def done(err):
            self.busy(path, False)
            if err:
                self.toast(f"{name}: {dbus_error(err)}")
        self.call(path, "org.bluez.Device1", method, done=done, timeout=30000)

    def pair(self, path, d):
        name = d.get("Alias") or d.get("Name")
        self.busy(path, True)

        def paired(err):
            if err:
                self.busy(path, False)
                self.toast(f"Could not pair with {name}: {dbus_error(err)}")
                return
            self.set_prop(path, "org.bluez.Device1", "Trusted", GLib.Variant("b", True))
            self.call(path, "org.bluez.Device1", "Connect", done=lambda _e: (self.busy(path, False), self.toast(f"Paired with {name}")), timeout=30000)
        self.call(path, "org.bluez.Device1", "Pair", done=paired)

    def forget(self, path, d):
        self.call(self.adapter, "org.bluez.Adapter1", "RemoveDevice", GLib.Variant("(o)", (path,)),
                  done=lambda err: self.toast(dbus_error(err)) if err else self.toast(f"Forgot {d.get('Alias') or d.get('Name')}"))

    def on_power(self, sw, *_):
        if not self.adapter:
            return
        on = sw.get_active()

        def done(err):
            if err and on and "Blocked" in str(err):
                run(["rfkill", "unblock", "bluetooth"])
                self.set_prop(self.adapter, "org.bluez.Adapter1", "Powered", GLib.Variant("b", True),
                              lambda e: e and self.toast(dbus_error(e)))
            elif err:
                self.toast(dbus_error(err))
            self.queue_refresh()
            if on:
                self.discover(True)
        self.set_prop(self.adapter, "org.bluez.Adapter1", "Powered", GLib.Variant("b", on), done)

    def discover(self, on):
        if not self.adapter:
            return
        self.set_prop(self.adapter, "org.bluez.Adapter1", "Discoverable", GLib.Variant("b", on))
        self.call(self.adapter, "org.bluez.Adapter1", "StartDiscovery" if on else "StopDiscovery", timeout=10000)

    def on_shown(self):
        super().on_shown()
        self.refresh()
        self.discover(True)

    def on_hidden(self):
        super().on_hidden()
        if not self.adapter:
            return
        for iface, method, args in (("org.freedesktop.DBus.Properties", "Set",
                                     GLib.Variant("(ssv)", ("org.bluez.Adapter1", "Discoverable", GLib.Variant("b", False)))),
                                    ("org.bluez.Adapter1", "StopDiscovery", None)):
            try:
                dbus_call(self.bus, BLUEZ, self.adapter, iface, method, args, timeout=3000)
            except GLib.Error:
                pass


# ---- Power ------------------------------------------------------------------------

UPOWER = "org.freedesktop.UPower"
IDLE_CONF = os.path.expanduser("~/.config/slate/idle.json")
DELAYS = [(60, "1 minute"), (120, "2 minutes"), (300, "5 minutes"), (600, "10 minutes"),
          (900, "15 minutes"), (1800, "30 minutes"), (3600, "1 hour"), (0, "Never")]
PROFILES = [("power-saver", "Power Saver", "Longer battery life, a slower computer"),
            ("balanced", "Balanced", "Speed when it is needed, quiet otherwise"),
            ("performance", "Performance", "As fast as the computer goes, louder and warmer")]


def duration(seconds):
    h, m = divmod(int(seconds) // 60, 60)
    return f"{h} h {m} min" if h else f"{m} min"


class PowerPage(Page):
    """The battery, the power mode, and when the screen locks and turns off."""

    def __init__(self, win):
        super().__init__(win)
        try:
            self.bus = system_bus()
        except GLib.Error:
            self.bus = None

        self.battery = Adw.PreferencesGroup(title="Battery")
        self.level = Adw.ActionRow(title="Battery")
        self.bar = Gtk.LevelBar(min_value=0, max_value=100, valign=Gtk.Align.CENTER, hexpand=True)
        self.bar.set_size_request(160, -1)
        self.level.add_suffix(self.bar)
        self.battery.add(self.level)
        self.page.add(self.battery)

        self.mode = Adw.PreferencesGroup(title="Power Mode")
        self.page.add(self.mode)
        self.mode_rows = {}
        first = None
        for key, title, sub in PROFILES:
            row = Adw.ActionRow(title=title, subtitle=sub, activatable=True)
            radio = Gtk.CheckButton(valign=Gtk.Align.CENTER, group=first)
            first = first or radio
            radio.connect("toggled", lambda r, k=key: r.get_active() and self.set_profile(k))
            row.add_prefix(radio)
            row.set_activatable_widget(radio)
            self.mode.add(row)
            self.mode_rows[key] = (row, radio)

        screen = Adw.PreferencesGroup(title="Screen", description="Counted from your last keystroke or mouse movement; what Slate does on its own screen does not count.")
        self.page.add(screen)
        conf = self.idle_conf()
        self.lock_row = Adw.ComboRow(title="Lock the screen after", model=Gtk.StringList.new([t for _, t in DELAYS]))
        self.off_row = Adw.ComboRow(title="Turn the screen off after", model=Gtk.StringList.new([t for _, t in DELAYS]))
        for row, key, default in ((self.lock_row, "lock_after", 600), (self.off_row, "screen_off_after", 900)):
            value = conf.get(key, default)
            row.set_selected(next((i for i, (s, _) in enumerate(DELAYS) if s == value), len(DELAYS) - 1))
            row.connect("notify::selected", lambda *_: self.save_idle())
            screen.add(row)
        lock_note = Adw.ActionRow(title="The screen also locks before the computer sleeps", subtitle="Super+L locks it now")
        screen.add(lock_note)

        self.every(10, self.refresh)

    def idle_conf(self):
        try:
            with open(IDLE_CONF) as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    def save_idle(self):
        conf = {"lock_after": DELAYS[self.lock_row.get_selected()][0],
                "screen_off_after": DELAYS[self.off_row.get_selected()][0]}
        os.makedirs(os.path.dirname(IDLE_CONF), exist_ok=True)
        with open(IDLE_CONF + ".tmp", "w") as f:
            json.dump(conf, f)
        os.replace(IDLE_CONF + ".tmp", IDLE_CONF)
        run(["systemctl", "--user", "restart", "slate-idle.service"])

    def refresh(self):
        props = {}
        if self.bus:
            path = "/org/freedesktop/UPower/devices/DisplayDevice"
            for p in ("IsPresent", "Type", "State", "Percentage", "TimeToEmpty", "TimeToFull"):
                props[p] = dbus_get(self.bus, UPOWER, path, UPOWER + ".Device", p)
        present = bool(props.get("IsPresent")) and props.get("Type") == 2
        self.battery.set_visible(present)
        if present:
            pct = props.get("Percentage") or 0
            state = props.get("State")
            self.bar.set_value(pct)
            # UPower states: 1 charging, 2 discharging, 4 fully charged, 5 pending charge
            if state == 1:
                text = "Charging" + (f", full in {duration(props['TimeToFull'])}" if props.get("TimeToFull") else "")
            elif state == 2:
                text = "On battery" + (f", about {duration(props['TimeToEmpty'])} left" if props.get("TimeToEmpty") else "")
            elif state == 4:
                text = "Fully charged"
            else:
                text = "Plugged in, not charging"
            self.level.set_title(f"{pct:.0f} %")
            self.level.set_subtitle(text)

        profiles = run(["powerprofilesctl", "list"])
        available = {k for k, *_ in PROFILES if f"{k}:" in profiles}
        current = run(["powerprofilesctl", "get"])
        self.mode.set_visible(bool(available))
        for key, (row, radio) in self.mode_rows.items():
            row.set_visible(key in available)
            if key == current and not radio.get_active():
                self.updating = True
                radio.set_active(True)
                self.updating = False

    def set_profile(self, key):
        if getattr(self, "updating", False):
            return
        if run(["powerprofilesctl", "get"]) != key:
            run(["powerprofilesctl", "set", key])


# ---- Keyboard: input methods ------------------------------------------------------

SYSTEM_SETTINGS = "/etc/slate/system-settings.json"
INPUT_METHODS = {
    "pinyin": ("Chinese — Pinyin", "拼音、双拼、五笔、仓颉"),
    "rime": ("Chinese — Rime", "中州韻：繁體、注音、粵拼等"),
    "mozc": ("Japanese — Mozc", "日本語"),
    "hangul": ("Korean — Hangul", "한국어"),
}
FCITX_PROFILE = os.path.expanduser("~/.config/fcitx5/profile")


def system_settings():
    try:
        with open(SYSTEM_SETTINGS) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def sync_fcitx_profile(enabled):
    """fcitx5 keeps its own copy of the input method list once it has run; give it the
    installed ones (order kept, new ones last, removed ones dropped)."""
    if not os.path.exists(FCITX_PROFILE):
        return
    import configparser
    cp = configparser.RawConfigParser(strict=False, interpolation=None)
    cp.optionxform = str
    try:
        cp.read(FCITX_PROFILE, encoding="utf-8")
    except configparser.Error:
        os.remove(FCITX_PROFILE)
        return
    items = []
    i = 0
    while cp.has_section(f"Groups/0/Items/{i}"):
        items.append(cp.get(f"Groups/0/Items/{i}", "Name", fallback=""))
        cp.remove_section(f"Groups/0/Items/{i}")
        i += 1
    keyboards = [n for n in items if n.startswith("keyboard-")] or ["keyboard-us"]
    kept = [n for n in items if n in enabled]
    names = keyboards + kept + [n for n in enabled if n not in kept]
    for i, n in enumerate(names):
        cp.add_section(f"Groups/0/Items/{i}")
        cp.set(f"Groups/0/Items/{i}", "Name", n)
        cp.set(f"Groups/0/Items/{i}", "Layout", "")
    if cp.has_section("Groups/0") and cp.get("Groups/0", "DefaultIM", fallback="") not in names:
        cp.set("Groups/0", "DefaultIM", (kept + enabled + keyboards)[0])
    with open(FCITX_PROFILE, "w", encoding="utf-8") as f:
        cp.write(f, space_around_delimiters=False)


def progress_line(line):
    """A short, readable form of a rebuild's output line."""
    line = line.strip()
    m = re.search(r"/nix/store/[a-z0-9]{32}-([^'\s]+)", line)
    pkg = m.group(1).removesuffix(".drv") if m else ""
    if line.startswith("copying path"):
        return f"Downloading {pkg}…"
    if line.startswith("building '") or line.startswith("building "):
        return f"Building {pkg}…" if pkg else "Building…"
    if "activating the configuration" in line:
        return "Applying…"
    if line.startswith("building the system configuration"):
        return "Preparing…"
    return None


class KeyboardPage(Page):
    """Input methods (Chinese, Japanese, Korean), installed for the whole system on demand."""

    def __init__(self, win):
        super().__init__(win)
        self.sys = system_settings()
        ims = self.sys.get("inputMethods", {})
        self.enabled = list(ims.get("enabled", []))

        group = Adw.PreferencesGroup(
            title="Input Methods",
            description="Type Chinese, Japanese or Korean. Adding one installs it for this computer and needs your password. Ctrl+Space switches between it and the keyboard.")
        self.page.add(group)
        self.switches = {}
        offered = ims.get("available", [])
        for name in [n for n in INPUT_METHODS if n in offered] + [n for n in offered if n not in INPUT_METHODS]:
            title, sub = INPUT_METHODS.get(name, (name, ""))
            row = Adw.SwitchRow(title=title, subtitle=sub)
            row.set_active(name in self.enabled)
            row.connect("notify::active", lambda r, *_a, n=name: self.on_toggle(n, r))
            group.add(row)
            self.switches[name] = row
        if not self.switches:
            group.add(Adw.ActionRow(title="This system does not offer input methods", subtitle="It runs without the SlateOS module's desktop, or a version before 0.2"))

        self.progress = Adw.ActionRow(title="", visible=False)
        self.progress_spinner = Adw.Spinner()
        self.progress.add_prefix(self.progress_spinner)
        group.add(self.progress)

        more = Adw.PreferencesGroup()
        self.config_row = Adw.ActionRow(title="Input Method Settings", subtitle="Shortcuts, candidate window, and each method's options", activatable=True)
        self.config_row.add_suffix(Gtk.Image.new_from_icon_name("go-next-symbolic"))
        self.config_row.connect("activated", lambda *_: subprocess.Popen(["fcitx5-configtool"]))
        more.add(self.config_row)
        self.page.add(more)
        self.config_row.set_visible(bool(self.enabled))
        self.reverting = False

    def on_toggle(self, name, row):
        if self.reverting:
            return
        want = [n for n in self.enabled if n != name] + ([name] if row.get_active() else [])
        title = INPUT_METHODS.get(name, (name,))[0]
        verb = "Add" if row.get_active() else "Remove"
        d = Adw.AlertDialog(
            heading=f"{verb} {title}?",
            body=("SlateOS downloads it and updates the system, which takes a minute or two. "
                  "Your password is needed. Apps that are already open may need to be reopened to use it.")
            if row.get_active() else "SlateOS updates the system without it. Your password is needed.")
        d.add_response("cancel", "Cancel")
        d.add_response("ok", verb)
        d.set_response_appearance("ok", Adw.ResponseAppearance.SUGGESTED)
        d.set_close_response("cancel")
        d.connect("response", lambda _d, resp: self.apply(want) if resp == "ok" else self.revert(name))
        d.present(self.win)

    def revert(self, name):
        self.reverting = True
        self.switches[name].set_active(name in self.enabled)
        self.reverting = False

    def apply(self, want):
        added = [n for n in want if n not in self.enabled]
        names = ", ".join(INPUT_METHODS.get(n, (n,))[0] for n in added)
        env = dict(os.environ, SLATE_ASKPASS_TITLE="Settings needs your password",
                   SLATE_ASKPASS_BODY=(f"To install {names} for this computer." if added
                                       else "To remove an input method from this computer."))
        for sw in self.switches.values():
            sw.set_sensitive(False)
        self.progress.set_visible(True)
        self.progress.set_title("Updating the system…")
        self.progress.set_subtitle("")
        cmd = ["sudo", "-A", "/run/current-system/sw/bin/slateos-settings", "set", "input-methods", ",".join(want) or "none"]

        def work():
            p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
            tail = []
            for line in p.stdout:
                tail = (tail + [line.rstrip()])[-8:]
                short = progress_line(line)
                if short:
                    GLib.idle_add(self.progress.set_subtitle, short)
            return p.wait(), tail

        def done(res):
            self.progress.set_visible(False)
            for sw in self.switches.values():
                sw.set_sensitive(True)
            if isinstance(res, Exception):
                code, tail = 1, [str(res)]
            else:
                code, tail = res
            if code == 0:
                self.enabled = want
                self.after_change()
                self.toast("Done. Ctrl+Space switches input methods." if want else "Input methods removed.")
            else:
                cancelled = bool(tail) and all(t.startswith("sudo:") for t in tail if t.strip())
                self.reverting = True
                for n, sw in self.switches.items():
                    sw.set_active(n in self.enabled)
                self.reverting = False
                if cancelled:
                    self.toast("Nothing changed: the password was not given.")
                else:
                    err = Adw.AlertDialog(heading="The system was not changed", body="\n".join(tail[-5:]))
                    err.add_response("ok", "OK")
                    err.present(self.win)
            self.config_row.set_visible(bool(self.enabled))

        in_thread(work, done)

    def after_change(self):
        run(["systemctl", "--user", "stop", "slate-input-method.service"])
        if self.enabled:
            sync_fcitx_profile(self.enabled)
            run(["systemctl", "--user", "daemon-reload"])
            run(["systemctl", "--user", "restart", "slate-input-method.service"])


# ---- Printers ---------------------------------------------------------------------

class PrintersPage(Page):
    """The printers CUPS knows, the default one, and system-config-printer to add more."""

    def __init__(self, win):
        super().__init__(win)
        self.group = Adw.PreferencesGroup(title="Printers", description="Printers on the network that support driverless printing (most made in the last ten years) appear in every app's Print dialog without being added.")
        self.page.add(self.group)
        self.rows = []
        actions = Adw.PreferencesGroup()
        add = Adw.ActionRow(title="Add a Printer", subtitle="A USB printer, one at a network address, or one that needs a driver", activatable=True)
        add.add_suffix(Gtk.Image.new_from_icon_name("go-next-symbolic"))
        add.connect("activated", lambda *_: subprocess.Popen(["system-config-printer"]))
        actions.add(add)
        self.page.add(actions)
        self.every(10, self.refresh)

    def refresh(self):
        self.clear(self.group, self.rows)
        default = run(["lpstat", "-d"])
        default = default.split(": ", 1)[1].strip() if ": " in default else ""
        states = {}
        for line in run(["lpstat", "-p"]).splitlines():
            m = re.match(r"printer (\S+) (?:is (\w+)|now printing|disabled)", line)
            if m:
                states[m.group(1)] = "Idle" if m.group(2) == "idle" else ("Stopped" if "disabled" in line else "Printing")
        names = [n for n in run(["lpstat", "-e"]).splitlines() if n.strip()]
        for name in names:
            row = Adw.ActionRow(title=GLib.markup_escape_text(name),
                                subtitle=states.get(name, "On the network, ready to use"))
            row.add_prefix(Gtk.Image.new_from_icon_name("printer-symbolic"))
            if name == default:
                row.add_suffix(Gtk.Label(label="Default", css_classes=["dim-label"]))
            else:
                btn = Gtk.Button(label="Make Default", valign=Gtk.Align.CENTER, css_classes=["flat"])
                btn.connect("clicked", lambda *_, n=name: (run(["lpoptions", "-d", n]), self.refresh()))
                row.add_suffix(btn)
            self.group.add(row)
            self.rows.append(row)
        if not names:
            row = Adw.ActionRow(title="No printers found")
            self.group.add(row)
            self.rows.append(row)


class Window(Adw.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Settings", default_width=820, default_height=560)
        split = Adw.NavigationSplitView()
        self.set_content(split)

        sidebar_page = Adw.NavigationPage(title="Settings")
        listbox = Gtk.ListBox(css_classes=["navigation-sidebar"])
        toolbar = Adw.ToolbarView()
        toolbar.add_top_bar(Adw.HeaderBar())
        toolbar.set_content(listbox)
        sidebar_page.set_child(toolbar)
        split.set_sidebar(sidebar_page)

        self.stack = Gtk.Stack()
        content_toolbar = Adw.ToolbarView()
        content_toolbar.add_top_bar(Adw.HeaderBar())
        content_toolbar.set_content(self.stack)
        content_page = Adw.NavigationPage(title="Display", child=content_toolbar)
        split.set_content(content_page)

        pages = [
            ("Network", "network-wireless-symbolic", lambda: NetworkPage(self)),
            ("Bluetooth", "bluetooth-symbolic", lambda: BluetoothPage(self)),
            ("Display", "video-display-symbolic", lambda: DisplayPage(self)),
            ("Appearance", "preferences-desktop-appearance-symbolic", lambda: AppearancePage(self)),
            ("Sound", "audio-volume-high-symbolic", SoundPage),
            ("Keyboard", "input-keyboard-symbolic", lambda: KeyboardPage(self)),
            ("Power", "battery-good-symbolic", lambda: PowerPage(self)),
            ("Printers", "printer-symbolic", lambda: PrintersPage(self)),
            ("Slate", "starred-symbolic", SlatePage),
            ("AI", "system-run-symbolic", AIPage),
        ]
        for name, icon, ctor in pages:
            row = Gtk.ListBoxRow()
            box = Gtk.Box(spacing=10, margin_top=8, margin_bottom=8, margin_start=6)
            box.append(Gtk.Image.new_from_icon_name(icon))
            box.append(Gtk.Label(label=name, xalign=0))
            row.set_child(box)
            row.page_name = name
            listbox.append(row)
            self.stack.add_named(ctor(), name)

        def on_select(_lb, row):
            if row is not None:
                self.stack.set_visible_child_name(row.page_name)
                content_page.set_title(row.page_name)

        listbox.connect("row-selected", on_select)
        listbox.select_row(listbox.get_row_at_index(0))
        self.listbox = listbox

    def show_page(self, name):
        i = 0
        while (row := self.listbox.get_row_at_index(i)) is not None:
            if row.page_name.lower() == name.lower():
                self.listbox.select_row(row)
                return True
            i += 1
        return False


class App(Adw.Application):
    def __init__(self):
        # One instance: `slate-settings power` from the menu bar turns the open window to
        # the Power page.
        super().__init__(application_id="dev.benchgrid.slate.Settings", flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)

    def do_command_line(self, command_line):
        args = command_line.get_arguments()[1:]
        win = self.props.active_window or Window(self)
        if args and not win.show_page(args[0]):
            command_line.printerr(f"slate-settings: no page {args[0]}\n")
        win.present()
        return 0


if __name__ == "__main__":
    sys.exit(App().run(sys.argv))
