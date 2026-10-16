#!/usr/bin/env python3
"""slate-theme: the SlateOS appearance switch.

    slate-theme dark | light        switch the whole desktop
    slate-theme wallpaper PATH      set the desktop background (any image; "default" resets)
    slate-theme status              what is set
    slate-theme apply               re-apply the saved choices (used at login)

What a theme touches: GTK and libadwaita apps (the color-scheme and gtk-theme keys the
settings portal serves), sway's window colours and background, waybar's style, foot's
colours (foot follows the portal on its own), mako and fuzzel. Everything is written to
the user's own config so it survives logins and never touches /etc.
"""
import json
import os
import subprocess
import sys

HOME = os.path.expanduser("~")
STATE = os.path.join(HOME, ".config/slate/theme.json")
SWAY_D = os.path.join(HOME, ".config/slate/sway.d/theme.conf")
WAYBAR_USER = os.path.join(HOME, ".config/waybar")
MAKO_USER = os.path.join(HOME, ".config/mako/config")
FUZZEL_USER = os.path.join(HOME, ".config/fuzzel/fuzzel.ini")
FOOT_USER = os.path.join(HOME, ".config/foot/foot.ini")
SYSTEM_WAYBAR = "/etc/xdg/waybar"
# "default" follows the theme; the old single wallpaper path means the same.
DEFAULT_WALLPAPER = "default"
LEGACY_DEFAULTS = ("/etc/slate/wallpaper.png",)
THEME_WALLPAPER = "/etc/slate/wallpaper-{theme}.jpg"

THEMES = {
    "dark": {
        "color_scheme": "prefer-dark", "gtk": "adw-gtk3-dark", "icons": "Colloid-Dark",
        # Title bars match adw-gtk3's dark header; focus shows in the title's colour.
        "sway": [
            "client.focused #303030 #303030 #f2f2f7 #303030 #ffffff1f",
            "client.focused_inactive #2a2a2a #2a2a2a #8e8e93 #2a2a2a #ffffff12",
            "client.unfocused #2a2a2a #2a2a2a #8e8e93 #2a2a2a #ffffff12",
            "client.urgent #e5a35a #e5a35a #13151b #e5a35a #e5a35a",
        ],
        "waybar_css": "style.css",
        "mako": {"background-color": "#1f1f23d9", "text-color": "#f2f2f7", "border-color": "#ffffff1f", "progress-color": "over #7c9cf5"},
        "fuzzel": {"background": "1f1f23e0", "text": "f2f2f7ff", "placeholder": "8e8e93ff", "prompt": "8fa7f0ff", "input": "f2f2f7ff",
                   "match": "aec2ffff", "selection": "7c9cf540", "selection-text": "ffffffff", "selection-match": "d4dfffff", "border": "ffffff1f"},
    },
    "light": {
        "color_scheme": "prefer-light", "gtk": "adw-gtk3", "icons": "Colloid",
        "sway": [
            "client.focused #ebebeb #ebebeb #1d1d1f #ebebeb #0000001f",
            "client.focused_inactive #f2f2f2 #f2f2f2 #8e8e93 #f2f2f2 #00000014",
            "client.unfocused #f2f2f2 #f2f2f2 #8e8e93 #f2f2f2 #00000014",
            "client.urgent #e5a35a #e5a35a #13151b #e5a35a #e5a35a",
        ],
        "waybar_css": "style-light.css",
        "mako": {"background-color": "#f7f7f9e6", "text-color": "#1d1d1f", "border-color": "#0000001a", "progress-color": "over #3b6de8"},
        "fuzzel": {"background": "f7f7f9eb", "text": "1d1d1fff", "placeholder": "8e8e93ff", "prompt": "3b6de8ff", "input": "1d1d1fff",
                   "match": "2f5bd0ff", "selection": "3b6de826", "selection-text": "1d1d1fff", "selection-match": "2f5bd0ff", "border": "0000001a"},
    },
}


def wallpaper_for(state):
    """The image to show: the person's, or the theme's own when they chose the default."""
    w = state.get("wallpaper") or DEFAULT_WALLPAPER
    if w == DEFAULT_WALLPAPER or w in LEGACY_DEFAULTS or not os.path.exists(w):
        return THEME_WALLPAPER.format(theme=state.get("theme", "dark"))
    return w


def load():
    try:
        with open(STATE) as f:
            return json.load(f)
    except Exception:  # noqa: BLE001
        return {"theme": "dark", "wallpaper": DEFAULT_WALLPAPER}


def save(state):
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    with open(STATE, "w") as f:
        json.dump(state, f, indent=2)


def run(cmd, check=False):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=10, check=check)
    except Exception as e:  # noqa: BLE001
        print(f"warning: {' '.join(cmd)}: {e}", file=sys.stderr)
        return None


