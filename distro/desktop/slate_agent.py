"""How the agent reaches a model, and how a new person learns the desktop: what the
installer, the first-run welcome and Settings share.

`AgentSetup` is the form: a Claude subscription (Claude Code's own sign-in), or an API
of the person's own, in OpenAI's or Anthropic's format (an address, a key, a model).
It writes `~/.config/slate/provider.toml`, which slash reads, and tries a connection
with `slate provider test`, the same code path the agent's requests take.
`Tour` is the short tutorial: a few pages of text.
"""
import json
import os
import shutil
import subprocess
import threading
import tomllib

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, GLib, Gtk  # noqa: E402

PROVIDER = os.path.expanduser("~/.config/slate/provider.toml")
WELCOME_DONE = os.path.expanduser("~/.local/state/slate/welcome-done")

# Services with an address worth remembering. The model is asked of the service
# (`GET …/models`) or typed: names change faster than this file.
PRESETS = [
    {"id": "openai", "name": "OpenAI", "format": "openai", "base_url": "https://api.openai.com/v1"},
    {"id": "anthropic", "name": "Anthropic", "format": "anthropic", "base_url": "https://api.anthropic.com"},
    {"id": "openrouter", "name": "OpenRouter", "format": "openai", "base_url": "https://openrouter.ai/api/v1"},
    {"id": "deepseek", "name": "DeepSeek", "format": "openai", "base_url": "https://api.deepseek.com/v1"},
    {"id": "custom-openai", "name": "Another service, OpenAI format", "format": "openai", "base_url": ""},
    {"id": "custom-anthropic", "name": "Another service, Anthropic format", "format": "anthropic", "base_url": ""},
]
FORMATS = [("openai", "OpenAI (chat completions)"), ("anthropic", "Anthropic (messages)")]

CSS = """
.agent-ok { color: @success_color; }
.agent-bad { color: @error_color; }
.tour-key { font-family: monospace; font-weight: 700; font-size: 13px; padding: 3px 9px; border-radius: 7px;
            background: alpha(@window_fg_color, 0.10); border: 1px solid alpha(@window_fg_color, 0.18); }
.tour-title { font-size: 20px; font-weight: 700; }
.tour-body { font-size: 14px; }
.tour-example { font-style: italic; opacity: 0.8; }
"""


def environment():
    """The session's environment, with the places per-user agent CLIs are installed to."""
    env = dict(os.environ)
    extra = [os.path.expanduser(p) for p in ("~/.npm-global/bin", "~/.local/bin", "~/.cargo/bin")]
    env["PATH"] = ":".join(extra + [env.get("PATH", ""), "/run/current-system/sw/bin"])
    return env


def run(cmd, timeout=15, text_in=None):
    try:
        p = subprocess.run(cmd, input=text_in, capture_output=True, text=True, timeout=timeout, env=environment())
        return p.returncode, p.stdout.strip()
    except Exception as e:  # noqa: BLE001
        return 1, f"({e})"


def in_thread(work, done):
    def go():
        try:
            result = work()
        except Exception as e:  # noqa: BLE001
            result = e
        GLib.idle_add(lambda: (done(result), False)[1])
    threading.Thread(target=go, daemon=True).start()


# ---- the provider file -------------------------------------------------------------

def quote(value):
    return json.dumps(str(value), ensure_ascii=False)


def provider_text(api=None):
    """provider.toml for an API connection (a dict), or for the agent's own sign-in."""
    if api is None:
        return 'connection = "subscription"\n'
    lines = ['connection = "api"', "", "[api]", f"format = {quote(api['format'])}", f"base_url = {quote(api['base_url'])}",
             f"api_key = {quote(api['api_key'])}"]
    for key in ("model", "small_model"):
        if api.get(key):
            lines.append(f"{key} = {quote(api[key])}")
    return "\n".join(lines) + "\n"


