"""How the agent reaches a model, and how a new person learns the desktop: what the
installer, the first-run welcome and Settings share.

`AgentSetup` is the form: a Claude subscription (Claude Code's own sign-in), a ChatGPT
subscription (Codex's own sign-in), or an API of the person's own, in OpenAI's or
Anthropic's format (an address, a key, a model). It writes
`~/.config/slate/provider.toml`, which slash reads, and tries a connection with
`slate provider test`, the same code path the agent's requests take.

The agent's programs, Claude Code and Codex, are not part of SlateOS's image: each
computer downloads them, and the form offers that where the program a choice needs
is missing (`slateos-settings set agents` on an installed system,
`slateos-live-agents` in the live session).

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

# The same places slash looks in (XDG, with the usual defaults).
CONFIG_HOME = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
PROVIDER = os.path.join(CONFIG_HOME, "slate", "provider.toml")
SLASH_CONFIG = os.path.join(CONFIG_HOME, "slate", "slash.toml")
WELCOME_DONE = os.path.join(os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"), "slate", "welcome-done")

# The agent's programs, by the name of their package: what a person calls it, its
# command, where a download comes from, and what slash calls it (slash.toml's backend).
PROGRAMS = {
    "claude-code": ("Claude Code", "claude", "Anthropic", "claude"),
    "codex": ("Codex", "codex", "the NixOS package cache", "codex"),
}
# On the live image: the programs `slateos-live-agents` can bring (distro/iso/live.nix).
LIVE_AGENTS = "/etc/slateos/agents.json"
# On an installed system: the programs `slateos-settings set agents` can install.
SYSTEM_SETTINGS = "/etc/slate/system-settings.json"
SLATEOS_SETTINGS = "/run/current-system/sw/bin/slateos-settings"

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
    """The session's environment, with the places agent programs are installed to: a
    person's own, the system's, and the profile the live session downloads them into."""
    env = dict(os.environ)
    extra = [os.path.expanduser(p) for p in ("~/.npm-global/bin", "~/.local/bin", "~/.cargo/bin")]
    env["PATH"] = ":".join(extra + [env.get("PATH", ""), "/run/current-system/sw/bin", "/nix/var/nix/profiles/default/bin"])
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


# ---- slash.toml: which program is the agent ----------------------------------------

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


def slash_config_text(cfg):
    """slash.toml is small and flat (scalars at the top, one level of tables): written by hand."""
    lines = ["# written by Slate Settings; slash reads this at start"]
    for k, v in cfg.items():
        if not isinstance(v, dict):
            lines.append(f"{k} = {toml_value(v)}")
    for k, v in cfg.items():
        if isinstance(v, dict):
            lines.append(f"\n[{k}]")
            for k2, v2 in v.items():
                lines.append(f"{k2} = {toml_value(v2)}")
    return "\n".join(lines) + "\n"


def save_slash_config(cfg):
    os.makedirs(os.path.dirname(SLASH_CONFIG), exist_ok=True)
    with open(SLASH_CONFIG, "w") as f:
        f.write(slash_config_text(cfg))


def backend():
    """The program slash runs as the agent: "claude" or "codex"."""
    name = load_slash_config().get("backend", "claude")
    return name if name in ("claude", "codex") else "claude"


def set_backend(name):
    cfg = load_slash_config()
    if cfg.get("backend", "claude") != name:
        cfg["backend"] = name
        save_slash_config(cfg)


# ---- the agent's programs ----------------------------------------------------------

def is_live():
    """The live image's session, where a program is downloaded into memory."""
    return os.path.exists(LIVE_AGENTS)


def has_program(name):
    return shutil.which(PROGRAMS[name][1], path=environment()["PATH"]) is not None


def offered_programs():
    """{package name: version} of the programs this system can download."""
    def live(data):
        return {name: (entry or {}).get("version", "") for name, entry in data.items()}

    def installed(data):
        return data.get("agents", {}).get("available", {})

    for path, pick in ((LIVE_AGENTS, live), (SYSTEM_SETTINGS, installed)):
        try:
            with open(path, encoding="utf-8") as f:
                found = pick(json.load(f))
        except (OSError, ValueError, AttributeError):
            continue
        found = {name: version for name, version in dict(found).items() if name in PROGRAMS}
        if found:
            return found
    return {}


