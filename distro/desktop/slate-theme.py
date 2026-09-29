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
DEFAULT_WALLPAPER = "/etc/slate/wallpaper.png"

THEMES = {
    "dark": {
        "color_scheme": "prefer-dark", "gtk": "Adwaita-dark", "icons": "Papirus-Dark",
        "sway": [
            "client.focused #2a2e3a #2a2e3a #e8eaf0 #7c9cf5 #3a3f4e",
            "client.focused_inactive #1c1f27 #1c1f27 #9ba1b0 #1c1f27 #262a36",
            "client.unfocused #1c1f27 #1c1f27 #6b7080 #1c1f27 #262a36",
            "client.urgent #e5a35a #e5a35a #13151b #e5a35a #e5a35a",
        ],
        "waybar_css": "style.css",
        "mako": {"background-color": "#161820f5", "text-color": "#d7dae2", "border-color": "#ffffff1a", "progress-color": "over #7c9cf5"},
        "fuzzel": {"background": "161820f5", "text": "d7dae2ff", "placeholder": "6b7080ff", "prompt": "8fa7f0ff", "input": "e8eaf0ff",
                   "match": "8fa7f0ff", "selection": "262a36ff", "selection-text": "ffffffff", "selection-match": "8fa7f0ff", "border": "ffffff1a"},
    },
    "light": {
        "color_scheme": "prefer-light", "gtk": "Adwaita", "icons": "Papirus",
        "sway": [
            "client.focused #d5d9e3 #d5d9e3 #1c1f27 #3b6de8 #c2c8d6",
            "client.focused_inactive #eceef3 #eceef3 #6b7080 #eceef3 #dfe2ea",
            "client.unfocused #eceef3 #eceef3 #8a90a0 #eceef3 #dfe2ea",
            "client.urgent #e5a35a #e5a35a #13151b #e5a35a #e5a35a",
        ],
        "waybar_css": "style-light.css",
        "mako": {"background-color": "#ffffffee", "text-color": "#1c1f27", "border-color": "#0000001f", "progress-color": "over #3b6de8"},
        "fuzzel": {"background": "f7f8fbf5", "text": "1c1f27ff", "placeholder": "8a90a0ff", "prompt": "3b6de8ff", "input": "1c1f27ff",
                   "match": "3b6de8ff", "selection": "dfe4f0ff", "selection-text": "1c1f27ff", "selection-match": "3b6de8ff", "border": "0000001f"},
    },
}


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
    wallpaper = state.get("wallpaper") or DEFAULT_WALLPAPER
    if not os.path.exists(wallpaper):
        wallpaper = DEFAULT_WALLPAPER

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
        if not os.path.isfile(path):
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
