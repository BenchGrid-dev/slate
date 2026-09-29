#!/usr/bin/env python3
"""slate-askpass: the password dialog sudo (and ssh) call when there is no terminal.

Agent tool calls have no tty, so `sudo` cannot ask on the terminal. With
SUDO_ASKPASS pointing here, sudo runs this program instead: it shows a dialog to
the person at the desktop and prints what they type on stdout. The agent only
ever sees whether the command ran. Cancel exits non-zero and sudo fails.

Usage: slate-askpass [prompt]
"""
import os
import sys

# sudo passes our stderr through to the command's caller (an agent's tool call):
# keep GTK's driver chatter out of it.
try:
    os.dup2(os.open(os.devnull, os.O_WRONLY), 2)
except OSError:
    pass

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, GLib, Gtk  # noqa: E402

CSS = b"""
window.askpass { background: #1c1f27; }
.askpass-title { font-weight: 700; font-size: 16px; color: #e8eaf0; }
.askpass-body { color: #9ba1b0; font-size: 13px; }
entry.askpass { border-radius: 10px; padding: 8px 12px; }
"""


def main():
    prompt = " ".join(sys.argv[1:]).strip() or "[sudo] password:"
    # sudo's own prompt is "[sudo] password for alice:"; keep the user name, drop the noise.
    who = ""
    if "password for" in prompt:
        who = prompt.split("password for", 1)[1].strip().rstrip(":").strip()
    result = {"password": None}

    app = Adw.Application(application_id="dev.benchgrid.slate.Askpass")

    def activate(app):
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        win = Adw.ApplicationWindow(application=app, title="SlateOS", default_width=420, resizable=False)
        win.add_css_class("askpass")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10, margin_top=22, margin_bottom=18, margin_start=24, margin_end=24)
        box.append(Gtk.Label(label="Slate needs your password", xalign=0, css_classes=["askpass-title"]))
        body = "A task wants to run a command as administrator" + (f" (user {who})" if who else "") + ". The password goes to sudo only; the agent never sees it."
        box.append(Gtk.Label(label=body, xalign=0, wrap=True, css_classes=["askpass-body"]))
        entry = Gtk.PasswordEntry(show_peek_icon=True, css_classes=["askpass"], placeholder_text="Password")
        box.append(entry)
        row = Gtk.Box(spacing=8, halign=Gtk.Align.END, margin_top=6)
        cancel = Gtk.Button(label="Cancel")
        ok = Gtk.Button(label="Allow", css_classes=["suggested-action"])
        row.append(cancel)
        row.append(ok)
        box.append(row)
        win.set_content(box)

        def accept(*_):
            result["password"] = entry.get_text()
            win.close()

        def reject(*_):
            win.close()

        entry.connect("activate", accept)
        ok.connect("clicked", accept)
        cancel.connect("clicked", reject)
        keys = Gtk.EventControllerKey()
        keys.connect("key-pressed", lambda _c, kv, _k, _s: (reject() or True) if kv == Gdk.KEY_Escape else False)
        win.add_controller(keys)
        win.present()
        entry.grab_focus()
        # Do not hang forever if nobody is at the desktop.
        GLib.timeout_add_seconds(120, lambda: (win.close(), False)[1])

    app.connect("activate", activate)
    app.run(None)
    if result["password"] is None:
        sys.exit(1)
    sys.stdout.write(result["password"] + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