def programs_summary():
    """One line on the agent's programs this system has, for Settings."""
    versions = offered_programs()
    have = [name for name in PROGRAMS if has_program(name)]
    lack = [name for name in PROGRAMS if name not in have]
    parts = []
    if have:
        parts.append(", ".join(PROGRAMS[n][0] + (f" {versions[n]}" if versions.get(n) else "") for n in have) + " installed")
    if lack:
        parts.append(" and ".join(PROGRAMS[n][0] for n in lack) + (" is" if len(lack) == 1 else " are") + " not installed")
    return "; ".join(parts)


def download_command(need):
    """(the programs, the command that brings them, its environment). The live session
    takes the one program that is needed, because it goes into memory; an installed
    system takes every program its release offers."""
    if is_live():
        return [need], ["sudo", "-n", "slateos-live-agents", need], environment()
    names = sorted(offered_programs()) or [need]
    env = dict(environment(), SLATE_ASKPASS_TITLE="SlateOS needs your password",
               SLATE_ASKPASS_BODY="To download and install the agent's programs (" + ", ".join(PROGRAMS[n][0] for n in names) + ").")
    return names, ["sudo", "-A", SLATEOS_SETTINGS, "set", "agents", ",".join(names)], env


def progress_step(line):
    """What to show for a line of the download's output; None leaves what is shown."""
    low = line.lower()
    if line.startswith("== "):
        return line[3:4].upper() + line[4:]
    if low.startswith("building the system configuration") or low.startswith("updating the system"):
        return "Preparing (a minute or two)…"
    if low.startswith("copying path") or "downloading" in low:
        return "Downloading…"
    if low.startswith("building '"):
        return "Putting the programs in place…"
    if low.startswith("activating the configuration"):
        return "Almost done…"
    return None


def download_problem(tail):
    """One line on why a download did not finish, from the end of its output."""
    lines = [t for t in tail if t.strip()]
    if lines and all(t.startswith("sudo:") for t in lines):
        return "Nothing was downloaded: the password was not given."
    told = [t for t in lines if not t.startswith(("sudo:", "=="))]
    reason = next((t for t in reversed(told) if t.lower().startswith("error:")), told[-1] if told else "")
    return f"The download did not finish: {reason[:260]}" if reason else "The download did not finish."


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


def codex_account():
    """Codex names no account: "" when it is signed in, None when not."""
    code, _out = run(["codex", "login", "status"])
    return "" if code == 0 else None


ACCOUNT = {"claude-code": claude_account, "codex": codex_account}
LOGIN = {"claude-code": "claude auth login", "codex": "codex login"}


def describe(provider=None):
    """One line on how the agent connects now, for a summary."""
    if backend() == "codex":
        if not has_program("codex"):
            return "Not set up: Codex is not installed"
        return "ChatGPT subscription (Codex)" if codex_account() is not None else "Not connected: Codex is not signed in"
    if not has_program("claude-code"):
        return "Not set up: Claude Code is not installed"
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
    """Whether the agent can work: its program is on this system, and it reaches a
    model (an API connection, or the program's own sign-in)."""
    if backend() == "codex":
        return has_program("codex") and codex_account() is not None
    if not has_program("claude-code"):
        return False
    provider = load_provider()
    if provider.get("connection") == "api" and provider.get("api", {}).get("api_key"):
        return True
    return claude_account() is not None


def install_css():
    from gi.repository import Gdk
    css = Gtk.CssProvider()
    css.load_from_string(CSS)
    Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)


def confirm_without_agent(window, on_skip):
    """Going on without an agent is possible and a bad idea: say so before it happens."""
    dialog = Adw.AlertDialog(
        heading="Continue without an agent?",
        body="This is strongly discouraged. SlateOS is built around its agent: without one the prompt (Super+S) and "
             "the terminal do not answer, and nothing gets done by asking. What is left is a bare desktop.\n\n"
             "You can connect an agent later in Settings, AI. Until you do, SlateOS asks again at every login.")
    dialog.add_response("back", "Go Back")
    dialog.add_response("skip", "Continue Without an Agent")
    dialog.set_response_appearance("skip", Adw.ResponseAppearance.DESTRUCTIVE)
    dialog.set_default_response("back")
    dialog.set_close_response("back")
    dialog.connect("response", lambda _d, response: response == "skip" and on_skip())
    dialog.present(window)


