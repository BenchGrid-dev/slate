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
import time

import gi

gi.require_version("Gtk4LayerShell", "1.0")
from gi.repository import Gtk4LayerShell as LayerShell  # noqa: E402  (must load before Gtk)

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk, Pango  # noqa: E402

WIDTH = 460
PASSIVE_HIDE_SECONDS = 18

CSS_LIGHT = b"""
window.slate { background: transparent; }
.pill { background: rgba(247, 248, 251, 0.97); border: 1px solid rgba(0, 0, 0, 0.14); border-radius: 24px;
        padding: 6px 14px 6px 12px; }
.glyph { color: #3b6de8; font-size: 17px; margin-right: 8px; }
@keyframes slate-pulse { 0% { opacity: 1; } 50% { opacity: 0.35; } 100% { opacity: 1; } }
.glyph.working { animation: slate-pulse 1.2s ease-in-out infinite; }
.glyph.controlling { color: #b26a00; animation: slate-pulse 0.7s ease-in-out infinite; }
entry.ask, entry.ask text { background: none; border: none; box-shadow: none; outline: none; color: #1c1f27; font-size: 15px;
                            caret-color: #3b6de8; min-height: 0; padding: 4px 0; }
entry.ask placeholder, entry.ask text placeholder { color: #8a90a0; }
.hint { color: #8a90a0; font-size: 12px; margin-left: 8px; }
.hint.controlling { color: #b26a00; font-weight: 600; }
.card { background: rgba(247, 248, 251, 0.97); border: 1px solid rgba(0, 0, 0, 0.14); border-radius: 18px;
        padding: 12px 16px; }
.query { color: #6b7080; font-size: 13px; }
.reply { color: #1c1f27; font-size: 14px; }
.reply.error { color: #c0392b; }
.activity { color: #8a90a0; font-size: 12px; }
.activity.failed { color: #c0392b; }
.thought { color: #7a8091; font-size: 12px; font-style: italic; }
.approval { background: rgba(178, 106, 0, 0.08); border: 1px solid rgba(178, 106, 0, 0.45); border-radius: 12px; padding: 10px 12px; }
.approval-title { color: #b26a00; font-weight: 600; font-size: 13px; }
.approval-detail { font-family: monospace; color: #1c1f27; font-size: 12px; }
.approval button { border-radius: 8px; padding: 2px 12px; min-height: 26px; }
.queue { background: rgba(247, 248, 251, 0.97); border: 1px solid rgba(0, 0, 0, 0.14); border-radius: 14px; padding: 8px 12px; }
.queue-title { color: #8a90a0; font-size: 11px; font-weight: 600; }
.queue-text { color: #1c1f27; font-size: 13px; }
.queue-btn { min-height: 22px; min-width: 22px; padding: 0 4px; color: #6b7080; }
"""

CSS = b"""
window.slate { background: transparent; }
.pill { background: rgba(24, 26, 34, 0.97); border: 1px solid rgba(255, 255, 255, 0.16); border-radius: 24px;
        padding: 6px 14px 6px 12px; }
.glyph { color: #8fa7f0; font-size: 17px; margin-right: 8px; }
@keyframes slate-pulse { 0% { opacity: 1; } 50% { opacity: 0.35; } 100% { opacity: 1; } }
.glyph.working { animation: slate-pulse 1.2s ease-in-out infinite; }
.glyph.controlling { color: #e5a35a; animation: slate-pulse 0.7s ease-in-out infinite; }
entry.ask, entry.ask text { background: none; border: none; box-shadow: none; outline: none; color: #e8eaf0; font-size: 15px;
                            caret-color: #8fa7f0; min-height: 0; padding: 4px 0; }
entry.ask placeholder, entry.ask text placeholder { color: #6b7080; }
.hint { color: #6b7080; font-size: 12px; margin-left: 8px; }
.hint.controlling { color: #e5a35a; font-weight: 600; }
.card { background: rgba(24, 26, 34, 0.97); border: 1px solid rgba(255, 255, 255, 0.16); border-radius: 18px;
        padding: 12px 16px; }
.query { color: #9ba1b0; font-size: 13px; }
.reply { color: #e8eaf0; font-size: 14px; }
.reply.error { color: #ff8a80; }
.activity { color: #6b7080; font-size: 12px; }
.thought { color: #7a8091; font-size: 12px; font-style: italic; }
.activity.failed { color: #ff8a80; }
.approval { background: rgba(229, 163, 90, 0.10); border: 1px solid rgba(229, 163, 90, 0.45); border-radius: 12px; padding: 10px 12px; }
.approval-title { color: #e5a35a; font-weight: 600; font-size: 13px; }
.approval-detail { font-family: monospace; color: #d7dae2; font-size: 12px; }
.approval button { border-radius: 8px; padding: 2px 12px; min-height: 26px; }
.queue { background: rgba(24, 26, 34, 0.97); border: 1px solid rgba(255, 255, 255, 0.16); border-radius: 14px; padding: 8px 12px; }
.queue-title { color: #6b7080; font-size: 11px; font-weight: 600; }
.queue-text { color: #d7dae2; font-size: 13px; }
.queue-btn { min-height: 22px; min-width: 22px; padding: 0 4px; color: #9ba1b0; }
"""


