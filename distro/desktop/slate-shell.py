#!/usr/bin/env python3
"""Slate Shell: the desktop's floating Slate panel.

A layer-shell window anchored top-right (click the panel's Slate button or press
Mod+s to toggle). It runs `slash --serve` and shows the conversation, streamed
answers, tool activity, approvals as buttons, and the agent's status. Esc hides it.
"""
import json
import os
import subprocess
import sys
import threading

import gi

gi.require_version("Gtk4LayerShell", "1.0")
from gi.repository import Gtk4LayerShell as LayerShell  # noqa: E402  (must load before Gtk)

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

CSS = b"""
window.slate-window, .slate-window { background-color: #1d2733; border: 1px solid #4c8bf5; border-radius: 12px; }
.slate-root { background-color: #1d2733; }
.slate-title { font-weight: bold; font-size: 15px; color: #e6edf3; }
.slate-status { color: #8b98a5; font-size: 12px; }
.slate-status.working { color: #4c8bf5; }
.slate-status.controlling { color: #ffb454; font-weight: bold; }
.msg-user { background: #2b3a4a; color: #e6edf3; border-radius: 10px; padding: 8px 10px; }
.msg-agent { background: #22303f; color: #e6edf3; border-radius: 10px; padding: 8px 10px; }
.msg-tool { color: #8b98a5; font-size: 12px; padding: 0 6px; }
.msg-tool.failed { color: #ff7b72; }
.msg-note { color: #8b98a5; font-size: 12px; font-style: italic; padding: 0 6px; }
.msg-error { color: #ff7b72; }
.approval { background: #3a2f1a; border: 1px solid #ffb454; border-radius: 10px; padding: 10px; }
.approval-title { font-weight: bold; color: #ffb454; }
.approval-detail { font-family: monospace; color: #e6edf3; }
.entry { background: #0f1720; color: #e6edf3; border: 1px solid #2b3a4a; border-radius: 10px; padding: 8px; }
"""


def run(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=5).stdout.strip()
    except Exception:  # noqa: BLE001
        return ""


