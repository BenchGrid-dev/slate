#!/usr/bin/env python3
"""slateos-installer: put SlateOS on this computer's disk, from the live image.

    slateos-installer            the installer
    slateos-installer --welcome  first the choice between trying and installing; the
                                 live session opens it at login

It asks where (a whole disk, which is erased), who (user name, password, computer
name), how the agent reaches a model (a Claude subscription or an API key: a required
step, because SlateOS without an agent does not answer) and how (time zone,
applications, disk encryption), then runs slateos-install with those answers and
follows its progress, with the short tutorial to read meanwhile. The password reaches
slateos-install as a hash only; the disk passphrase and the agent's key go through
files only the person and root can read.

For development: --dry-run passes --dry-run on (partitions and writes the
configuration, then stops), --disk PATH offers PATH (a loop device) besides the real
disks, and SLATEOS_INSTALL_COMMAND replaces "sudo -n slateos-install".
"""
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk  # noqa: E402

# The agent's connection and the tutorial are shared with the installed desktop
# (distro/desktop/slate_agent.py); the package puts the module next to this program.
LIB = "@lib@"
sys.path.insert(0, LIB if not LIB.startswith("@") else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "desktop"))
import slate_agent  # noqa: E402

APP_ID = "dev.benchgrid.slate.Installer"
# Absolute paths, filled in by the package (sudo may not keep PATH).
INSTALL = "@install@"
MKPASSWD = "@mkpasswd@"
INHIBIT = "@inhibit@"
if INSTALL.startswith("@"):
    INSTALL = "slateos-install"
if MKPASSWD.startswith("@"):
    MKPASSWD = "mkpasswd"
if INHIBIT.startswith("@"):
    INHIBIT = "systemd-inhibit"
# The install's output as it comes, for when the window cannot show it.
LOG_FILE = "/tmp/slateos-install.log"

GB = 1024**3  # what virtual machine settings and most tools call a GB
MIN_DISK = 20 * GB
RESERVED_USERS = {
    "root", "nobody", "daemon", "bin", "sys", "nixos", "nixbld", "messagebus", "polkituser",
    "sshd", "avahi", "greeter", "systemd-network", "systemd-resolve", "systemd-timesync",
}
CSS = b"""
.welcome-buttons button { padding: 10px 26px; font-size: 15px; }
.log { font-family: monospace; font-size: 11px; }
.stage { font-size: 15px; font-weight: 600; }
"""

# What slateos-install prints at each step, and how far along that is.
STAGES = [
    ("== partitioning", 0.03, "Preparing the disk"),
    ("== encrypting", 0.06, "Encrypting the disk"),
    ("== subvolumes", 0.08, "Preparing the disk"),
    ("== configuration", 0.10, "Writing the configuration"),
    ("== installing", 0.12, "Copying SlateOS to the disk"),
    ("installing the boot loader", 0.95, "Installing the boot loader"),
    ("setting up /etc", 0.97, "Finishing"),
    ("installation finished", 0.99, "Finishing"),
]


def human_size(n):
    return f"{n / GB:.0f} GB" if n >= 10 * GB else f"{n / GB:.1f} GB"


def live_medium_disk():
    """The disk the live image booted from, which must not be offered."""
    try:
        src = subprocess.run(["findmnt", "-n", "-o", "SOURCE", "/iso"], capture_output=True, text=True).stdout.strip()
        if not src:
            return None
        parent = subprocess.run(["lsblk", "-n", "-o", "PKNAME", src], capture_output=True, text=True).stdout.strip()
        return "/dev/" + (parent or os.path.basename(src))
    except OSError:
        return None


