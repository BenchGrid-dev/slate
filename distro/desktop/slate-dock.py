#!/usr/bin/env python3
"""slate-dock: the SlateOS dock.

Pinned applications and the open ones in one row. A dot under an application means it
has a window on your screen; a small diamond means Slate has one of its windows on its
own (background) screen, and clicking the icon brings that window over instead of
sending your focus to a screen you cannot see.

Left click starts or switches to an application, middle click opens a new window,
right click offers keep in / remove from the dock, a new window, and close. Pins
live in ~/.config/slate/dock.json (defaults: /etc/slate/dock.json); the look follows
slate-theme (~/.config/slate/theme.json) live.
"""
import json
import os
import re
import shlex
import subprocess
import sys
import threading

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Gtk4LayerShell", "1.0")
from gi.repository import Gdk, Gio, GLib, Gtk  # noqa: E402
from gi.repository import Gtk4LayerShell as LayerShell  # noqa: E402

HOME = os.path.expanduser("~")
CONFIG = os.path.join(HOME, ".config/slate/dock.json")
SYSTEM_CONFIG = "/etc/slate/dock.json"
THEME = os.path.join(HOME, ".config/slate/theme.json")
ICONS = os.environ.get("SLATE_DOCK_ICONS", os.path.join(os.path.dirname(os.path.realpath(__file__)), "../share/slate-dock"))
ICON_SIZE = 48
DEFAULT_PINNED = ["slate:launcher", "thunar.desktop", "firefox.desktop", "foot.desktop", "thunderbird.desktop",
                  "writer.desktop", "org.xfce.mousepad.desktop", "slate-settings.desktop", "slate:slate"]
IGNORE_APP_IDS = {"at.yrlf.wl_mirror", "slate-shell", "dev.benchgrid.slate.Shell"}

CSS = """
window.slate-dock { background: transparent; }
.pill { border-radius: 22px; padding: 5px 7px; }
.item { padding: 4px 4px 0 4px; border-radius: 14px; transition: background 140ms ease-out; }
.item image { transition: margin 140ms ease-out; margin-top: 4px; margin-bottom: 0; }
.item:hover image { margin-top: 0; margin-bottom: 4px; }
.dot { min-width: 4px; min-height: 4px; border-radius: 2px; margin: 2px 0 1px 0; }
.dot.off { opacity: 0; }
.badge { font-size: 11px; margin: 0 0 0 0; padding: 0; }
.sep { min-width: 1px; margin: 10px 5px 12px 5px; }
@keyframes breathe { 0% { opacity: 1; } 50% { opacity: 0.45; } 100% { opacity: 1; } }
.item.working image { animation: breathe 1.4s ease-in-out infinite; }
popover contents { border-radius: 12px; }
"""
CSS_DARK = """
.pill { background: rgba(32, 32, 38, 0.42); border: 1px solid rgba(255, 255, 255, 0.13);
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.07); }
.item:hover { background: rgba(255, 255, 255, 0.08); }
.dot { background: rgba(255, 255, 255, 0.78); }
.badge { color: #8fa7f0; }
.sep { background: rgba(255, 255, 255, 0.16); }
"""
CSS_LIGHT = """
.pill { background: rgba(255, 255, 255, 0.46); border: 1px solid rgba(0, 0, 0, 0.08);
        box-shadow: inset 0 1px rgba(255, 255, 255, 0.6); }
.item:hover { background: rgba(0, 0, 0, 0.06); }
.dot { background: rgba(0, 0, 0, 0.62); }
.badge { color: #3b6de8; }
.sep { background: rgba(0, 0, 0, 0.14); }
"""


def theme_is_dark():
    try:
        return json.load(open(THEME)).get("theme", "dark") != "light"
    except Exception:  # noqa: BLE001
        return True


def load_pins():
    for path in (CONFIG, SYSTEM_CONFIG):
        try:
            pins = json.load(open(path)).get("pinned")
            if isinstance(pins, list):
                return pins
        except Exception:  # noqa: BLE001
            continue
    return list(DEFAULT_PINNED)


def save_pins(pins):
    os.makedirs(os.path.dirname(CONFIG), exist_ok=True)
    tmp = CONFIG + ".tmp"
    with open(tmp, "w") as f:
        json.dump({"pinned": pins}, f, indent=2)
    os.replace(tmp, CONFIG)


def swaymsg(*args):
    try:
        return subprocess.run(["swaymsg", *args], capture_output=True, text=True, timeout=5).stdout
    except Exception:  # noqa: BLE001
        return ""


