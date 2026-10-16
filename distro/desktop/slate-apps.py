#!/usr/bin/env python3
"""slate-apps: tidy the application list the launcher and the dock show.

Hides developer tools, terminal programs and the NixOS manual, and gives a few
applications plain names, by writing overrides into ~/.local/share/applications.
Every file it writes carries X-Slate-Apps=true; files without it (the person's own)
are never touched, and overrides that no longer apply are removed. The slate-apps
user service runs it at login and after every upgrade.

    slate-apps           apply
    slate-apps --list    show what it would hide and rename
"""
import configparser
import getpass
import os
import sys

HOME = os.path.expanduser("~")
DATA_HOME = os.environ.get("XDG_DATA_HOME") or os.path.join(HOME, ".local/share")
OUT = os.path.join(DATA_HOME, "applications")
MARK = "X-Slate-Apps"

HIDE = {
    "footclient.desktop", "foot-server.desktop", "nixos-manual.desktop",
    "thunar-bulk-rename.desktop", "thunar-settings.desktop",
    "org.gtk.Demo4.desktop", "org.gtk.gtk4.NodeEditor.desktop", "org.gtk.PrintEditor4.desktop",
    "org.gtk.Shaper.desktop", "org.gtk.WidgetFactory4.desktop",
    "gtk3-demo.desktop", "gtk3-widget-factory.desktop", "gtk3-icon-browser.desktop",
    "base.desktop", "math.desktop", "xterm.desktop", "cups.desktop",
}
RENAME = {
    "thunar.desktop": "Files",
    "foot.desktop": "Terminal",
    "org.pwmt.zathura.desktop": "Document Viewer",
    "mpv.desktop": "Videos",
    "org.xfce.mousepad.desktop": "Text Editor",
}


def data_dirs():
    # XDG_DATA_DIRS as the session has it, then NixOS's own places: a user service does
    # not always get the session's variable.
    user = getpass.getuser()
    dirs = (os.environ.get("XDG_DATA_DIRS", "").split(":")
            + [os.path.join(HOME, ".nix-profile/share"), f"/etc/profiles/per-user/{user}/share",
               "/run/current-system/sw/share", "/usr/local/share", "/usr/share"])
    seen = []
    for d in dirs:
        d = os.path.join(d, "applications")
        if d and os.path.isdir(d) and os.path.realpath(d) != os.path.realpath(OUT) and d not in seen:
            seen.append(d)
    return seen


def read(path):
    cp = configparser.RawConfigParser(strict=False, interpolation=None)
    cp.optionxform = str  # keys are case-sensitive
    try:
        cp.read(path, encoding="utf-8")
    except (configparser.Error, UnicodeDecodeError):
        return None
    return cp if cp.has_section("Desktop Entry") else None


def system_entries():
    """The first definition of every desktop id along XDG_DATA_DIRS."""
    found = {}
    for d in data_dirs():
        for name in os.listdir(d):
            if name.endswith(".desktop") and name not in found:
                found[name] = os.path.join(d, name)
    return found


def plan(entries):
    hide, rename = {}, {}
    names_in_use = set()
    for did, path in entries.items():
        cp = read(path)
        if cp is None:
            continue
        e = cp["Desktop Entry"]
        if e.get("NoDisplay", "false").lower() == "true" or e.get("Hidden", "false").lower() == "true":
            continue
        if did in HIDE or e.get("Terminal", "false").lower() == "true":
            hide[did] = path
        else:
            names_in_use.add(e.get("Name", ""))
    for did, name in RENAME.items():
        if did in entries and did not in hide and name not in names_in_use:
            rename[did] = (entries[did], name)
            names_in_use.add(name)
    return hide, rename


def ours(path):
    cp = read(path)
    return cp is not None and cp["Desktop Entry"].get(MARK, "") == "true"


def write(did, src, name=None, hidden=False):
    dst = os.path.join(OUT, did)
    if os.path.exists(dst) and not ours(dst):
        return False  # the person's own override
    lines, in_entry, done = [], False, set()
    for line in open(src, encoding="utf-8", errors="replace"):
        stripped = line.strip()
        if stripped.startswith("["):
            if in_entry:
                lines.extend(extra(name, hidden, done))
            in_entry = stripped == "[Desktop Entry]"
            lines.append(line)
            continue
        key = stripped.split("=", 1)[0]
        if in_entry and name and key == "Name":
            lines.append(f"Name={name}\n")
            done.add("Name")
            continue
        if in_entry and hidden and key == "NoDisplay":
            continue
        if in_entry and key == MARK:
            continue
        lines.append(line if line.endswith("\n") else line + "\n")
    if in_entry:
        lines.extend(extra(name, hidden, done))
    os.makedirs(OUT, exist_ok=True)
    tmp = dst + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.writelines(lines)
    os.replace(tmp, dst)
    return True


def extra(name, hidden, done):
    out = []
    if hidden:
        out.append("NoDisplay=true\n")
    if name and "Name" not in done:
        out.append(f"Name={name}\n")
    out.append(f"{MARK}=true\n")
    return out


def main():
    entries = system_entries()
    hide, rename = plan(entries)
    if "--list" in sys.argv:
        for did in sorted(hide):
            print("hide  ", did)
        for did, (_, name) in sorted(rename.items()):
            print("rename", did, "->", name)
        return
    wanted = set(hide) | set(rename)
    for did, path in hide.items():
        write(did, path, hidden=True)
    for did, (path, name) in rename.items():
        write(did, path, name=name)
    # Overrides of ours that no longer apply (the app is gone, or the rule changed).
    if os.path.isdir(OUT):
        for name in os.listdir(OUT):
            p = os.path.join(OUT, name)
            if name.endswith(".desktop") and name not in wanted and ours(p):
                os.remove(p)
    print(f"slate-apps: {len(hide)} hidden, {len(rename)} renamed")


if __name__ == "__main__":
    main()