def swaymsg(command):
    return run(["swaymsg", command])


def signal_processes(names, sig):
    """Send `sig` to every process whose comm is one of `names` (NixOS wraps binaries as
    .name-wrapped, so plain pkill -x misses them)."""
    import signal as _signal  # noqa: F401
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            comm = open(f"/proc/{pid}/comm").read().strip()
        except OSError:
            continue
        if comm in names:
            try:
                os.kill(int(pid), sig)
            except OSError:
                pass


def apply(state):
    t = THEMES[state["theme"]]
    wallpaper = wallpaper_for(state)

    # GTK3 reads gsettings; GTK4/libadwaita, foot and others read the portal, which reads the same keys.
    for key, val in (("color-scheme", t["color_scheme"]), ("gtk-theme", t["gtk"]), ("icon-theme", t["icons"])):
        run(["dconf", "write", f"/org/gnome/desktop/interface/{key}", f"'{val}'"])

    # sway: window colours and background, live now and in the user's config for next time.
    os.makedirs(os.path.dirname(SWAY_D), exist_ok=True)
    lines = ["# written by slate-theme; do not edit, use `slate-theme`", *t["sway"], f'output * bg "{wallpaper}" fill']
    with open(SWAY_D, "w") as f:
        f.write("\n".join(lines) + "\n")
    for line in lines[1:]:
        swaymsg(line)

    # waybar: the user's style overrides the system one; it imports the system file of
    # the theme so system updates keep applying. SIGUSR2 reloads it.
    os.makedirs(WAYBAR_USER, exist_ok=True)
    src = os.path.join(SYSTEM_WAYBAR, t["waybar_css"])
    if os.path.exists(src):
        with open(os.path.join(WAYBAR_USER, "style.css"), "w") as f:
            f.write(f'/* written by slate-theme */\n@import url("file://{src}");\n')
        import signal
        signal_processes(("waybar", ".waybar-wrapped"), signal.SIGUSR2)

    # foot: new terminals start with the theme (initial-color-theme); running ones are
    # told to switch (foot: SIGUSR1 = dark, SIGUSR2 = light).
    os.makedirs(os.path.dirname(FOOT_USER), exist_ok=True)
    with open(FOOT_USER, "w") as f:
        f.write("# written by slate-theme; the system defaults live in /etc/xdg/foot/foot.ini\n")
        f.write("include=/etc/xdg/foot/foot.ini\n\n[main]\n")
        f.write(f"initial-color-theme={state['theme']}\n")
    import signal as _sig
    signal_processes(("foot", ".foot-wrapped", "footclient"), _sig.SIGUSR1 if state["theme"] == "dark" else _sig.SIGUSR2)

    # mako: rewrite the user's config from the system one with the theme's colours.
    base = "/etc/xdg/mako/config"
    if os.path.exists(base):
        os.makedirs(os.path.dirname(MAKO_USER), exist_ok=True)
        out = []
        for line in open(base):
            key = line.split("=", 1)[0].strip()
            if key in t["mako"] and not line.startswith("["):
                out.append(f"{key}={t['mako'][key]}\n")
            else:
                out.append(line)
        with open(MAKO_USER, "w") as f:
            f.writelines(out)
        run(["makoctl", "reload"])

    # fuzzel: same, colours section.
    base = "/etc/xdg/fuzzel/fuzzel.ini"
    if os.path.exists(base):
        os.makedirs(os.path.dirname(FUZZEL_USER), exist_ok=True)
        out, section = [], ""
        for line in open(base):
            if line.startswith("["):
                section = line.strip()
            key = line.split("=", 1)[0].strip()
            if section == "[colors]" and key in t["fuzzel"]:
                out.append(f"{key}={t['fuzzel'][key]}\n")
            elif section == "[main]" and key == "icon-theme":
                out.append(f"icon-theme={t['icons']}\n")
            else:
                out.append(line)
        with open(FUZZEL_USER, "w") as f:
            f.writelines(out)


def main():
    args = sys.argv[1:]
    state = load()
    if not args or args[0] == "status":
        print(json.dumps(state))
        return
    cmd = args[0]
    if cmd in ("dark", "light"):
        state["theme"] = cmd
    elif cmd == "wallpaper":
        if len(args) < 2:
            print("usage: slate-theme wallpaper PATH|default", file=sys.stderr)
            sys.exit(2)
        path = DEFAULT_WALLPAPER if args[1] == "default" else os.path.abspath(os.path.expanduser(args[1]))
        if path != DEFAULT_WALLPAPER and not os.path.isfile(path):
            print(f"no such image: {path}", file=sys.stderr)
            sys.exit(1)
        state["wallpaper"] = path
    elif cmd == "apply":
        pass
    else:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    save(state)
    apply(state)
    print(json.dumps(state))


if __name__ == "__main__":
    main()
