#!/usr/bin/env python3
"""Slate Shell: the desktop's floating Slate prompt.

A layer-shell overlay with a prompt top-right, in the spirit of Siri on macOS: a
pill you type into, and below it one exchange at a time (your request, the
answer, what Slate is doing, approvals as buttons). It appears on the panel's
Slate button or Mod+s, and goes away when you press Esc or click anywhere else.
While Slate works it keeps going in the background; the result and any approval
show up in the same place without stealing your keyboard (clicks elsewhere pass
through), and a click on them brings the keyboard back.

It drives `slash --serve`; the conversation itself continues across exchanges
(the agent remembers the session), only the display is one exchange at a time.
"""
import json
import os
import re
import subprocess
import sys
import threading

import gi

gi.require_version("Gtk4LayerShell", "1.0")
from gi.repository import Gtk4LayerShell as LayerShell  # noqa: E402  (must load before Gtk)

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

WIDTH = 460
PASSIVE_HIDE_SECONDS = 18

CSS = b"""
window.slate { background: transparent; }
.pill { background: rgba(24, 26, 34, 0.96); border: 1px solid rgba(255, 255, 255, 0.10); border-radius: 24px;
        padding: 6px 14px 6px 12px; box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45); }
.glyph { color: #8fa7f0; font-size: 17px; margin-right: 8px; }
@keyframes slate-pulse { 0% { opacity: 1; } 50% { opacity: 0.35; } 100% { opacity: 1; } }
.glyph.working { animation: slate-pulse 1.2s ease-in-out infinite; }
.glyph.controlling { color: #e5a35a; animation: slate-pulse 0.7s ease-in-out infinite; }
entry.ask, entry.ask text { background: none; border: none; box-shadow: none; outline: none; color: #e8eaf0; font-size: 15px;
                            caret-color: #8fa7f0; min-height: 0; padding: 4px 0; }
entry.ask placeholder, entry.ask text placeholder { color: #6b7080; }
.hint { color: #6b7080; font-size: 12px; margin-left: 8px; }
.hint.controlling { color: #e5a35a; font-weight: 600; }
.card { background: rgba(24, 26, 34, 0.96); border: 1px solid rgba(255, 255, 255, 0.10); border-radius: 18px;
        padding: 12px 16px; box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45); }
.query { color: #9ba1b0; font-size: 13px; }
.reply { color: #e8eaf0; font-size: 14px; }
.reply.error { color: #ff8a80; }
.activity { color: #6b7080; font-size: 12px; }
.activity.failed { color: #ff8a80; }
.approval { background: rgba(229, 163, 90, 0.10); border: 1px solid rgba(229, 163, 90, 0.45); border-radius: 12px; padding: 10px 12px; }
.approval-title { color: #e5a35a; font-weight: 600; font-size: 13px; }
.approval-detail { font-family: monospace; color: #d7dae2; font-size: 12px; }
.approval button { border-radius: 8px; padding: 2px 12px; min-height: 26px; }
"""