def load_provider(path=PROVIDER):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def write_private(path, text):
    """Write a file only its owner can read (it holds a key)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".new"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.chmod(tmp, 0o600)
    os.replace(tmp, path)


def test_connection(api):
    """(ok, one line for the person) from `slate provider test`."""
    code, out = run(["slate", "provider", "test", "-"], timeout=90, text_in=provider_text(api))
    try:
        result = json.loads(out.splitlines()[-1])
        return bool(result.get("ok")), result.get("message", "")
    except (ValueError, IndexError):
        return False, out or "slate provider test gave no answer"


def list_models(api):
    """(models, problem) from `slate provider models`."""
    code, out = run(["slate", "provider", "models", "-"], timeout=45, text_in=provider_text(dict(api, model=api.get("model") or "x")))
    try:
        result = json.loads(out.splitlines()[-1])
    except (ValueError, IndexError):
        return [], out or "no answer"
    return (result.get("models", []), None) if result.get("ok") else ([], result.get("message", ""))


def claude_account():
    """Who Claude Code is signed in as: a name, "" when signed in without one, None when not."""
    code, out = run(["claude", "auth", "status"])
    try:
        status = json.loads(out)
    except ValueError:
        return None
    if not status.get("loggedIn"):
        return None
    return status.get("email") or status.get("account") or status.get("authMethod") or ""


def describe(provider=None):
    """One line on how the agent connects now, for a summary."""
    provider = load_provider() if provider is None else provider
    api = provider.get("api") if provider.get("connection") == "api" else None
    if api:
        host = api.get("base_url", "").split("://")[-1].split("/")[0]
        return f"API at {host}" + (f", {api['model']}" if api.get("model") else "")
    who = claude_account()
    if who is None:
        return "Not connected"
    return "Claude subscription" + (f" ({who})" if who else "")


def is_connected():
    """Whether the agent can reach a model: an API connection, or a signed-in Claude Code."""
    provider = load_provider()
    if provider.get("connection") == "api" and provider.get("api", {}).get("api_key"):
        return True
    return claude_account() is not None


def install_css():
    from gi.repository import Gdk
    css = Gtk.CssProvider()
    css.load_from_string(CSS)
    Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)


# ---- the form ----------------------------------------------------------------------

class AgentSetup(Gtk.Box):
    """The choice between a subscription and an API of one's own, with its fields.

    `on_change()` is called whenever `complete` may have changed. `complete` is true
    when the agent can connect: signed in, or an API connection that answered a test
    (or that the person chose to keep although it did not).
    """

    def __init__(self, on_change=None, window=None):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.on_change = on_change or (lambda: None)
        self.window = window
        self.account = None          # Claude Code's sign-in, as last seen
        self.tested = None           # the API fields that last passed a test (or were kept anyway)
        self.busy = False
        self.poll = None
        saved = load_provider().get("api") or {}

        page = Adw.PreferencesPage()
        self.append(page)
        how = Adw.PreferencesGroup(
            title="Connect your agent",
            description="SlateOS works through an AI agent. Choose how it reaches a model; you can change this later in Settings, AI.")
        page.add(how)
        self.sub_row = Adw.ActionRow(title="Claude subscription", subtitle="Sign in with your Claude account (Pro or Max). Nothing else to set up.", activatable=True)
        self.sub_check = Gtk.CheckButton(valign=Gtk.Align.CENTER)
        self.sub_row.add_prefix(self.sub_check)
        self.sub_row.set_activatable_widget(self.sub_check)
        how.add(self.sub_row)
        self.api_row = Adw.ActionRow(title="An API key of your own", subtitle="OpenAI, Anthropic, or any service that speaks one of their formats: an address, a key, a model.", activatable=True)
        self.api_check = Gtk.CheckButton(valign=Gtk.Align.CENTER, group=self.sub_check)
        self.api_row.add_prefix(self.api_check)
        self.api_row.set_activatable_widget(self.api_check)
        how.add(self.api_row)

        # A subscription: Claude Code signs in by itself, in a terminal and the browser.
        self.sub_group = Adw.PreferencesGroup()
        page.add(self.sub_group)
        self.signin_row = Adw.ActionRow(title="Claude Code", subtitle="Checking…")
        self.signin = Gtk.Button(label="Sign In…", valign=Gtk.Align.CENTER, css_classes=["suggested-action"])
        self.signin.connect("clicked", self.sign_in)
        self.signin_row.add_suffix(self.signin)
        self.sub_group.add(self.signin_row)
        note = Gtk.Label(
            label="Sign-in happens in Claude Code itself: a terminal opens and shows a link for the browser. SlateOS does not see your password.",
            wrap=True, xalign=0, css_classes=["dim-label", "caption"], margin_top=8, margin_start=4, margin_end=4)
        self.sub_group.add(note)

        # An API: where, with which key, which model.
        self.api_group = Adw.PreferencesGroup()
        page.add(self.api_group)
        self.preset = Adw.ComboRow(title="Service", model=Gtk.StringList.new([p["name"] for p in PRESETS]))
        self.api_group.add(self.preset)
        self.format = Adw.ComboRow(title="Format", model=Gtk.StringList.new([name for _id, name in FORMATS]))
        self.api_group.add(self.format)
        self.address = Adw.EntryRow(title="API address")
        self.api_group.add(self.address)
        self.key = Adw.PasswordEntryRow(title="API key")
        self.api_group.add(self.key)
        self.model = Adw.EntryRow(title="Model")
        self.models_button = Gtk.MenuButton(icon_name="view-list-symbolic", valign=Gtk.Align.CENTER, css_classes=["flat"],
                                            tooltip_text="Ask the service which models it has")
        self.models_list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE, css_classes=["boxed-list"])
        self.models_list.connect("row-activated", self.pick_model)
        self.models_status = Gtk.Label(label="", wrap=True, max_width_chars=36, xalign=0, margin_top=6, margin_bottom=6, margin_start=8, margin_end=8)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, margin_top=6, margin_bottom=6, margin_start=6, margin_end=6)
        box.append(self.models_status)
        box.append(Gtk.ScrolledWindow(child=self.models_list, max_content_height=280, propagate_natural_height=True, min_content_width=300))
        self.models_popover = Gtk.Popover(child=box)
        self.models_popover.connect("show", lambda *_: self.load_models())
        self.models_button.set_popover(self.models_popover)
        self.model.add_suffix(self.models_button)
        self.api_group.add(self.model)
        self.test_row = Adw.ActionRow(title="Connection", subtitle="Not tried yet")
        self.spinner = Gtk.Spinner(valign=Gtk.Align.CENTER)
        self.test_row.add_suffix(self.spinner)
        self.keep = Gtk.Button(label="Use It Anyway", valign=Gtk.Align.CENTER, css_classes=["flat"], visible=False,
                               tooltip_text="Keep these settings although the test did not pass (no network yet, for instance)")
        self.keep.connect("clicked", self.keep_untested)
        self.test_row.add_suffix(self.keep)
        self.test = Gtk.Button(label="Test", valign=Gtk.Align.CENTER, css_classes=["suggested-action"])
        self.test.connect("clicked", self.run_test)
        self.test_row.add_suffix(self.test)
        self.api_group.add(self.test_row)
        key_note = Gtk.Label(
            label="The key is kept in a file only you can read, and stays out of reach of the commands the agent runs.",
            wrap=True, xalign=0, css_classes=["dim-label", "caption"], margin_top=8, margin_start=4, margin_end=4)
        self.api_group.add(key_note)

        # What was set up before.
        if saved:
            fmt = saved.get("format", "openai")
            known = next((i for i, p in enumerate(PRESETS) if p["base_url"] and p["base_url"] == saved.get("base_url")), None)
            custom = next(i for i, p in enumerate(PRESETS) if p["id"] == f"custom-{fmt}")
            self.preset.set_selected(known if known is not None else custom)
            self.format.set_selected([f for f, _ in FORMATS].index(fmt) if fmt in [f for f, _ in FORMATS] else 0)
            self.address.set_text(saved.get("base_url", ""))
            self.key.set_text(saved.get("api_key", ""))
            self.model.set_text(saved.get("model", ""))
            self.tested = self.fields()
        else:
            self.apply_preset()
        self.preset.connect("notify::selected", lambda *_: self.apply_preset())
        self.format.connect("notify::selected", lambda *_: self.changed())
        for row in (self.address, self.key, self.model):
            row.connect("changed", lambda *_: self.changed())
        (self.api_check if load_provider().get("connection") == "api" else self.sub_check).set_active(True)
        # Both buttons: the one that is switched off reports before the other is on.
        self.sub_check.connect("toggled", lambda *_: self.changed())
        self.api_check.connect("toggled", lambda *_: self.changed())
        self.connect("map", lambda *_: self.start_polling())
        self.connect("unmap", lambda *_: self.stop_polling())
        self.changed()
        self.refresh_account()

    # State

    def mode(self):
        return "api" if self.api_check.get_active() else "subscription"

    def fields(self):
        return {"format": FORMATS[self.format.get_selected()][0], "base_url": self.address.get_text().strip(),
                "api_key": self.key.get_text().strip(), "model": self.model.get_text().strip()}

    def fields_problem(self):
        f = self.fields()
        if not f["base_url"].startswith(("https://", "http://")):
            return "Enter the service's address, starting with https://"
        if not f["api_key"]:
            return "Enter the key"
        if f["format"] == "openai" and not f["model"]:
            return "Name the model (the list button asks the service)"
        return None

    @property
    def complete(self):
        if self.mode() == "subscription":
            return self.account is not None
        return self.fields_problem() is None and self.tested == self.fields()

    def summary(self):
        if self.mode() == "subscription":
            return "Claude subscription" + (f" ({self.account})" if self.account else "")
        f = self.fields()
        return f"API at {f['base_url'].split('://')[-1].split('/')[0]}" + (f", {f['model']}" if f["model"] else "")

    def changed(self):
        api = self.mode() == "api"
        self.sub_group.set_visible(not api)
        self.api_group.set_visible(api)
        if api:
            problem = self.fields_problem()
            current = self.tested == self.fields()
            self.test.set_sensitive(problem is None and not self.busy)
            self.models_button.set_sensitive(bool(self.fields()["api_key"]) and self.fields()["base_url"].startswith(("https://", "http://")))
            if not self.busy and not current:
                self.test_row.remove_css_class("agent-ok")
                self.test_row.remove_css_class("agent-bad")
                self.test_row.set_subtitle(problem or "Not tried yet")
                self.keep.set_visible(False)
        self.on_change()

    def apply_preset(self):
        preset = PRESETS[self.preset.get_selected()]
        self.format.set_selected([f for f, _ in FORMATS].index(preset["format"]))
        custom = preset["id"].startswith("custom-")
        self.format.set_sensitive(custom)
        if preset["base_url"] or custom:
            self.address.set_text(preset["base_url"])
        self.model.set_title("Model" if preset["format"] == "openai" else "Model (empty: Claude Code's own choice)")
        self.changed()

    # A subscription

    def refresh_account(self):
        def done(who):
            self.account = None if isinstance(who, Exception) else who
            if self.account is None:
                self.signin_row.set_subtitle("Not signed in")
                self.signin.set_label("Sign In…")
            else:
                self.signin_row.set_subtitle("Signed in" + (f" as {self.account}" if self.account else ""))
                self.signin.set_label("Sign In Again…")
                self.signin.remove_css_class("suggested-action")
            self.on_change()
        in_thread(claude_account, done)

    def start_polling(self):
        if self.poll is None:
            self.poll = GLib.timeout_add_seconds(4, lambda: (self.mode() == "subscription" and self.refresh_account(), True)[1])

    def stop_polling(self):
        if self.poll is not None:
            GLib.source_remove(self.poll)
            self.poll = None

    def sign_in(self, *_):
        if not shutil.which("claude", path=environment()["PATH"]):
            self.signin_row.set_subtitle("Claude Code is not installed on this system")
            return
        # The CLI prints a link, opens the browser and waits for the code; the shell
        # afterwards keeps the window readable.
        script = 'claude auth login; echo; echo "You can close this window."; exec ${SHELL:-sh}'
        subprocess.Popen(["foot", "--title", "Sign in to Claude", "-e", "sh", "-c", script], env=environment())

    # An API

    def load_models(self):
        api = self.fields()
        self.models_status.set_label("Asking the service…")
        while (row := self.models_list.get_row_at_index(0)) is not None:
            self.models_list.remove(row)

        def done(result):
            models, problem = ([], str(result)) if isinstance(result, Exception) else result
            if problem:
                self.models_status.set_label(f"The service gave no list ({problem}). Type the model's name instead.")
                return
            self.models_status.set_label(f"{len(models)} models. Pick one:" if models else "The service lists no models. Type the name instead.")
            for name in models:
                self.models_list.append(Gtk.ListBoxRow(child=Gtk.Label(label=name, xalign=0, margin_top=7, margin_bottom=7, margin_start=10, margin_end=10)))
        in_thread(lambda: list_models(api), done)

    def pick_model(self, _list, row):
        self.model.set_text(row.get_child().get_label())
        self.models_popover.popdown()

    def run_test(self, *_):
        api = self.fields()
        self.busy = True
        self.test.set_sensitive(False)
        self.keep.set_visible(False)
        self.spinner.start()
        self.test_row.remove_css_class("agent-ok")
        self.test_row.remove_css_class("agent-bad")
        self.test_row.set_subtitle("Sending a short request…")

        def done(result):
            ok, message = (False, str(result)) if isinstance(result, Exception) else result
            self.busy = False
            self.spinner.stop()
            if api != self.fields():       # edited meanwhile: this answer is about something else
                self.changed()
                return
            self.test_row.set_subtitle(message)
            self.test_row.add_css_class("agent-ok" if ok else "agent-bad")
            self.tested = api if ok else None
            self.keep.set_visible(not ok)
            self.test.set_sensitive(True)
            self.on_change()
        in_thread(lambda: test_connection(api), done)

    def keep_untested(self, *_):
        self.tested = self.fields()
        self.keep.set_visible(False)
        self.test_row.set_subtitle("Kept without a passed test. Settings, AI can try it again.")
        self.on_change()

    # The result

    def files(self):
        """What to put in a home directory for this choice: {relative path: text}.
        A subscription is Claude Code's own files, copied as they are."""
        if self.mode() == "api":
            return {".config/slate/provider.toml": provider_text(self.fields())}
        out = {".config/slate/provider.toml": provider_text(None)}
        for rel in (".claude/.credentials.json", ".claude.json"):
            try:
                with open(os.path.join(os.path.expanduser("~"), rel)) as f:
                    out[rel] = f.read()
            except OSError:
                pass
        return out

    def save(self):
        """Make the choice this person's own (slash reads it at its next start)."""
        write_private(PROVIDER, provider_text(self.fields() if self.mode() == "api" else None))