def run_in_session(command):
    """Start through sway, so the program belongs to the session, not to the dock."""
    subprocess.Popen(["swaymsg", "exec", command], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


# ---------------------------------------------------------------- windows and apps
class Window:
    def __init__(self, node, output):
        self.con = node["id"]
        self.app_id = node.get("app_id") or (node.get("window_properties") or {}).get("class") or ""
        self.title = node.get("name") or ""
        self.ident = node.get("foreign_toplevel_identifier") or ""
        self.focused = bool(node.get("focused"))
        self.background = output.startswith("HEADLESS-")


def windows():
    try:
        tree = json.loads(swaymsg("-t", "get_tree") or "{}")
    except ValueError:
        return []
    out = []

    def walk(n, output):
        if n.get("type") == "output":
            output = n.get("name", "")
        if n.get("type") in ("con", "floating_con") and (n.get("app_id") or n.get("window_properties")):
            w = Window(n, output or "")
            if w.app_id not in IGNORE_APP_IDS:
                out.append(w)
        for c in n.get("nodes", []) + n.get("floating_nodes", []):
            walk(c, output)

    walk(tree, "")
    return out


class Apps:
    """Desktop entries by id, and a way from a window's app_id to its entry."""

    def __init__(self):
        self.reload()

    def reload(self):
        self.by_id = {}
        self.by_key = {}
        for info in Gio.AppInfo.get_all():
            if not isinstance(info, Gio.DesktopAppInfo):
                continue
            did = info.get_id() or ""
            self.by_id[did] = info
            base = did[:-8] if did.endswith(".desktop") else did
            keys = {base.lower(), base.lower().split(".")[-1]}
            wm = info.get_startup_wm_class()
            if wm:
                keys.add(wm.lower())
            exe = os.path.basename(info.get_executable() or "").lower()
            if exe:
                keys.add(exe)
            for k in keys:
                if k and (k not in self.by_key or info.should_show()):
                    self.by_key[k] = did

    def entry_for(self, app_id):
        a = app_id.lower()
        return self.by_key.get(a) or self.by_key.get(a.split(".")[-1])


def command_line(info):
    cmd = info.get_commandline() or info.get_executable() or ""
    return re.sub(r"\s%[fFuUdDnNickvm]", "", cmd).strip()


# ---------------------------------------------------------------- the dock
class Item:
    """One icon in the dock: a pinned entry, a running app, or one of Slate's own."""

    def __init__(self, dock, key, name, icon, pinned):
        self.dock, self.key, self.name, self.pinned = dock, key, name, pinned
        self.windows = []
        self.widget = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, css_classes=["item"])
        overlay = Gtk.Overlay()
        image = Gtk.Image(pixel_size=ICON_SIZE)
        if isinstance(icon, Gio.Icon):
            image.set_from_gicon(icon)
        elif icon and os.path.isfile(icon):
            image.set_from_file(icon)
        else:
            image.set_from_icon_name(icon or "application-x-executable")
        overlay.set_child(image)
        self.badge = Gtk.Label(label="◆", css_classes=["badge"], halign=Gtk.Align.END, valign=Gtk.Align.START, visible=False)
        overlay.add_overlay(self.badge)
        self.widget.append(overlay)
        self.dot = Gtk.Box(css_classes=["dot", "off"], halign=Gtk.Align.CENTER)
        self.widget.append(self.dot)
        self.widget.set_tooltip_text(name)
        click = Gtk.GestureClick(button=0)
        click.connect("released", self.on_click)
        self.widget.add_controller(click)

    def set_windows(self, wins):
        self.windows = wins
        here = [w for w in wins if not w.background]
        away = [w for w in wins if w.background]
        if here:
            self.dot.remove_css_class("off")
        else:
            self.dot.add_css_class("off")
        self.badge.set_visible(bool(away))
        tip = self.name
        if away and not here:
            tip += " · on Slate's screen, click to bring it here"
        elif away:
            tip += f" · {len(away)} on Slate's screen"
        self.widget.set_tooltip_text(tip)

    # actions
    def launch(self):
        if self.key == "slate:launcher":
            run_in_session("fuzzel")
        elif self.key == "slate:slate":
            run_in_session("slate-shell")
        elif self.key == "slate:trash":
            run_in_session("thunar trash:///")
        else:
            info = self.dock.apps.by_id.get(self.key)
            if info:
                run_in_session(command_line(info))

    def activate(self):
        here = [w for w in self.windows if not w.background]
        away = [w for w in self.windows if w.background]
        if here:
            focused = [i for i, w in enumerate(here) if w.focused]
            target = here[(focused[0] + 1) % len(here)] if focused else here[0]
            swaymsg(f"[con_id={target.con}] focus")
        elif away:
            subprocess.Popen(["slate-desktop", "show", away[0].ident or away[0].app_id],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            self.launch()

    def on_click(self, gesture, _n, _x, _y):
        button = gesture.get_current_button()
        if button == Gdk.BUTTON_PRIMARY:
            self.activate()
        elif button == Gdk.BUTTON_MIDDLE:
            self.launch()
        elif button == Gdk.BUTTON_SECONDARY:
            self.menu()

    def menu(self):
        pop = Gtk.Popover(has_arrow=False, position=Gtk.PositionType.TOP)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        def add(label, cb):
            b = Gtk.Button(label=label, css_classes=["flat"])
            b.connect("clicked", lambda *_: (pop.popdown(), cb()))
            box.append(b)
        if self.key == "slate:trash":
            add("Open", self.launch)
            add("Empty Trash", lambda: subprocess.Popen(["gio", "trash", "--empty"]))
        else:
            if not self.key.startswith("slate:") or self.key == "slate:slate":
                add("New Window" if self.windows else "Open", self.launch)
            if any(w.background for w in self.windows):
                add("Bring Slate's window here", lambda: subprocess.Popen(
                    ["slate-desktop", "show", next(w for w in self.windows if w.background).ident]))
            if self.key in self.dock.pins:
                add("Remove from Dock", lambda: self.dock.unpin(self.key))
            elif not self.key.startswith("app:"):
                add("Keep in Dock", lambda: self.dock.pin(self.key))
            if self.windows:
                add("Close" if len(self.windows) == 1 else f"Close {len(self.windows)} Windows",
                    lambda: [swaymsg(f"[con_id={w.con}] kill") for w in self.windows])
        pop.set_child(box)
        pop.set_parent(self.widget)
        pop.connect("closed", lambda p: GLib.idle_add(p.unparent))
        pop.popup()


class DockWindow(Gtk.ApplicationWindow):
    def __init__(self, app, monitor):
        super().__init__(application=app, title="Dock")
        self.add_css_class("slate-dock")
        LayerShell.init_for_window(self)
        LayerShell.set_namespace(self, "slate-dock")
        LayerShell.set_layer(self, LayerShell.Layer.TOP)
        LayerShell.set_monitor(self, monitor)
        LayerShell.set_anchor(self, LayerShell.Edge.BOTTOM, True)
        LayerShell.set_margin(self, LayerShell.Edge.BOTTOM, 6)
        LayerShell.set_exclusive_zone(self, ICON_SIZE + 26)
        LayerShell.set_keyboard_mode(self, LayerShell.KeyboardMode.NONE)
        self.pill = Gtk.Box(css_classes=["pill"], spacing=2)
        self.set_child(self.pill)


class Dock(Gtk.Application):
    def __init__(self):
        super().__init__(application_id="dev.benchgrid.slate.Dock", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)
        self.windows_by_monitor = {}
        self.pins = load_pins()
        self.apps = None
        self.css = Gtk.CssProvider()
        self.theme_css = Gtk.CssProvider()
        self.agent_working = False

    def do_activate(self):
        if self.apps is not None:
            return
        self.apps = Apps()
        display = Gdk.Display.get_default()
        self.css.load_from_string(CSS)
        Gtk.StyleContext.add_provider_for_display(display, self.css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        Gtk.StyleContext.add_provider_for_display(display, self.theme_css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1)
        self.apply_theme()
        monitors = display.get_monitors()
        monitors.connect("items-changed", lambda *_: GLib.idle_add(self.sync_monitors))
        self.sync_monitors()
        # Theme and pins: follow the files.
        self.watchers = []
        for path, cb in ((THEME, self.apply_theme), (CONFIG, self.reload_pins)):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            mon = Gio.File.new_for_path(path).monitor_file(Gio.FileMonitorFlags.NONE, None)
            mon.connect("changed", lambda *_a, cb=cb: GLib.timeout_add(150, lambda: (cb(), False)[1]))
            self.watchers.append(mon)
        # Applications installed or removed.
        self.app_monitor = Gio.AppInfoMonitor.get()
        self.app_monitor.connect("changed", lambda *_: (self.apps.reload(), self.refresh()))
        threading.Thread(target=self.follow_sway, daemon=True).start()
        GLib.timeout_add_seconds(2, self.poll_status)
        GLib.timeout_add_seconds(10, lambda: (self.refresh(), True)[1])  # trash fill state

    def apply_theme(self):
        self.theme_css.load_from_string(CSS_DARK if theme_is_dark() else CSS_LIGHT)
        Gtk.Settings.get_default().set_property("gtk-application-prefer-dark-theme", theme_is_dark())

    def reload_pins(self):
        self.pins = load_pins()
        self.refresh()

    def pin(self, key):
        if key not in self.pins:
            self.pins.insert(len(self.pins) - 1 if self.pins and self.pins[-1] == "slate:slate" else len(self.pins), key)
            save_pins(self.pins)
            self.refresh()

    def unpin(self, key):
        if key in self.pins:
            self.pins.remove(key)
            save_pins(self.pins)
            self.refresh()

    def sync_monitors(self):
        display = Gdk.Display.get_default()
        wanted = {}
        for i in range(display.get_monitors().get_n_items()):
            m = display.get_monitors().get_item(i)
            if not (m.get_connector() or "").startswith("HEADLESS-"):
                wanted[m.get_connector() or str(i)] = m
        for name in list(self.windows_by_monitor):
            if name not in wanted:
                self.windows_by_monitor.pop(name).destroy()
        for name, m in wanted.items():
            if name not in self.windows_by_monitor:
                w = DockWindow(self, m)
                self.windows_by_monitor[name] = w
                w.present()
        self.refresh()
        return False

    def follow_sway(self):
        """Refresh on every window event; reconnect if sway restarts."""
        while True:
            try:
                p = subprocess.Popen(["swaymsg", "-m", "-t", "subscribe", '["window","workspace"]'],
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
                for _line in p.stdout:
                    GLib.idle_add(self.refresh)
            except Exception:  # noqa: BLE001
                pass
            threading.Event().wait(2)

    def poll_status(self):
        try:
            out = subprocess.run(["slate", "agent-status"], capture_output=True, text=True, timeout=3).stdout
            working = json.loads(out or "{}").get("class") in ("working", "controlling")
        except Exception:  # noqa: BLE001
            working = False
        if working != self.agent_working:
            self.agent_working = working
            self.refresh()
        return True

    def trash_full(self):
        try:
            info = Gio.File.new_for_uri("trash:///").query_info("trash::item-count", Gio.FileQueryInfoFlags.NONE, None)
            return info.get_attribute_uint32("trash::item-count") > 0
        except Exception:  # noqa: BLE001
            return False

    def make_item(self, key, pinned):
        if key == "slate:launcher":
            return Item(self, key, "Applications", os.path.join(ICONS, "slate-launcher.svg"), pinned)
        if key == "slate:slate":
            return Item(self, key, "Slate", os.path.join(ICONS, "slate.svg"), pinned)
        if key == "slate:trash":
            return Item(self, key, "Trash", "user-trash-full" if self.trash_full() else "user-trash", pinned)
        info = self.apps.by_id.get(key)
        if info is None:
            return None
        return Item(self, key, info.get_display_name() or info.get_name(), info.get_icon(), pinned)

    def refresh(self):
        if self.apps is None:
            return False
        wins = windows()
        by_entry = {}
        for w in wins:
            did = self.apps.entry_for(w.app_id) or f"app:{w.app_id}"
            by_entry.setdefault(did, []).append(w)
        keys = [k for k in self.pins if k.startswith("slate:") or k in self.apps.by_id]
        running = [k for k in by_entry if k not in keys]
        for dock in self.windows_by_monitor.values():
            child = dock.pill.get_first_child()
            while child:
                nxt = child.get_next_sibling()
                dock.pill.remove(child)
                child = nxt
            for k in keys:
                item = self.make_item(k, True)
                if item:
                    item.set_windows(by_entry.get(k, []))
                    if k == "slate:slate" and self.agent_working:
                        item.widget.add_css_class("working")
                    dock.pill.append(item.widget)
            if running:
                dock.pill.append(Gtk.Box(css_classes=["sep"]))
                for k in running:
                    item = self.make_item(k, False)
                    if item is None:
                        app_id = k[4:]
                        item = Item(self, k, by_entry[k][0].title or app_id, app_id, False)
                    item.set_windows(by_entry[k])
                    dock.pill.append(item.widget)
            dock.pill.append(Gtk.Box(css_classes=["sep"]))
            trash = self.make_item("slate:trash", False)
            dock.pill.append(trash.widget)
        return False


def main():
    app = Dock()
    sys.exit(app.run(sys.argv[:1]))


if __name__ == "__main__":
    main()