def markup(text):
    """Pango markup for the little markdown agents produce: `code`, **bold**, headings."""
    out = []
    for line in GLib.markup_escape_text(text).split("\n"):
        stripped = line.lstrip("#").strip() if line.startswith("#") else line
        if line.startswith("#"):
            out.append(f"<b>{stripped}</b>")
        else:
            out.append(line)
    text = "\n".join(out)
    text = re.sub(r"`([^`\n]+)`", r"<tt>\1</tt>", text)
    text = re.sub(r"\*\*([^*\n]+)\*\*", r"<b>\1</b>", text)
    return text


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
        self.add_css_class("slate")

        # Two shapes. Active (typing): the surface covers the whole output, transparent,
        # with the keyboard held exclusively; it never resizes, so the compositor never
        # re-arranges layers while it is open (sway drops layer keyboard focus on every
        # re-arrange), and a click anywhere outside the panel dismisses it. Passive (a
        # result or approval arrived while the user was elsewhere): the surface is just
        # the panel, top-right, with no keyboard, so everything else keeps working; a
        # click on it switches to the active shape.
        LayerShell.init_for_window(self)
        LayerShell.set_layer(self, LayerShell.Layer.OVERLAY)
        LayerShell.set_namespace(self, "slate-shell")
        self.set_shape(active=True)

        outer = Gtk.Box()
        outer.append(Gtk.Box(hexpand=True))  # pushes the panel to the right edge
        self.panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, valign=Gtk.Align.START, hexpand=False,
                             width_request=WIDTH, margin_top=8, margin_end=10)
        outer.append(self.panel)
        self.set_child(outer)
        root = self.panel

        # The pill: status glyph, the prompt, a small hint.
        pill = Gtk.Box(css_classes=["pill"], width_request=WIDTH)
        self.glyph = Gtk.Label(label="◆", css_classes=["glyph"])
        self.entry = Gtk.Entry(hexpand=True, placeholder_text="Ask Slate", css_classes=["ask"])
        self.entry.connect("activate", self.on_send)
        self.hint = Gtk.Label(label="", ellipsize=Pango.EllipsizeMode.END, max_width_chars=22, css_classes=["hint"])
        for w in (self.glyph, self.entry, self.hint):
            pill.append(w)
        root.append(pill)

        # The card: this exchange only.
        self.card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, css_classes=["card"], width_request=WIDTH, visible=False)
        self.query = Gtk.Label(xalign=0, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, max_width_chars=42, css_classes=["query"], visible=False)
        self.reply = Gtk.Label(xalign=0, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, selectable=True, max_width_chars=42, css_classes=["reply"], visible=False)
        self.activity = Gtk.Label(xalign=0, ellipsize=Pango.EllipsizeMode.END, max_width_chars=42, css_classes=["activity"], visible=False)
        self.approval_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        for w in (self.query, self.reply, self.activity, self.approval_box):
            self.card.append(w)
        root.append(self.card)

        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", self.on_key)
        self.add_controller(keys)
        # Clicks: on the panel (after a passive show) claim the keyboard; anywhere else on
        # the surface dismiss it. Capture runs window -> panel, bubble runs panel -> window,
        # so the panel's capture handler marks the press before the window's bubble handler.
        first = Gtk.GestureClick(propagation_phase=Gtk.PropagationPhase.CAPTURE)
        first.connect("pressed", lambda *_: setattr(self, "press_on_panel", False))
        self.add_controller(first)
        on_panel = Gtk.GestureClick(propagation_phase=Gtk.PropagationPhase.CAPTURE)
        on_panel.connect("pressed", self.on_panel_press)
        self.panel.add_controller(on_panel)
        last = Gtk.GestureClick(propagation_phase=Gtk.PropagationPhase.BUBBLE)
        last.connect("pressed", self.on_surface_press)
        self.add_controller(last)
        self.press_on_panel = False

        self.busy = False
        self.reply_text = ""
        self.passive = False  # shown without keyboard focus (result or approval arrived)
        self.hide_timer = None
        self.approvals = {}
        self.auto = False
        self.session = Session(self.on_event)
        GLib.timeout_add_seconds(2, self.poll_control)

    # ---- showing and hiding
    def set_shape(self, active):
        LayerShell.set_anchor(self, LayerShell.Edge.TOP, True)
        LayerShell.set_anchor(self, LayerShell.Edge.RIGHT, True)
        LayerShell.set_anchor(self, LayerShell.Edge.BOTTOM, active)
        LayerShell.set_anchor(self, LayerShell.Edge.LEFT, active)
        LayerShell.set_keyboard_mode(self, LayerShell.KeyboardMode.EXCLUSIVE if active else LayerShell.KeyboardMode.NONE)

    def show_active(self):
        """Open for typing: takes the keyboard; a click anywhere else dismisses it."""
        self.cancel_hide_timer()
        self.passive = False
        self.set_shape(active=True)
        self.present()
        self.entry.grab_focus()

    def show_passive(self):
        """Show a result or an approval without taking the keyboard from what the user is doing."""
        if self.get_visible() and not self.passive:
            return
        self.passive = True
        self.set_shape(active=False)
        self.set_visible(True)
        self.arm_hide_timer()

    def claim_focus(self):
        if self.passive:
            self.passive = False
            self.cancel_hide_timer()
            self.set_shape(active=True)
            self.entry.grab_focus()

    def hide(self):
        self.cancel_hide_timer()
        self.set_visible(False)
        self.passive = False
        if not self.busy and not self.approvals:
            self.clear_card()

    def toggle(self):
        if self.get_visible() and not self.passive:
            self.hide()
        else:
            self.show_active()

    def arm_hide_timer(self):
        self.cancel_hide_timer()
        if not self.approvals:
            self.hide_timer = GLib.timeout_add_seconds(PASSIVE_HIDE_SECONDS, self.on_hide_timer)

    def cancel_hide_timer(self):
        if self.hide_timer:
            GLib.source_remove(self.hide_timer)
            self.hide_timer = None

    def on_hide_timer(self):
        self.hide_timer = None
        if self.passive and not self.approvals:
            self.hide()
        return False

    def on_panel_press(self, *_):
        self.press_on_panel = True
        self.claim_focus()

    def on_surface_press(self, *_):
        if not self.press_on_panel and not self.passive:
            self.hide()

    # ---- the card
    def clear_card(self):
        self.reply_text = ""
        for w in (self.query, self.reply, self.activity):
            w.set_text("")
            w.set_visible(False)
        self.reply.remove_css_class("error")
        self.card.set_visible(False)

    def show_reply(self, text, error=False):
        self.reply_text = text
        try:
            self.reply.set_markup(markup(text))
        except Exception:  # noqa: BLE001
            self.reply.set_text(text)
        self.reply.set_visible(bool(text))
        if error:
            self.reply.add_css_class("error")
        self.card.set_visible(True)

    def show_activity(self, text, failed=False):
        self.activity.set_text(text)
        self.activity.set_visible(bool(text))
        if failed:
            self.activity.add_css_class("failed")
        else:
            self.activity.remove_css_class("failed")
        self.card.set_visible(True)

    def set_state(self, state, hint=None):
        for c in ("working", "controlling"):
            self.glyph.remove_css_class(c)
            self.hint.remove_css_class(c)
        if state in ("working", "controlling"):
            self.glyph.add_css_class(state)
            self.hint.add_css_class(state)
        if hint is None:
            hint = "auto" if self.auto else ""
        self.hint.set_text(hint)

    # ---- events from slash
    def on_event(self, ev):
        kind = ev.get("event")
        if kind == "ready":
            self.auto = bool(ev.get("auto_approve"))
            self.set_state("idle")
        elif kind == "turn_start":
            self.busy = True
            self.set_state("working", "working")
            self.show_activity("thinking…")
        elif kind == "text_delta":
            self.show_reply(self.reply_text + ev.get("text", ""))
        elif kind == "text":
            self.show_reply(ev.get("text", ""))
        elif kind == "tool_start":
            self.show_activity(f"▸ {ev.get('name')}  {ev.get('detail', '')}")
        elif kind == "tool_end":
            if not ev.get("ok", True):
                self.show_activity(f"✗ {ev.get('name')}  {ev.get('detail', '')}", failed=True)
        elif kind == "approval_needed":
            self.add_approval(ev)
        elif kind == "approval_resolved":
            card = self.approvals.pop(ev.get("id"), None)
            if card:
                self.approval_box.remove(card)
            if self.passive:
                self.arm_hide_timer()
        elif kind == "note":
            text = ev.get("text", "").strip()
            if text:
                if "auto" in text.lower():
                    self.auto = "on" in text.lower() and "off" not in text.lower()
                self.show_activity(text)
        elif kind == "error":
            self.show_reply(ev.get("text", ""), error=True)
        elif kind == "done":
            self.busy = False
            self.set_state("idle")
            if ev.get("summary") and ev.get("ok") is False and not self.reply_text:
                self.show_reply(ev["summary"], error=True)
            self.show_activity(ev.get("stats") or "")
            if not self.get_visible():
                self.show_passive()
            elif self.passive:
                self.arm_hide_timer()
        elif kind == "exited":
            self.busy = False
            self.set_state("idle", "offline")
            self.show_reply("slash exited; press Mod+s to start a new session.", error=True)
        return False

    def add_approval(self, ev):
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, css_classes=["approval"])
        card.append(Gtk.Label(label=f"{ev.get('tool')} · {ev.get('tier')} · needs your OK", xalign=0, css_classes=["approval-title"]))
        card.append(Gtk.Label(label=ev.get("summary", ""), xalign=0, wrap=True, wrap_mode=Pango.WrapMode.WORD_CHAR, max_width_chars=44, css_classes=["approval-detail"]))
        if ev.get("reason"):
            card.append(Gtk.Label(label=ev["reason"], xalign=0, wrap=True, max_width_chars=44, css_classes=["activity"]))
        row = Gtk.Box(spacing=6)
        for text, allow, remember, css in (("Allow", True, False, "suggested-action"), ("Always this task", True, True, ""), ("Deny", False, False, "destructive-action")):
            b = Gtk.Button(label=text, css_classes=[css] if css else [])
            b.connect("clicked", lambda _b, a=allow, r=remember, i=ev["id"]: self.answer(i, a, r))
            row.append(b)
        card.append(row)
        self.approvals[ev["id"]] = card
        self.approval_box.append(card)
        self.card.set_visible(True)
        self.cancel_hide_timer()
        if not self.get_visible():
            self.show_passive()

    def answer(self, approval_id, allow, remember):
        self.session.send({"op": "approve", "id": approval_id, "allow": allow, "remember": remember})

    # ---- input
    def on_send(self, *_):
        text = self.entry.get_text().strip()
        if not text:
            return
        self.entry.set_text("")
        self.clear_card()
        self.query.set_text(text)
        self.query.set_visible(True)
        self.card.set_visible(True)
        self.session.send({"op": "prompt", "text": text})

    def on_key(self, _ctrl, keyval, _code, _state):
        if keyval == Gdk.KEY_Escape:
            self.hide()
            return True
        return False

    def poll_control(self):
        st = run(["slate-desktop", "status"])
        if '"controlling":true' in st.replace(" ", ""):
            self.set_state("controlling", "Esc takes back control")
        elif self.glyph.has_css_class("controlling"):
            self.set_state("working" if self.busy else "idle")
        return True


class App(Adw.Application):
    def __init__(self):
        # Arguments are handled by hand (--hidden); do not let GApplication reject them.
        super().__init__(application_id="dev.benchgrid.slate.Shell", flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.win = None

    def do_command_line(self, _cmdline):
        self.activate()
        return 0

    def do_startup(self):
        Adw.Application.do_startup(self)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def do_activate(self):
        # First launch shows the prompt (or starts hidden with --hidden, as the
        # session does at login); every later `slate-shell` invocation toggles it.
        if self.win is None:
            self.win = ShellWindow(self)
            if "--hidden" in sys.argv:
                self.win.set_visible(False)
            else:
                self.win.show_active()
        else:
            self.win.toggle()


if __name__ == "__main__":
    App().run(None)
