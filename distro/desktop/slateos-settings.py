#!/usr/bin/env python3
"""slateos-settings: the system-wide choices Settings makes.

    slateos-settings show                        what the running system has and offers
    slateos-settings get KEY                     the value in effect (agent-while-locked, lock-while-agent-works)
    slateos-settings set input-methods NAME,...  install these input methods ("none": no input method)
    slateos-settings set agent-while-locked pause|elements|continue
                                                 what the agent may do in windows while the screen is locked
    slateos-settings set lock-while-agent-works yes|no
                                                 no: the idle lock waits until the agent stops using the screen

The choices live in settings.json next to the system configuration (/etc/slateos, or
/etc/nixos where there is no /etc/slateos), and the configuration itself wins where it
sets the same option. Input methods are built into the system (services.slate.settingsFile):
`set` rebuilds it, and puts the file back the way it was if the rebuild fails. The
agent's choices are read while the session runs and take effect at once. `set` runs as
root, so only someone with the password changes what the agent may do.
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


def read_settings(path):
    """settings.json as text (None if absent) and as a dict."""
    try:
        with open(path, encoding="utf-8") as f:
            before = f.read()
    except FileNotFoundError:
        return None, {}
    try:
        return before, (json.loads(before) if before.strip() else {})
    except ValueError:
        fail(f"{path} is not valid JSON; fix or remove it first")


def write_settings(path, current):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(current, f, indent=2)
        f.write("\n")
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)


# The agent's choices: command-line name -> (settings.json key, system-settings.json key
# under "agent", the values, the default). Read at run time; no rebuild.
AGENT = {
    "agent-while-locked": ("agentWhileLocked", "whileLocked", ["pause", "elements", "continue"], "pause"),
    "lock-while-agent-works": ("lockWhileAgentWorks", "lockWhileWorking", ["yes", "no"], "yes"),
}


def to_json(name, value):
    return value == "yes" if name == "lock-while-agent-works" else value


def from_json(name, value):
    if name == "lock-while-agent-works":
        return None if value is None else ("yes" if value else "no")
    return value


def agent_get(name):
    """The value in effect and whether the configuration fixes it."""
    key, system_key, values, default = AGENT[name]
    fixed = from_json(name, system().get("agent", {}).get(system_key))
    if fixed in values:
        return fixed, True
    _, current = read_settings(settings_path())
    value = from_json(name, current.get(key))
    return (value if value in values else default), False


def agent_set(name, value):
    key, system_key, values, _ = AGENT[name]
    offered = system().get("agent", {}).get("whileLockedChoices", values) if name == "agent-while-locked" else values
    if value not in offered:
        fail(f"{name} takes {'|'.join(offered)} on this system", 2)
    if agent_get(name)[1]:
        fail(f"the configuration sets this itself (services.slate.desktop.{'agentWhileLocked' if name == 'agent-while-locked' else 'lockWhileAgentWorks'})", 3)
    path = settings_path()
    _, current = read_settings(path)
    current[key] = to_json(name, value)
    write_settings(path, current)
    print(f"{path}: {name} {value}")


def set_input_methods(value):
    offered = system().get("inputMethods", {}).get("available", [])
    names = [] if value == "none" else [n for n in value.split(",") if n]
    unknown = [n for n in names if n not in offered]
    if unknown:
        fail(f"unknown input method {', '.join(unknown)} (this system offers {', '.join(offered) or 'none'})", 2)
    if len(set(names)) != len(names):
        fail("an input method is named twice", 2)

    path = settings_path()
    before, current = read_settings(path)
    current["inputMethods"] = names
    write_settings(path, current)

    print(f"Updating the system ({path}: input methods {', '.join(names) or 'none'})", flush=True)
    if subprocess.run([REBUILD, "switch"]).returncode != 0:
        if before is None:
            os.remove(path)
        else:
            tmp = path + ".tmp"
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
        agent = {name: agent_get(name)[0] for name in AGENT}
        print(json.dumps({"file": settings_path(), **system(), "effective": agent}, indent=2))
        return
    if args[0] == "get" and len(args) == 2 and args[1] in AGENT:
        print(agent_get(args[1])[0])
        return
    if args[0] == "set" and len(args) == 3 and (args[1] == "input-methods" or args[1] in AGENT):
        if os.geteuid() != 0:
            fail("needs root: sudo slateos-settings set ...")
        if args[1] == "input-methods":
            set_input_methods(args[2])
        else:
            agent_set(args[1], args[2])
        return
    fail(f"unknown command {' '.join(args)} (see --help)", 2)


if __name__ == "__main__":
    main()