class Session:
    """A `slash --serve` child; events are delivered on the GTK main loop."""

    def __init__(self, on_event):
        self.on_event = on_event
        self.proc = subprocess.Popen(
            ["slash", "--serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, cwd=os.path.expanduser("~"), env=dict(os.environ, NO_COLOR="1"),
        )
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        for line in self.proc.stdout:
            try:
                ev = json.loads(line)
            except Exception:  # noqa: BLE001
                ev = {"event": "raw", "text": line.rstrip()}
            GLib.idle_add(self.on_event, ev)
        GLib.idle_add(self.on_event, {"event": "exited"})

    def send(self, msg):
        try:
            self.proc.stdin.write(json.dumps(msg) + "\n")
            self.proc.stdin.flush()
        except Exception:  # noqa: BLE001
            pass


class ShellWindow(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Slate")
        self.set_default_size(440, 620)
        self.add_css_class("slate-window")

        LayerShell.init_for_window(self)
        LayerShell.set_layer(self, LayerShell.Layer.TOP)
        LayerShell.set_anchor(self, LayerShell.Edge.TOP, True)
        LayerShell.set_anchor(self, LayerShell.Edge.RIGHT, True)
        LayerShell.set_margin(self, LayerShell.Edge.TOP, 40)
        LayerShell.set_margin(self, LayerShell.Edge.RIGHT, 8)
        LayerShell.set_keyboard_mode(self, LayerShell.KeyboardMode.ON_DEMAND)
        LayerShell.set_namespace(self, "slate-shell")

        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin_top=10, margin_bottom=10, margin_start=12, margin_end=12, css_classes=["slate-root"])
        self.set_child(root)

        header = Gtk.Box(spacing=8)
        title = Gtk.Label(label="◆ Slate", xalign=0, css_classes=["slate-title"], hexpand=True)
        self.status = Gtk.Label(label="starting…", xalign=1, css_classes=["slate-status"])
        settings_btn = Gtk.Button(icon_name="preferences-system-symbolic", css_classes=["flat"], tooltip_text="Settings")
        settings_btn.connect("clicked", lambda *_: subprocess.Popen(["slate-settings"]))
        close_btn = Gtk.Button(icon_name="window-close-symbolic", css_classes=["flat"], tooltip_text="Hide (Esc)")
        close_btn.connect("clicked", lambda *_: self.set_visible(False))
        for w in (title, self.status, settings_btn, close_btn):
            header.append(w)
        root.append(header)

        self.scroller = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER)
        self.messages = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.scroller.set_child(self.messages)
        root.append(self.scroller)

        entry_row = Gtk.Box(spacing=6)
        self.entry = Gtk.Entry(hexpand=True, placeholder_text="Ask Slate… (/undo, /auto on, /memories)", css_classes=["entry"])
        self.entry.connect("activate", self.on_send)
        send_btn = Gtk.Button(icon_name="mail-send-symbolic", css_classes=["suggested-action"])
        send_btn.connect("clicked", self.on_send)
        entry_row.append(self.entry)
        entry_row.append(send_btn)
        root.append(entry_row)

        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", self.on_key)
        self.add_controller(keys)

        self.current_agent = None  # the label being streamed into
        self.approvals = {}
        self.busy = False
        self.session = Session(self.on_event)
        GLib.timeout_add_seconds(2, self.poll_control)

    # ---- UI helpers
    def add(self, widget):
        self.messages.append(widget)
        GLib.idle_add(self._scroll_to_end)

    def _scroll_to_end(self):
        adj = self.scroller.get_vadjustment()
        adj.set_value(adj.get_upper() - adj.get_page_size())
        return False

    def label(self, text, css):
        lbl = Gtk.Label(label=text, xalign=0, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, selectable=True, css_classes=[css])
        return lbl

    def set_status(self, text, css=None):
        self.status.set_text(text)
        for c in ("working", "controlling"):
            self.status.remove_css_class(c)
        if css:
            self.status.add_css_class(css)

    # ---- events from slash
    def on_event(self, ev):
        kind = ev.get("event")
        if kind == "ready":
            self.set_status(f"{ev.get('backend')} · ready" + (" · ⚡ auto" if ev.get("auto_approve") else ""))
            self.add(self.label("Ready. Talk to your computer here; approvals show up as buttons.", "msg-note"))
        elif kind == "turn_start":
            self.busy = True
            self.current_agent = None
            self.set_status("working…", "working")
        elif kind == "text_delta":
            if self.current_agent is None:
                self.current_agent = self.label("", "msg-agent")
                self.add(self.current_agent)
            self.current_agent.set_text(self.current_agent.get_text() + ev.get("text", ""))
            self._scroll_to_end()
        elif kind == "text":
            if self.current_agent is None:
                self.add(self.label(ev.get("text", ""), "msg-agent"))
            else:
                self.current_agent.set_text(ev.get("text", ""))
            self.current_agent = None
        elif kind == "tool_start":
            self.add(self.label(f"▸ {ev.get('name')}: {ev.get('detail', '')[:120]}", "msg-tool"))
        elif kind == "tool_end":
            if not ev.get("ok", True):
                lbl = self.label(f"✗ {ev.get('name')}: {ev.get('detail', '')[:160]}", "msg-tool")
                lbl.add_css_class("failed")
                self.add(lbl)
        elif kind == "approval_needed":
            self.add_approval(ev)
        elif kind == "approval_resolved":
            card = self.approvals.pop(ev.get("id"), None)
            if card:
                self.messages.remove(card)
        elif kind == "note":
            text = ev.get("text", "").strip()
            if text:
                self.add(self.label(text, "msg-note"))
        elif kind == "error":
            self.add(self.label(ev.get("text", ""), "msg-error"))
        elif kind == "done":
            self.busy = False
            self.current_agent = None
            stats = ev.get("stats")
            self.set_status(("done · " + stats) if stats else "ready")
            if ev.get("summary") and ev.get("ok") is False:
                self.add(self.label(ev["summary"], "msg-error"))
        elif kind == "exited":
            self.set_status("slash exited")
            self.add(self.label("slash exited; reopen the panel to start a new session.", "msg-error"))
        return False

    def add_approval(self, ev):
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, css_classes=["approval"])
        card.append(Gtk.Label(label=f"Approval needed · {ev.get('tool')} ({ev.get('tier')})", xalign=0, css_classes=["approval-title"]))
        card.append(Gtk.Label(label=ev.get("summary", ""), xalign=0, wrap=True, css_classes=["approval-detail"]))
        card.append(Gtk.Label(label=ev.get("reason", ""), xalign=0, wrap=True, css_classes=["msg-note"]))
        row = Gtk.Box(spacing=6)
        for text, allow, remember, css in (("Allow once", True, False, "suggested-action"), ("Always this task", True, True, ""), ("Deny", False, False, "destructive-action")):
            b = Gtk.Button(label=text, css_classes=[css] if css else [])
            b.connect("clicked", lambda _b, a=allow, r=remember, i=ev["id"]: self.answer(i, a, r))
            row.append(b)
        card.append(row)
        self.approvals[ev["id"]] = card
        self.add(card)
        self.present()

    def answer(self, approval_id, allow, remember):
        self.session.send({"op": "approve", "id": approval_id, "allow": allow, "remember": remember})

    # ---- input
    def on_send(self, *_):
        text = self.entry.get_text().strip()
        if not text:
            return
        self.entry.set_text("")
        self.add(self.label(text, "msg-user"))
        self.session.send({"op": "prompt", "text": text})

    def on_key(self, _ctrl, keyval, _code, _state):
        if keyval == Gdk.KEY_Escape:
            self.set_visible(False)
            return True
        return False

    def poll_control(self):
        st = run(["slate-desktop", "status"])
        if '"controlling":true' in st.replace(" ", ""):
            self.set_status("controlling your mouse/keyboard · Esc takes it back", "controlling")
        elif self.status.has_css_class("controlling"):
            self.set_status("ready")
        return True

    def toggle(self):
        if self.get_visible():
            self.set_visible(False)
        else:
            self.present()
            self.entry.grab_focus()


class App(Adw.Application):
    def __init__(self):
        # Arguments are handled by hand (--hidden); do not let GApplication reject them.
        super().__init__(application_id="dev.benchgrid.slate.Shell", flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)

    def do_command_line(self, _cmdline):
        self.activate()
        return 0
        self.win = None

    def do_startup(self):
        Adw.Application.do_startup(self)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def do_activate(self):
        # First launch shows the panel (or starts it hidden with --hidden, as the
        # session does at login); every later `slate-shell` invocation toggles it.
        if self.win is None:
            self.win = ShellWindow(self)
            if "--hidden" in sys.argv:
                self.win.set_visible(False)
            else:
                self.win.present()
                self.win.entry.grab_focus()
        else:
            self.win.toggle()


if __name__ == "__main__":
    App().run(None)
