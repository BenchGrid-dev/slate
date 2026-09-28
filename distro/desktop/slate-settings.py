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


def run(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=10).stdout.strip()
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