def find_disks(extra=None):
    """Whole disks: (path, size in bytes, description)."""
    out = subprocess.run(["lsblk", "-J", "-b", "-d", "-o", "NAME,PATH,SIZE,MODEL,TYPE,RO,TRAN"], capture_output=True, text=True).stdout
    try:
        devices = json.loads(out).get("blockdevices", [])
    except ValueError:
        devices = []
    skip = live_medium_disk()
    disks = []
    for d in devices:
        name = d.get("name") or ""
        if d.get("type") != "disk" or d.get("ro") in (True, "1") or name.startswith(("zram", "loop", "sr")):
            continue
        if d.get("path") == skip:
            continue
        model = (d.get("model") or "").strip()
        tran = (d.get("tran") or "").upper()
        if not model:
            model = "Virtual disk" if name.startswith("vd") else "Disk"
        desc = f"{model} · {tran}" if tran else model
        disks.append((d["path"], int(d.get("size") or 0), desc))
    if extra:
        size = 0
        try:
            size = int(subprocess.run(["blockdev", "--getsize64", extra], capture_output=True, text=True).stdout.strip() or 0)
        except (OSError, ValueError):
            pass
        disks.append((extra, size or MIN_DISK, "Test device"))
    return disks


def time_zones():
    zones = set()
    for tab in ("/etc/zoneinfo/zone1970.tab", "/usr/share/zoneinfo/zone1970.tab"):
        try:
            with open(tab) as f:
                for line in f:
                    if line.strip() and not line.startswith("#"):
                        parts = line.split("\t")
                        if len(parts) >= 3:
                            zones.add(parts[2].strip())
            break
        except OSError:
            continue
    return ["UTC"] + sorted(zones)


def current_zone():
    try:
        tz = subprocess.run(["timedatectl", "show", "-p", "Timezone", "--value"], capture_output=True, text=True).stdout.strip()
        return tz or "UTC"
    except OSError:
        return "UTC"


def radio_row(title, subtitle, group=None):
    check = Gtk.CheckButton()
    if group is not None:
        check.set_group(group)
    row = Adw.ActionRow(title=title, subtitle=subtitle)
    row.add_prefix(check)
    row.set_activatable_widget(check)
    return row, check