def theme_is_dark(style=None):
    """The desktop's theme: slate-theme's choice first (it is what the person set), the
    toolkit's colour-scheme flag otherwise."""
    try:
        with open(os.path.expanduser("~/.config/slate/theme.json")) as f:
            return json.load(f).get("theme", "dark") != "light"
    except Exception:  # noqa: BLE001
        return style.get_dark() if style is not None else True


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


def tool_label(name):
    """`mcp__desktop__desktop_click` -> `desktop_click`; other MCP tools -> `server: tool`."""
    name = name or ""
    if name.startswith("mcp__"):
        parts = name.split("__", 2)
        if len(parts) == 3:
            return parts[2] if parts[2].startswith(parts[1]) else f"{parts[1]}: {parts[2]}"
    return name


def seat0_focus():
    """The container the person's seat has focused (None if unknown)."""
    try:
        seats = json.loads(subprocess.run(["swaymsg", "-t", "get_seats"], capture_output=True, text=True, timeout=3).stdout)
        for s in seats:
            if s.get("name") == "seat0":
                return s.get("focus")
    except Exception:  # noqa: BLE001
        return None
    return None


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
        self.thought = Gtk.Label(xalign=0, ellipsize=Pango.EllipsizeMode.START, max_width_chars=42, css_classes=["thought"], visible=False)
        self.approval_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        for w in (self.query, self.reply, self.activity, self.thought, self.approval_box):
            self.card.append(w)
        root.append(self.card)

        # Messages typed while Slate works wait here; each can be edited, sent now
        # (steering: the current turn is interrupted) or dropped. They go out one by
        # one when a turn ends.
        self.queue_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, css_classes=["queue"], width_request=WIDTH, visible=False)
        self.queue_box.append(Gtk.Label(label="Queued", xalign=0, css_classes=["queue-title"]))
        root.append(self.queue_box)
        self.queue = []  # [(row widget, text)]

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
        self.steer_text = None
        self.reply_text = ""
        self.thinking_text = ""
        self.activity_text = ""
        self.started = 0.0
        self.tick_timer = None
        self.passive = False  # shown without keyboard focus (result or approval arrived)
        self.hide_timer = None
        self.approvals = {}
        self.auto = False
        self.controlling = False
        self.session = Session(self.on_event)
        # The takeover poll runs off the main loop: the desktop daemon can be busy for
        # seconds (screenshots, launches) and a blocked main loop freezes the overlay,
        # which, while it holds the keyboard, freezes the whole desktop.
        threading.Thread(target=self._poll_control_thread, daemon=True).start()
        # When the person's focus moves to another window while the prompt is shown
        # passively, they have moved on: put the prompt away (approvals stay).
        self.user_focus = None
        threading.Thread(target=self._watch_user_focus, daemon=True).start()

    # ---- showing and hiding
    def set_shape(self, active):
        LayerShell.set_anchor(self, LayerShell.Edge.TOP, True)
        LayerShell.set_anchor(self, LayerShell.Edge.RIGHT, True)
        LayerShell.set_anchor(self, LayerShell.Edge.BOTTOM, active)
        LayerShell.set_anchor(self, LayerShell.Edge.LEFT, active)
        LayerShell.set_keyboard_mode(self, LayerShell.KeyboardMode.EXCLUSIVE if active else LayerShell.KeyboardMode.NONE)

    def show_active(self):
        """Open for typing: takes the keyboard; a click anywhere else dismisses it."""
        self.apply_theme_now()
        self.cancel_hide_timer()
        self.passive = False
        self.set_shape(active=True)
        self.present()
        self.entry.grab_focus()

    def show_passive(self):
        """Show a result or an approval without taking the keyboard from what the user is doing."""
        if self.get_visible() and not self.passive:
            return
        self.go_passive()

    def go_passive(self, keep=False):
        self.passive = True
        self.user_focus = seat0_focus()
        self.set_shape(active=False)
        self.set_visible(True)
        if not keep:
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
        self.apply_theme_now()

    def apply_theme_now(self):
        app = self.get_application()
        if app is not None and hasattr(app, "apply_css"):
            app.apply_css(Adw.StyleManager.get_default())

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
        self.thinking_text = ""
        self.activity_text = ""
        for w in (self.query, self.reply, self.activity, self.thought):
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
        self.activity_text = text
        self.render_activity()
        self.activity.set_visible(bool(text))
        if failed:
            self.activity.add_css_class("failed")
        else:
            self.activity.remove_css_class("failed")
        self.card.set_visible(True)

    def render_activity(self):
        self.activity.set_text(self.activity_text)

    def on_tick(self):
        if not self.busy:
            self.tick_timer = None
            return False
        if not self.controlling:
            self.hint.set_text(f"working · {int(time.monotonic() - self.started)}s")
        return True

    def show_thought(self, delta):
        self.thinking_text = (self.thinking_text + delta)[-600:]
        tail = self.thinking_text.strip().replace("\n", " ")
        self.thought.set_text(tail)
        self.thought.set_visible(bool(tail))
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
            self.started = time.monotonic()
            self.set_state("working", "working")
            self.show_activity("thinking…")
            if self.tick_timer is None:
                self.tick_timer = GLib.timeout_add(1000, self.on_tick)
            # Hand the screen back while Slate works: no keyboard grab, no full-screen
            # surface (the agent's own clicks must reach the apps), progress stays visible.
            if self.get_visible() and not self.passive:
                self.go_passive(keep=True)
        elif kind == "thinking":
            self.show_thought(ev.get("text", ""))
        elif kind == "text_delta":
            self.show_reply(self.reply_text + ev.get("text", ""))
        elif kind == "text":
            self.show_reply(ev.get("text", ""))
        elif kind == "tool_start":
            self.thought.set_visible(False)
            self.show_activity(f"▸ {tool_label(ev.get('name'))}  {ev.get('detail', '')}")
        elif kind == "tool_end":
            if not ev.get("ok", True):
                self.show_activity(f"✗ {tool_label(ev.get('name'))}  {ev.get('detail', '')}", failed=True)
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
            self.thought.set_visible(False)
            self.set_state("idle")
            if ev.get("summary") and ev.get("ok") is False and not self.reply_text:
                self.show_reply(ev["summary"], error=True)
            self.show_activity(ev.get("stats") or "")
            if not self.get_visible():
                self.show_passive()
            elif self.passive:
                self.arm_hide_timer()
            if self.tick_timer is not None:
                GLib.source_remove(self.tick_timer)
                self.tick_timer = None
            # Anything queued (or a steering message) goes out now.
            if self.send_next_queued():
                self.cancel_hide_timer()
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

    # ---- the queue
    def enqueue(self, text):
        row = Gtk.Box(spacing=6, css_classes=["queue-row"])
        label = Gtk.Label(label=text, xalign=0, hexpand=True, ellipsize=Pango.EllipsizeMode.END, max_width_chars=34, css_classes=["queue-text"])
        row.append(label)
        for icon, tip, fn in (("document-edit-symbolic", "Edit", self.queue_edit), ("media-skip-forward-symbolic", "Send now (interrupts the current work)", self.queue_steer), ("window-close-symbolic", "Remove", self.queue_delete)):
            b = Gtk.Button(icon_name=icon, css_classes=["flat", "queue-btn"], tooltip_text=tip)
            b.connect("clicked", lambda _b, r=row, f=fn: f(r))
            row.append(b)
        self.queue.append((row, text))
        self.queue_box.append(row)
        self.queue_box.set_visible(True)
        self.hint.set_text(f"working · {len(self.queue)} queued")

    def queue_remove(self, row):
        self.queue = [(r, t) for (r, t) in self.queue if r is not row]
        self.queue_box.remove(row)
        self.queue_box.set_visible(bool(self.queue))
        return True

    def queue_text(self, row):
        return next((t for (r, t) in self.queue if r is row), "")

    def queue_edit(self, row):
        text = self.queue_text(row)
        self.queue_remove(row)
        self.entry.set_text(text)
        self.entry.set_position(-1)
        self.claim_focus() if self.passive else self.entry.grab_focus()

    def queue_delete(self, row):
        self.queue_remove(row)

    def queue_steer(self, row):
        text = self.queue_text(row)
        self.queue_remove(row)
        if self.busy:
            # Interrupt what Slate is doing; the message goes out as soon as the turn ends.
            self.steer_text = text
            self.session.send({"op": "cancel"})
            self.show_activity("interrupting…")
        else:
            self.send_prompt(text)

    def send_next_queued(self):
        if self.steer_text is not None:
            text, self.steer_text = self.steer_text, None
            self.send_prompt("(You interrupted the previous task to say this; take it into account.) " + text)
            return True
        if self.queue:
            row, text = self.queue[0]
            self.queue_remove(row)
            self.send_prompt(text)
            return True
        return False

    def send_prompt(self, text):
        self.clear_card()
        self.query.set_text(text)
        self.query.set_visible(True)
        self.card.set_visible(True)
        self.session.send({"op": "prompt", "text": text})

    # ---- input
    def on_send(self, *_):
        text = self.entry.get_text().strip()
        if not text:
            return
        self.entry.set_text("")
        if self.busy:
            self.enqueue(text)
            return
        self.send_prompt(text)

    def on_key(self, _ctrl, keyval, _code, _state):
        if keyval == Gdk.KEY_Escape:
            self.hide()
            return True
        return False

    def _poll_control_thread(self):
        while True:
            st = run(["slate-desktop", "status"])
            controlling = '"controlling":true' in st.replace(" ", "")
            if controlling != self.controlling:
                self.controlling = controlling
                GLib.idle_add(self.on_control_changed)
            time.sleep(2)

    def _watch_user_focus(self):
        try:
            p = subprocess.Popen(["swaymsg", "-t", "subscribe", "-m", '["window"]'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        except Exception:  # noqa: BLE001
            return
        for line in p.stdout:
            try:
                ev = json.loads(line)
            except Exception:  # noqa: BLE001
                continue
            if ev.get("change") != "focus":
                continue
            focus = seat0_focus()
            if focus is not None:
                GLib.idle_add(self.on_user_focus, focus)

    def on_user_focus(self, con):
        previous, self.user_focus = self.user_focus, con
        if previous is not None and con != previous and self.get_visible() and self.passive and not self.approvals:
            self.hide()
        return False

    def on_control_changed(self):
        if self.controlling:
            self.set_state("controlling", "Esc takes back control")
        else:
            self.set_state("working" if self.busy else "idle")
        return False


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
        self.provider = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), self.provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        # Follow the desktop's colour scheme (slate-theme dark|light), live.
        style = Adw.StyleManager.get_default()
        style.set_color_scheme(Adw.ColorScheme.DEFAULT)
        self.apply_css(style)
        style.connect("notify::dark", lambda s, _p: self.apply_css(s))

    def apply_css(self, style):
        self.provider.load_from_data(CSS if theme_is_dark(style) else CSS_LIGHT)

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
