#!/usr/bin/env python3
"""Slate Settings: a small settings app for the Slate desktop.

Pages: Display (resolution, scale, HiDPI, with a confirm-or-revert countdown),
Sound, Network, and Slate (agent status, memories). Display changes are applied
live through sway and persisted to ~/.config/slate/sway.d/output.conf, which the
Slate sway profile includes.
"""
import glob
import json
import os
import subprocess
import threading
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
        self.outputs = swaymsg("-t", "get_outputs", as_json=True) or []
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


class NetworkPage(Gtk.Box):
    def __init__(self):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Network", description="Managed by NetworkManager; ask Slate to connect to a network.")
        page.add(group)
        for line in run(["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"]).splitlines():
            parts = line.split(":")
            if len(parts) >= 4 and parts[1] != "loopback":
                group.add(Adw.ActionRow(title=parts[0], subtitle=f"{parts[1]} · {parts[2]} · {parts[3] or 'no connection'}"))
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
            ("Display", "video-display-symbolic", lambda: DisplayPage(self)),
            ("Sound", "audio-volume-high-symbolic", SoundPage),
            ("Network", "network-wireless-symbolic", NetworkPage),
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


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id="dev.benchgrid.slate.Settings", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_activate(self):
        win = self.props.active_window or Window(self)
        win.present()


if __name__ == "__main__":
    App().run(None)