class Installer(Adw.ApplicationWindow):
    def __init__(self, app, welcome, dry_run, extra_disk):
        super().__init__(application=app, title="Install SlateOS", default_width=720, default_height=670)
        self.dry_run = dry_run
        self.extra_disk = extra_disk
        self.running = False
        self.inhibit_cookie = 0
        self.log_file = None
        self.log_lines = []
        self.disk_checks = []
        self.copied = 0
        self.expected_paths = 1

        self.header_title = Adw.WindowTitle(title="Install SlateOS")
        header = Adw.HeaderBar(title_widget=self.header_title)
        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.SLIDE_LEFT_RIGHT, vexpand=True)

        self.back = Gtk.Button(label="Back")
        self.back.connect("clicked", lambda *_: self.go(-1))
        self.next = Gtk.Button(label="Continue", css_classes=["suggested-action"])
        self.next.connect("clicked", lambda *_: self.go(1))
        bar = Gtk.Box(spacing=8, margin_top=10, margin_bottom=12, margin_start=16, margin_end=16)
        bar.append(self.back)
        bar.append(Gtk.Box(hexpand=True))
        bar.append(self.next)

        self.toolbar = Adw.ToolbarView()
        self.toolbar.add_top_bar(header)
        self.toolbar.set_content(self.stack)
        self.toolbar.add_bottom_bar(bar)
        self.toast = Adw.ToastOverlay(child=self.toolbar)
        self.set_content(self.toast)

        self.stack.add_named(self.welcome_page(), "welcome")
        self.steps = ["disk", "account", "agent", "setup", "review"]
        self.stack.add_named(self.disk_page(), "disk")
        self.stack.add_named(self.account_page(), "account")
        self.home_files = None
        self.agent = slate_agent.AgentSetup(on_change=self.update_next, window=self)
        self.stack.add_named(self.agent, "agent")
        self.stack.add_named(self.setup_page(), "setup")
        self.review_group = Adw.PreferencesGroup(title="Ready to install")
        review = Adw.PreferencesPage()
        review.add(self.review_group)
        self.stack.add_named(review, "review")
        self.stack.add_named(self.progress_page(), "progress")
        self.stack.add_named(self.done_page(), "done")
        self.stack.add_named(self.failed_page(), "failed")

        self.connect("close-request", self.on_close_request)
        self.show("welcome" if welcome else "disk")

    # Pages

    def welcome_page(self):
        page = Adw.StatusPage(
            icon_name="slateos",
            title="Welcome to SlateOS",
            description="Try SlateOS without changing anything on this computer, or install it.\n"
            "You can also install later: Install SlateOS is in the dock.",
        )
        box = Gtk.Box(spacing=12, halign=Gtk.Align.CENTER, css_classes=["welcome-buttons"])
        try_btn = Gtk.Button(label="Try SlateOS", css_classes=["pill"])
        try_btn.connect("clicked", lambda *_: self.close())
        install_btn = Gtk.Button(label="Install SlateOS", css_classes=["pill", "suggested-action"])
        install_btn.connect("clicked", lambda *_: self.show("disk"))
        box.append(try_btn)
        box.append(install_btn)
        page.set_child(box)
        return page

    def disk_page(self):
        page = Adw.PreferencesPage()
        self.disk_group = Adw.PreferencesGroup(
            title="Where should SlateOS go?",
            description="SlateOS takes a whole disk. Everything on the disk you choose is erased.",
        )
        refresh = Gtk.Button(icon_name="view-refresh-symbolic", valign=Gtk.Align.CENTER, tooltip_text="Look for disks again", css_classes=["flat"])
        refresh.connect("clicked", lambda *_: self.fill_disks())
        self.disk_group.set_header_suffix(refresh)
        page.add(self.disk_group)
        self.disk_rows = []
        self.fill_disks()
        return page

    def fill_disks(self):
        for row in self.disk_rows:
            self.disk_group.remove(row)
        self.disk_rows, self.disk_checks = [], []
        # Always a radio group, even for one disk: a lone check button can be clicked
        # off again, and nothing would then be chosen.
        group = self.disk_group_leader = Gtk.CheckButton()
        for path, size, desc in find_disks(self.extra_disk):
            small = size < MIN_DISK
            subtitle = f"{human_size(size)} · {desc}" + (" · too small, SlateOS needs 20 GB" if small else "")
            row, check = radio_row(path, subtitle, group)
            row.set_sensitive(not small)
            check.disk = path
            check.connect("toggled", lambda *_: self.update_next())
            self.disk_group.add(row)
            self.disk_rows.append(row)
            self.disk_checks.append(check)
        usable = [c for c, r in zip(self.disk_checks, self.disk_rows) if r.get_sensitive()]
        if len(usable) == 1:
            usable[0].set_active(True)
        if not self.disk_rows:
            row = Adw.ActionRow(
                title="No disk found",
                subtitle="Add a disk of at least 20 GB (in a virtual machine: in its settings, then restart it), then look again.",
            )
            self.disk_group.add(row)
            self.disk_rows.append(row)
        self.update_next()

    def account_page(self):
        page = Adw.PreferencesPage()
        group = Adw.PreferencesGroup(title="Who will use it?", description="This account signs in automatically; the password is for administrator tasks and the lock screen.")
        self.user_row = Adw.EntryRow(title="User name")
        self.user_row.set_text("")
        self.pw_row = Adw.PasswordEntryRow(title="Password")
        self.pw2_row = Adw.PasswordEntryRow(title="Password again")
        self.host_row = Adw.EntryRow(title="Computer name")
        self.host_row.set_text("slateos")
        for r in (self.user_row, self.pw_row, self.pw2_row, self.host_row):
            r.connect("changed", lambda *_: self.update_next())
            group.add(r)
        self.account_hint = Gtk.Label(xalign=0, wrap=True, css_classes=["dim-label", "caption"], margin_top=8)
        group.add(self.account_hint)
        page.add(group)
        return page

    def setup_page(self):
        page = Adw.PreferencesPage()
        where = Adw.PreferencesGroup(title="Time zone")
        self.zones = time_zones()
        # Shown as "America/New York", searched anywhere in the name ("york" finds it).
        names = [z.replace("_", " ") for z in self.zones]
        self.zone_row = Adw.ComboRow(title="Time zone", model=Gtk.StringList.new(names), enable_search=True)
        self.zone_row.set_expression(Gtk.PropertyExpression.new(Gtk.StringObject, None, "string"))
        self.zone_row.set_search_match_mode(Gtk.StringFilterMatchMode.SUBSTRING)
        cur = current_zone()
        self.zone_row.set_selected(self.zones.index(cur) if cur in self.zones else 0)
        where.add(self.zone_row)
        page.add(where)

        apps = Adw.PreferencesGroup(title="Applications")
        self.full_row, self.full_check = radio_row("Everyday apps", "Office suite, mail, and the basics. Downloads the extra apps, so it needs the internet.")
        self.min_row, self.min_check = radio_row("Minimal", "Browser, files, terminal, text editor and the app store. Installs from this disk; no internet needed.", self.full_check)
        self.full_check.set_active(True)
        apps.add(self.full_row)
        apps.add(self.min_row)
        page.add(apps)

        security = Adw.PreferencesGroup(title="Security")
        self.encrypt_row = Adw.SwitchRow(
            title="Encrypt the disk",
            subtitle="A passphrase is asked at every start; without it nobody can read the disk.",
        )
        self.pass_row = Adw.PasswordEntryRow(title="Disk passphrase", visible=False)
        self.pass2_row = Adw.PasswordEntryRow(title="Disk passphrase again", visible=False)

        def on_encrypt(*_):
            on = self.encrypt_row.get_active()
            self.pass_row.set_visible(on)
            self.pass2_row.set_visible(on)
            self.update_next()

        self.encrypt_row.connect("notify::active", on_encrypt)
        for r in (self.pass_row, self.pass2_row):
            r.connect("changed", lambda *_: self.update_next())
        security.add(self.encrypt_row)
        security.add(self.pass_row)
        security.add(self.pass2_row)
        page.add(security)
        return page

    def progress_page(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10, margin_top=22, margin_bottom=16, margin_start=48, margin_end=48)
        head = Gtk.Box(spacing=14, halign=Gtk.Align.CENTER)
        head.append(Gtk.Image(icon_name="slateos", pixel_size=48))
        head.append(Gtk.Label(label="Installing SlateOS", css_classes=["title-1"]))
        box.append(head)
        self.stage = Gtk.Label(label="Starting", css_classes=["stage"])
        box.append(self.stage)
        self.bar = Gtk.ProgressBar(margin_top=2)
        box.append(self.bar)
        box.append(Gtk.Label(label="This takes a while. Keep the computer on and connected to power.", css_classes=["dim-label"], wrap=True))
        # Something to read meanwhile: how to work with the agent. It turns by itself.
        self.tour = slate_agent.Tour(auto=14, compact=True)
        self.tour.set_margin_top(10)
        self.tour.set_vexpand(True)
        box.append(self.tour)
        self.log_view = Gtk.TextView(editable=False, monospace=True, cursor_visible=False, css_classes=["log"], wrap_mode=Gtk.WrapMode.CHAR)
        scroll = Gtk.ScrolledWindow(child=self.log_view, min_content_height=150)
        self.details = Gtk.Expander(label="Details", child=scroll)
        self.details.connect("notify::expanded", lambda e, *_: self.tour.set_visible(not e.get_expanded()))
        box.append(self.details)
        return box

    def done_page(self):
        page = Adw.StatusPage(
            # In Adwaita too: at login the window can open before the icon theme applies.
            icon_name="object-select-symbolic",
            title="SlateOS is installed",
            description="Remove the installation medium, then restart. In a virtual machine, eject the "
            "ISO image first (UTM: the CD/DVD button in the toolbar, then Clear).\n"
            "After the restart, sign in to your AI from Settings → AI and press Super+S.",
        )
        box = Gtk.Box(spacing=12, halign=Gtk.Align.CENTER, css_classes=["welcome-buttons"])
        keep = Gtk.Button(label="Keep Trying", css_classes=["pill"])
        keep.connect("clicked", lambda *_: self.close())
        restart = Gtk.Button(label="Restart Now", css_classes=["pill", "suggested-action"])
        restart.connect("clicked", lambda *_: subprocess.Popen(["systemctl", "reboot"]))
        box.append(keep)
        box.append(restart)
        page.set_child(box)
        return page

    def failed_page(self):
        self.failed = Adw.StatusPage(icon_name="dialog-error-symbolic", title="The installation did not finish")
        box = Gtk.Box(spacing=12, halign=Gtk.Align.CENTER, css_classes=["welcome-buttons"])
        save = Gtk.Button(label="Save the Log", css_classes=["pill"])
        save.connect("clicked", self.save_log)
        again = Gtk.Button(label="Start Over", css_classes=["pill", "suggested-action"])
        again.connect("clicked", lambda *_: self.show("disk"))
        box.append(save)
        box.append(again)
        self.failed.set_child(box)
        return self.failed

    # Navigation

    def show(self, name):
        self.stack.set_visible_child_name(name)
        in_steps = name in self.steps
        self.toolbar.set_reveal_bottom_bars(in_steps)
        if in_steps:
            i = self.steps.index(name)
            self.header_title.set_subtitle(f"Step {i + 1} of {len(self.steps)}")
            self.back.set_visible(i > 0)
            last = name == "review"
            self.next.set_label(f"Erase {self.disk()} and Install" if last else "Continue")
            self.next.set_css_classes(["destructive-action"] if last else ["suggested-action"])
            if last:
                self.fill_review()
        else:
            self.header_title.set_subtitle("")
        self.update_next()

    def go(self, delta):
        name = self.stack.get_visible_child_name()
        i = self.steps.index(name)
        if delta > 0 and not self.disk():
            self.toast.add_toast(Adw.Toast(title="Choose the disk to install on."))
            self.show("disk")
            return
        if delta > 0 and name == "review":
            self.confirm()
            return
        self.show(self.steps[max(0, min(len(self.steps) - 1, i + delta))])

    def disk(self):
        for c in self.disk_checks:
            if c.get_active():
                return c.disk
        return None

    def account_problem(self):
        user = self.user_row.get_text().strip()
        host = self.host_row.get_text().strip()
        if not user:
            return "Choose a user name."
        if not re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", user):
            return "A user name uses lowercase letters, digits, - and _, and starts with a letter."
        if user in RESERVED_USERS:
            return f"“{user}” is reserved by the system; choose another user name."
        if not self.pw_row.get_text():
            return "Choose a password."
        if self.pw_row.get_text() != self.pw2_row.get_text():
            return "The two passwords do not match."
        if not re.fullmatch(r"[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?", host):
            return "A computer name uses letters, digits and -, up to 63 characters."
        return None

    def setup_problem(self):
        if self.encrypt_row.get_active():
            if not self.pass_row.get_text():
                return "Choose a disk passphrase."
            if self.pass_row.get_text() != self.pass2_row.get_text():
                return "The two passphrases do not match."
        return None

    def update_next(self):
        name = self.stack.get_visible_child_name()
        ok = True
        if name == "disk":
            ok = self.disk() is not None
        elif name == "account":
            problem = self.account_problem()
            touched = any(r.get_text() for r in (self.user_row, self.pw_row, self.pw2_row))
            self.account_hint.set_text(problem if (problem and touched) else "")
            ok = problem is None
        elif name == "agent":
            ok = self.agent.complete
        elif name == "setup":
            ok = self.setup_problem() is None
        self.next.set_sensitive(ok)

    def zone(self):
        return self.zones[self.zone_row.get_selected()]

    def apps(self):
        return "full" if self.full_check.get_active() else "minimal"

    def fill_review(self):
        for r in getattr(self, "review_rows", []):
            self.review_group.remove(r)
        rows = [
            ("Disk (erased)", self.disk() or ""),
            ("User", self.user_row.get_text().strip()),
            ("Computer name", self.host_row.get_text().strip()),
            ("Agent", self.agent.summary()),
            ("Time zone", self.zone()),
            ("Applications", "Everyday apps (needs the internet)" if self.apps() == "full" else "Minimal"),
            ("Disk encryption", "On" if self.encrypt_row.get_active() else "Off"),
        ]
        self.review_rows = []
        for title, value in rows:
            row = Adw.ActionRow(title=title, subtitle=value, css_classes=["property"])
            self.review_group.add(row)
            self.review_rows.append(row)

    def confirm(self):
        dialog = Adw.AlertDialog(
            heading=f"Erase {self.disk()}?",
            body="Everything on it is deleted and SlateOS is installed in its place. This cannot be undone.",
        )
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("install", "Erase and Install")
        dialog.set_response_appearance("install", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.set_close_response("cancel")
        dialog.connect("response", lambda _d, r: r == "install" and self.start())
        dialog.present(self)

    # Installing

    def start(self):
        if not self.disk():
            self.show("disk")
            return
        try:
            pw_hash = subprocess.run([MKPASSWD, "-m", "sha-512", "--stdin"], input=self.pw_row.get_text(), capture_output=True, text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError) as e:
            self.toast.add_toast(Adw.Toast(title=f"Could not hash the password: {e}"))
            return
        # As root, systemd-inhibit keeps the computer from sleeping, also when a laptop's
        # lid closes, until the install is over.
        default = (
            "sudo -n --preserve-env=SLATEOS_SRC,SLATEOS_TEMPLATE "
            f"{shlex.quote(INHIBIT)} --what=sleep:handle-lid-switch --who=SlateOS --why='SlateOS is being installed' "
            f"{shlex.quote(INSTALL)}"
        )
        cmd = shlex.split(os.environ.get("SLATEOS_INSTALL_COMMAND") or default)
        cmd += [
            "--disk", self.disk(), "--user", self.user_row.get_text().strip(), "--host", self.host_row.get_text().strip(),
            "--tz", self.zone(), "--apps", self.apps(), "--password-hash", pw_hash, "--yes",
        ]
        self.passfile = None
        if self.encrypt_row.get_active():
            fd, self.passfile = tempfile.mkstemp(prefix="slateos-pass-", dir=os.environ.get("XDG_RUNTIME_DIR") or None)
            with os.fdopen(fd, "w") as f:
                f.write(self.pass_row.get_text())
            cmd += ["--passphrase-file", self.passfile]
        # The agent's connection, as files for the new home directory: the key, or
        # Claude Code's sign-in from this session.
        self.home_files = tempfile.mkdtemp(prefix="slateos-home-", dir=os.environ.get("XDG_RUNTIME_DIR") or None)
        for rel, text in self.agent.files().items():
            slate_agent.write_private(os.path.join(self.home_files, rel), text)
        cmd += ["--home-files", self.home_files]
        if self.dry_run:
            cmd.append("--dry-run")
        self.expected_paths = self.count_store_paths()
        self.copied = 0
        self.log_lines = []
        self.log_view.get_buffer().set_text("")
        self.bar.set_fraction(0.0)
        self.stage.set_text("Starting")
        self.running = True
        self.show("progress")
        # The screen stays on while this window shows the progress: an install outlasts
        # the idle timeout, and the person comes back to see how far it got.
        self.inhibit_cookie = self.get_application().inhibit(self, Gtk.ApplicationInhibitFlags.IDLE, "SlateOS is being installed")
        try:
            self.log_file = open(LOG_FILE, "w")
        except OSError:
            self.log_file = None
        try:
            self.proc = Gio.Subprocess.new(cmd, Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_MERGE)
        except GLib.Error as e:
            self.finish(False, str(e.message))
            return
        self.out = Gio.DataInputStream.new(self.proc.get_stdout_pipe())
        self.out.read_line_async(GLib.PRIORITY_DEFAULT, None, self.on_line)
        self.proc.wait_async(None, self.on_exit)

    def count_store_paths(self):
        try:
            out = subprocess.run(["nix-store", "-q", "--requisites", "/run/current-system"], capture_output=True, text=True, timeout=30).stdout
            return max(1, out.count("\n"))
        except (OSError, subprocess.TimeoutExpired):
            return 3000

    def on_line(self, stream, result):
        try:
            line, _ = stream.read_line_finish_utf8(result)
        except GLib.Error:
            line = None
        if line is None:
            return
        self.add_log(line)
        low = line.lower()
        for marker, fraction, text in STAGES:
            if low.startswith(marker) or (not marker.startswith("==") and marker in low):
                self.bar.set_fraction(max(self.bar.get_fraction(), fraction))
                self.stage.set_text(text)
        if low.startswith("copying path"):
            self.copied += 1
            frac = 0.12 + 0.82 * min(1.0, self.copied / self.expected_paths)
            self.bar.set_fraction(max(self.bar.get_fraction(), frac))
        stream.read_line_async(GLib.PRIORITY_DEFAULT, None, self.on_line)

    def add_log(self, line):
        self.log_lines.append(line)
        if self.log_file:
            self.log_file.write(line + "\n")
            self.log_file.flush()
        buf = self.log_view.get_buffer()
        buf.insert(buf.get_end_iter(), line + "\n")
        if buf.get_line_count() > 3000:  # nixos-install is chatty; keep the tail
            _, cut = buf.get_iter_at_line(1000)
            buf.delete(buf.get_start_iter(), cut)
        mark = buf.create_mark(None, buf.get_end_iter(), False)
        self.log_view.scroll_mark_onscreen(mark)
        buf.delete_mark(mark)

    def on_exit(self, proc, result):
        try:
            proc.wait_finish(result)
        except GLib.Error:
            pass
        ok = proc.get_if_exited() and proc.get_exit_status() == 0
        last = next((ln for ln in reversed(self.log_lines) if ln.strip()), "")
        self.finish(ok, last)

    def finish(self, ok, last):
        self.running = False
        if self.inhibit_cookie:
            self.get_application().uninhibit(self.inhibit_cookie)
            self.inhibit_cookie = 0
        if self.log_file:
            self.log_file.close()
            self.log_file = None
        if self.passfile:
            try:
                os.remove(self.passfile)
            except OSError:
                pass
            self.passfile = None
        if self.home_files:
            shutil.rmtree(self.home_files, ignore_errors=True)
            self.home_files = None
        if ok:
            self.bar.set_fraction(1.0)
            self.show("done")
        else:
            self.failed.set_description(f"The last message was:\n{last}" if last else "")
            self.show("failed")

    def save_log(self, *_):
        path = os.path.expanduser("~/slateos-install.log")
        try:
            with open(path, "w") as f:
                f.write("\n".join(self.log_lines) + "\n")
            self.toast.add_toast(Adw.Toast(title=f"Saved to {path}"))
        except OSError as e:
            self.toast.add_toast(Adw.Toast(title=f"Could not save the log: {e.strerror}"))

    def on_close_request(self, *_):
        if self.running:
            self.toast.add_toast(Adw.Toast(title="SlateOS is being installed; this window closes when it is done."))
            return True
        return False


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)

    def do_startup(self):
        Adw.Application.do_startup(self)
        # Dark like the rest of the desktop, unless the system says light. At login the
        # settings portal may not answer yet, and libadwaita would then pick light.
        Adw.StyleManager.get_default().set_color_scheme(Adw.ColorScheme.PREFER_DARK)

    def do_command_line(self, command_line):
        args = command_line.get_arguments()[1:]
        win = self.props.active_window
        if win is None:
            extra = None
            if "--disk" in args and args.index("--disk") + 1 < len(args):
                extra = args[args.index("--disk") + 1]
            slate_agent.install_css()
            provider = Gtk.CssProvider()
            provider.load_from_data(CSS)
            Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
            win = Installer(self, "--welcome" in args, "--dry-run" in args, extra)
        win.present()
        return 0


if __name__ == "__main__":
    GLib.set_prgname(APP_ID)
    sys.exit(App().run(sys.argv))