# ---- the tutorial ------------------------------------------------------------------

# (icon, title, text, keys shown as caps, an example to try)
TOUR = [
    ("slate", "Ask for what you want",
     "Press Super+S anywhere. A prompt opens over what you are doing; say what you need in plain language and the agent does it on this computer.",
     ["Super", "S"], "“Summarize the page I have open.”  “Install a PDF editor.”  “Why is the fan loud?”"),
    ("video-display-symbolic", "It has a screen of its own",
     "The agent works with its own pointer and keyboard on a screen you do not see, so it never takes yours. The AI workspace, at the left of the menu bar, shows what it is doing; ask it to bring a window over when you want to use it.",
     [], "“Open my notes and bring them here.”"),
    ("security-high-symbolic", "It asks before what cannot be undone",
     "Ordinary work just runs. For anything that cannot be taken back (deleting outside your files' snapshots, administrator commands) a card appears with Allow and Deny, and says why it asks.",
     [], None),
    ("edit-undo-symbolic", "Undo a whole task",
     "Before the agent changes your files, SlateOS takes a snapshot. If a task went wrong, undo it as one step.",
     ["/undo"], "In the terminal: /undo --preview shows what would come back."),
    ("utilities-terminal-symbolic", "The terminal speaks both",
     "The terminal's shell is slash. Plain language goes to the agent; a line that starts with ! runs as a command; / is for slash itself.",
     ["!ls", "/help"], "“which of these files changed this week?”"),
    ("preferences-system-symbolic", "The rest is where you expect",
     "Super+Space opens apps. Settings has Wi-Fi, Bluetooth, displays, input methods and the agent's connection. The store on the dock installs apps from Flathub.",
     ["Super", "Space"], None),
]