# ---- the form ----------------------------------------------------------------------

class AgentSetup(Gtk.Box):
    """The choice between a subscription and an API of one's own, with its fields.

    `on_change()` is called whenever `complete` may have changed. `complete` is true
    when the agent can work: the program the choice needs is on this system, and it
    is signed in, or an API connection answered a test (or the person chose to keep
    it although it did not).

    `installing` is the installer's form. There an API connection is complete without
    Claude Code: the installation downloads the programs to the disk. A sign-in needs
    its program at once, in the live session.
    """

    def __init__(self, on_change=None, window=None, installing=False):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.on_change = on_change or (lambda: None)
        self.window = window
        self.installing = installing
        self.present = {name: has_program(name) for name in PROGRAMS}
        self.accounts = {name: None for name in PROGRAMS}   # the programs' sign-ins, as last seen
        self.tested = None           # the API fields that last passed a test (or were kept anyway)
        self.busy = False
        self.downloading = False
        self.problem = None          # why the last download did not finish
        self.poll = None
        saved = load_provider().get("api") or {}

        page = Adw.PreferencesPage()
        self.append(page)
        how = Adw.PreferencesGroup(
            title="Connect your agent",
            description="SlateOS works through an AI agent. Choose how it reaches a model; you can change this later in Settings, AI.")
        page.add(how)
        self.sub_row = Adw.ActionRow(title="Claude subscription", subtitle="Sign in with your Claude account (Pro or Max). The agent is Claude Code.", activatable=True)
        self.sub_check = Gtk.CheckButton(valign=Gtk.Align.CENTER)
        self.sub_row.add_prefix(self.sub_check)
        self.sub_row.set_activatable_widget(self.sub_check)
        how.add(self.sub_row)
        self.gpt_row = Adw.ActionRow(title="ChatGPT subscription", subtitle="Sign in with your ChatGPT account (Plus or Pro). The agent is Codex: it works in its own sandbox, without SlateOS's approvals and undo.", activatable=True)
        self.gpt_check = Gtk.CheckButton(valign=Gtk.Align.CENTER, group=self.sub_check)
        self.gpt_row.add_prefix(self.gpt_check)
        self.gpt_row.set_activatable_widget(self.gpt_check)
        how.add(self.gpt_row)
        self.api_row = Adw.ActionRow(title="An API key of your own", subtitle="OpenAI, Anthropic, or any service that speaks one of their formats: an address, a key, a model.", activatable=True)
        self.api_check = Gtk.CheckButton(valign=Gtk.Align.CENTER, group=self.sub_check)
        self.api_row.add_prefix(self.api_check)
        self.api_row.set_activatable_widget(self.api_check)
        how.add(self.api_row)

        # The program the choice needs, where this computer does not have it yet.
        self.prog_group = Adw.PreferencesGroup()
        page.add(self.prog_group)
        self.prog_row = Adw.ActionRow(title="", subtitle="")
        self.prog_spinner = Gtk.Spinner(valign=Gtk.Align.CENTER)
        self.prog_row.add_suffix(self.prog_spinner)
        self.prog_button = Gtk.Button(label="Download", valign=Gtk.Align.CENTER, css_classes=["suggested-action"])
        self.prog_button.connect("clicked", self.download)
        self.prog_row.add_suffix(self.prog_button)
        self.prog_group.add(self.prog_row)

        # A subscription: the program signs in by itself, in a terminal and the browser.
        self.sub_group = Adw.PreferencesGroup()
        page.add(self.sub_group)
        self.signin_row = Adw.ActionRow(title="Claude Code", subtitle="Checking…")
        self.signin = Gtk.Button(label="Sign In…", valign=Gtk.Align.CENTER, css_classes=["suggested-action"])
        self.signin.connect("clicked", self.sign_in)
        self.signin_row.add_suffix(self.signin)
        self.sub_group.add(self.signin_row)
        self.sub_note = Gtk.Label(label="", wrap=True, xalign=0, css_classes=["dim-label", "caption"], margin_top=8, margin_start=4, margin_end=4)
        self.sub_group.add(self.sub_note)

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

        if installing:
            # Where the programs come from: the image has none.
            note = Adw.PreferencesGroup()
            note.add(Gtk.Label(
                label="Claude Code and Codex are not part of SlateOS's image. The installation downloads them (a few hundred "
                      "megabytes): keep the internet connected. Without it SlateOS is installed without them, and asks again "
                      "at its first start.",
                wrap=True, xalign=0, css_classes=["dim-label", "caption"], margin_start=4, margin_end=4))
            page.add(note)

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
        if backend() == "codex":
            self.gpt_check.set_active(True)
        elif load_provider().get("connection") == "api":
            self.api_check.set_active(True)
        else:
            self.sub_check.set_active(True)
        # Every button: the one that is switched off reports before the other is on.
        for check in (self.sub_check, self.gpt_check, self.api_check):
            check.connect("toggled", lambda *_: (self.changed(), self.refresh()))
        self.connect("map", lambda *_: self.start_polling())
        self.connect("unmap", lambda *_: self.stop_polling())
        self.changed()
        self.refresh()

    # State

    def mode(self):
        return "api" if self.api_check.get_active() else "chatgpt" if self.gpt_check.get_active() else "subscription"

    def needed(self):
        """The program the choice runs as the agent."""
        return "codex" if self.mode() == "chatgpt" else "claude-code"

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
        need = self.needed()
        if self.mode() == "api":
            return self.fields_problem() is None and self.tested == self.fields() and (self.installing or self.present[need])
        return self.present[need] and self.accounts[need] is not None

    def summary(self):
        mode = self.mode()
        if mode == "api":
            f = self.fields()
            return f"API at {f['base_url'].split('://')[-1].split('/')[0]}" + (f", {f['model']}" if f["model"] else "")
        account = self.accounts[self.needed()]
        return ("ChatGPT subscription (Codex)" if mode == "chatgpt" else "Claude subscription") + (f" ({account})" if account else "")

    def changed(self):
        api = self.mode() == "api"
        need = self.needed()
        title, _command, source, _backend = PROGRAMS[need]
        present = self.present[need]
        self.sub_group.set_visible(not api)
        self.api_group.set_visible(api)

        # The program: asked for where the choice cannot go on without it.
        wanted = not present and not (self.installing and api)
        self.prog_group.set_visible(wanted or self.downloading)
        self.prog_button.set_visible(not self.downloading)
        if wanted and not self.downloading:
            self.prog_row.set_title(f"{title} is not on this computer yet")
            if self.problem:
                self.prog_row.set_subtitle(self.problem)
                self.prog_row.add_css_class("agent-bad")
                self.prog_button.set_label("Try Again")
            else:
                self.prog_row.remove_css_class("agent-bad")
                self.prog_button.set_label("Download")
                if is_live():
                    self.prog_row.set_subtitle(f"SlateOS's image does not include it. It comes from {source}: a few hundred megabytes, over the internet.")
                else:
                    self.prog_row.set_subtitle("SlateOS does not ship the agent's programs. Downloading them takes the internet, your password and a few minutes.")

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
        else:
            account = self.accounts[need]
            self.signin_row.set_title(title)
            self.signin.set_sensitive(present)
            if not present:
                self.signin_row.set_subtitle("Download it first")
                self.signin.set_label("Sign In…")
            elif account is None:
                self.signin_row.set_subtitle("Not signed in")
                self.signin.set_label("Sign In…")
                self.signin.add_css_class("suggested-action")
            else:
                self.signin_row.set_subtitle("Signed in" + (f" as {account}" if account else ""))
                self.signin.set_label("Sign In Again…")
                self.signin.remove_css_class("suggested-action")
            self.sub_note.set_label(f"Sign-in happens in {title} itself: a terminal opens and shows a link for the browser. SlateOS does not see your password.")
        self.on_change()

    def apply_preset(self):
        preset = PRESETS[self.preset.get_selected()]
        self.format.set_selected([f for f, _ in FORMATS].index(preset["format"]))
        custom = preset["id"].startswith("custom-")
        # A known service has one format; the row is for the others (and keeps the
        # form short enough to show its Test button without scrolling).
        self.format.set_visible(custom)
        if preset["base_url"] or custom:
            self.address.set_text(preset["base_url"])
        self.model.set_title("Model" if preset["format"] == "openai" else "Model (empty: Claude Code's own choice)")
        self.changed()

    # The programs and their sign-ins

    def refresh(self):
        """Look again at which programs are here and whether the needed one is signed in."""
        need, signs_in = self.needed(), self.mode() != "api"

        def work():
            present = {name: has_program(name) for name in PROGRAMS}
            return present, (ACCOUNT[need]() if signs_in and present[need] else None)

        def done(result):
            if isinstance(result, Exception):
                return
            present, account = result
            before = (self.present, self.accounts[need])
            self.present = present
            if signs_in:
                self.accounts[need] = account
            if before != (self.present, self.accounts[need]):
                self.changed()
        in_thread(work, done)

    def start_polling(self):
        if self.poll is None:
            self.poll = GLib.timeout_add_seconds(4, lambda: (self.refresh(), True)[1])

    def stop_polling(self):
        if self.poll is not None:
            GLib.source_remove(self.poll)
            self.poll = None

    def download(self, *_):
        if self.downloading:
            return
        need = self.needed()
        names, cmd, env = download_command(need)
        self.downloading = True
        self.problem = None
        self.prog_row.remove_css_class("agent-bad")
        self.prog_row.set_title("Downloading " + " and ".join(PROGRAMS[n][0] for n in names))
        self.prog_row.set_subtitle("Starting…")
        self.prog_spinner.start()
        self.changed()

        def work():
            p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, errors="replace", env=env)
            tail = []
            for line in p.stdout:
                line = line.strip()
                if not line:
                    continue
                tail = (tail + [line])[-30:]
                step = progress_step(line)
                if step:
                    GLib.idle_add(lambda s=step: (self.downloading and self.prog_row.set_subtitle(s), False)[1])
            return p.wait(), tail

        def done(result):
            code, tail = (1, [str(result)]) if isinstance(result, Exception) else result
            self.downloading = False
            self.prog_spinner.stop()
            self.present = {name: has_program(name) for name in PROGRAMS}
            if code != 0 or not self.present[need]:
                self.problem = download_problem(tail)
            self.changed()
            self.refresh()
        in_thread(work, done)

    def sign_in(self, *_):
        need = self.needed()
        # The program prints a link, opens the browser and waits for the code; the
        # shell afterwards keeps the window readable.
        script = f'{LOGIN[need]}; echo; echo "You can close this window."; exec ${{SHELL:-sh}}'
        subprocess.Popen(["foot", "--title", f"Sign in to {PROGRAMS[need][0]}", "-e", "sh", "-c", script], env=environment())

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
        A subscription is the program's own sign-in files, copied as they are."""
        mode = self.mode()
        if mode == "api":
            return {".config/slate/provider.toml": provider_text(self.fields())}
        out = {".config/slate/provider.toml": provider_text(None)}
        if mode == "chatgpt":
            out[".config/slate/slash.toml"] = slash_config_text({"backend": "codex"})
            copied = (".codex/auth.json",)
        else:
            copied = (".claude/.credentials.json", ".claude.json")
        for rel in copied:
            try:
                with open(os.path.join(os.path.expanduser("~"), rel)) as f:
                    out[rel] = f.read()
            except OSError:
                pass
        return out

    def save(self):
        """Make the choice this person's own (slash reads it at its next start)."""
        mode = self.mode()
        write_private(PROVIDER, provider_text(self.fields() if mode == "api" else None))
        set_backend(PROGRAMS[self.needed()][3])


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
            # As wide as the carousel, so the next page does not show at the edge.
            page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8 if compact else 12, valign=Gtk.Align.CENTER, hexpand=True,
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
