#!/usr/bin/env python3
"""slate-welcome: the first minutes on SlateOS. It connects the agent to a model when
that has not happened yet, then shows the short tutorial.

    slate-welcome              the welcome: the connection if it is missing, then the tour
    slate-welcome --first-run  the same, but only until it has been seen once (the
                               session starts it at login)
    slate-welcome --tour       the tour alone
    slate-welcome --agent      the connection alone (Settings, AI opens it)

The agent's connection is required: the first-run welcome comes back at every login
until the agent can reach a model, because without one SlateOS is a desktop that
does not answer.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gi  # noqa: E402

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

import slate_agent as A  # noqa: E402

APP_ID = "dev.benchgrid.slate.Welcome"


def restart_prompt():
    """The Slate prompt reads the connection when it starts: start it again."""
    subprocess.run(["pkill", "-f", "bin/.slate-shell-wrapped"], check=False)
    subprocess.Popen(["slate-shell", "--hidden"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class Welcome(Adw.ApplicationWindow):
    def __init__(self, app, only=None):
        super().__init__(application=app, title="Welcome to SlateOS", default_width=720, default_height=600)
        self.only = only
        self.title = Adw.WindowTitle(title="Welcome to SlateOS")
        header = Adw.HeaderBar(title_widget=self.title)
        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.SLIDE_LEFT_RIGHT, vexpand=True)
        self.back = Gtk.Button(label="Back")
        self.back.connect("clicked", lambda *_: self.go(-1))
        self.next = Gtk.Button(label="Continue", css_classes=["suggested-action"])
        self.next.connect("clicked", lambda *_: self.go(1))
        bar = Gtk.Box(spacing=8, margin_top=10, margin_bottom=12, margin_start=16, margin_end=16)
        bar.append(self.back)
        bar.append(Gtk.Box(hexpand=True))
        bar.append(self.next)
        view = Adw.ToolbarView()
        view.add_top_bar(header)
        view.set_content(self.stack)
        view.add_bottom_bar(bar)
        self.set_content(view)

        hello = Adw.StatusPage(
            icon_name="slateos", title="Welcome to SlateOS",
            description="A desktop you share with an AI agent. Two things before you start: connect the agent, and a minute on how to work with it.")
        self.pages = []      # filled below; the form reports changes while it is built
        self.agent = A.AgentSetup(on_change=self.update, window=self)
        self.tour = A.Tour()
        self.tour.carousel.connect("page-changed", lambda *_: self.update())
        pages = {"hello": hello, "agent": self.agent, "tour": self.tour}
        if only == "agent":
            self.pages = ["agent"]
        elif only == "tour":
            self.pages = ["tour"]
        else:
            # Someone who is connected already (the installer did it) goes straight on.
            self.pages = ["hello"] + ([] if A.is_connected() else ["agent"]) + ["tour"]
        for name in self.pages:
            self.stack.add_named(pages[name], name)
        self.show_page(self.pages[0])

    def page(self):
        return self.stack.get_visible_child_name()

    def show_page(self, name):
        self.stack.set_visible_child_name(name)
        self.title.set_title({"hello": "Welcome to SlateOS", "agent": "Connect Your Agent", "tour": "How to Work with It"}[name])
        self.update()

    def update(self):
        name = self.page()
        if name not in self.pages:
            return
        first = self.pages.index(name) == 0
        self.back.set_visible(not first or (name == "tour" and self.tour.position() > 0))
        if name == "hello":
            self.next.set_label("Get Started")
            self.next.set_sensitive(True)
        elif name == "agent":
            self.next.set_label("Save" if self.only == "agent" else "Continue")
            self.next.set_sensitive(self.agent.complete)
        else:
            self.next.set_label("Start Using SlateOS" if self.tour.at_end() else "Next")
            self.next.set_sensitive(True)

    def go(self, delta):
        name = self.page()
        if name == "tour" and ((delta > 0 and not self.tour.at_end()) or (delta < 0 and self.tour.position() > 0)):
            self.tour.turn(delta)
            GLib.timeout_add(350, lambda: (self.update(), False)[1])
            return
        if delta > 0 and name == "agent":
            self.agent.save()
            restart_prompt()
        i = self.pages.index(name) + delta
        if i >= len(self.pages):
            self.finish()
        elif i >= 0:
            self.show_page(self.pages[i])

    def finish(self):
        if self.only is None:
            os.makedirs(os.path.dirname(A.WELCOME_DONE), exist_ok=True)
            open(A.WELCOME_DONE, "w").close()
        self.close()


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)

    def do_command_line(self, command_line):
        args = command_line.get_arguments()[1:]
        win = self.props.active_window
        if win is None:
            only = "agent" if "--agent" in args else "tour" if "--tour" in args else None
            if "--first-run" in args and os.path.exists(A.WELCOME_DONE):
                if A.is_connected():
                    return 0
                only = "agent"      # seen before, but the agent has lost its connection
            A.install_css()
            win = Welcome(self, only)
        win.present()
        return 0


if __name__ == "__main__":
    GLib.set_prgname(APP_ID)
    sys.exit(App().run(sys.argv))