class Tour(Gtk.Box):
    """The tutorial as pages to flip through (and that turn by themselves when `auto`)."""

    def __init__(self, auto=0, compact=False):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.carousel = Adw.Carousel(hexpand=True, vexpand=not compact, spacing=24, allow_long_swipes=True)
        for icon, title, text, keys, example in TOUR:
            page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8 if compact else 12, valign=Gtk.Align.CENTER,
                           margin_start=28, margin_end=28, margin_top=4, margin_bottom=4)
            head = Gtk.Box(spacing=12, halign=Gtk.Align.CENTER)
            head.append(Gtk.Image(icon_name=icon, pixel_size=28 if compact else 40))
            head.append(Gtk.Label(label=title, css_classes=["tour-title"]))
            page.append(head)
            page.append(Gtk.Label(label=text, wrap=True, justify=Gtk.Justification.CENTER, max_width_chars=64, css_classes=["tour-body"]))
            if keys:
                caps = Gtk.Box(spacing=6, halign=Gtk.Align.CENTER)
                for i, key in enumerate(keys):
                    if i and len(keys) == 2 and keys[0] == "Super":
                        caps.append(Gtk.Label(label="+"))
                    caps.append(Gtk.Label(label=key, css_classes=["tour-key"]))
                page.append(caps)
            if example:
                page.append(Gtk.Label(label=example, wrap=True, justify=Gtk.Justification.CENTER, max_width_chars=64, css_classes=["tour-example"]))
            self.carousel.append(page)
        self.append(self.carousel)
        self.append(Adw.CarouselIndicatorDots(carousel=self.carousel))
        self.timer = None
        if auto:
            self.connect("map", lambda *_: self.start(auto))
            self.connect("unmap", lambda *_: self.stop())

    def start(self, seconds):
        if self.timer is None:
            self.timer = GLib.timeout_add_seconds(seconds, lambda: (self.turn(1, wrap=True), True)[1])

    def stop(self):
        if self.timer is not None:
            GLib.source_remove(self.timer)
            self.timer = None

    def position(self):
        return round(self.carousel.get_position())

    def at_end(self):
        return self.position() >= len(TOUR) - 1

    def turn(self, delta, wrap=False):
        n = self.position() + delta
        if wrap:
            n %= len(TOUR)
        n = max(0, min(len(TOUR) - 1, n))
        self.carousel.scroll_to(self.carousel.get_nth_page(n), True)
