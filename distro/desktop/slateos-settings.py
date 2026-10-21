#!/usr/bin/env python3
"""slateos-settings: the system-wide choices Settings makes, applied with a rebuild.

    slateos-settings show                        what the running system has and offers
    slateos-settings set input-methods NAME,...  install these input methods ("none": no input method)

The choices live in settings.json next to the system configuration (/etc/slateos, or
/etc/nixos where there is no /etc/slateos); the SlateOS module reads them
(services.slate.settingsFile), and the configuration itself wins where it sets the
same option. `set` runs as root: it writes the file, rebuilds the system, and puts
the file back the way it was if the rebuild fails.
"""
import json
import os
import subprocess
import sys

# What the running system offers and has, written by the module.
SYSTEM = "/etc/slate/system-settings.json"
REBUILD = "/run/current-system/sw/bin/slateos-rebuild"


def settings_path():
    base = "/etc/slateos" if os.path.isdir("/etc/slateos") else "/etc/nixos"
    return os.path.join(base, "settings.json")


def system():
    try:
        with open(SYSTEM, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def fail(msg, code=1):
    print(f"slateos-settings: {msg}", file=sys.stderr)
    sys.exit(code)


def set_input_methods(value):
    offered = system().get("inputMethods", {}).get("available", [])
    names = [] if value == "none" else [n for n in value.split(",") if n]
    unknown = [n for n in names if n not in offered]
    if unknown:
        fail(f"unknown input method {', '.join(unknown)} (this system offers {', '.join(offered) or 'none'})", 2)
    if len(set(names)) != len(names):
        fail("an input method is named twice", 2)

    path = settings_path()
    try:
        with open(path, encoding="utf-8") as f:
            before = f.read()
    except FileNotFoundError:
        before = None
    try:
        current = json.loads(before) if before else {}
    except ValueError:
        fail(f"{path} is not valid JSON; fix or remove it first")
    current["inputMethods"] = names
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(current, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)

    print(f"Updating the system ({path}: input methods {', '.join(names) or 'none'})", flush=True)
    if subprocess.run([REBUILD, "switch"]).returncode != 0:
        if before is None:
            os.remove(path)
        else:
            with open(tmp, "w", encoding="utf-8") as f:
                f.write(before)
            os.replace(tmp, path)
        fail("the rebuild failed; the settings are as they were")

    enabled = system().get("inputMethods", {}).get("enabled")
    if enabled != names:
        fail(f"the system has input methods {', '.join(enabled or []) or 'none'}: the configuration sets "
             "services.slate.desktop.inputMethods itself, or does not read settings.json "
             "(services.slate.settingsFile)", 3)
    print("Done.")


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__.strip())
        return
    if args[0] == "show":
        print(json.dumps({"file": settings_path(), **system()}, indent=2))
        return
    if args[0] == "set" and len(args) == 3 and args[1] == "input-methods":
        if os.geteuid() != 0:
            fail("needs root: sudo slateos-settings set ...")
        set_input_methods(args[2])
        return
    fail(f"unknown command {' '.join(args)} (see --help)", 2)


if __name__ == "__main__":
    main()
