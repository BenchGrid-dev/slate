#!/usr/bin/env python3
"""slate-update-notify: tells the person, once, what software update has to say: a new
version, one that waits for its restart, one that was installed, one that did not start.

It runs when the session starts and whenever slateos-update's state file changes (a
path unit of the session). What was told is kept in ~/.local/state/slate/update-notified.
A click on a notification opens Settings, Software Update.
"""
import json
import os
import subprocess
import sys
import time

STATE = os.environ.get("SLATEOS_UPDATE_STATE", "/var/lib/slateos-update/state.json")
SEEN = os.path.join(os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"), "slate", "update-notified")


def news(state):
    """What a state has to tell: a list of (key, title, body), a key for each piece of
    news so that it is told once."""
    items = []
    phase = state.get("phase", "idle")
    target = state.get("target") or {}
    offer = state.get("available") or {}
    last = state.get("last") or {}
    if phase == "restart" and not target.get("back"):
        items.append((f"ready:{target.get('version')}", f"SlateOS {target.get('version')} is ready", "Restart to use it."))
    elif phase == "idle" and offer.get("version"):
        items.append((f"available:{offer['version']}", f"SlateOS {offer['version']} is available",
                      offer.get("summary") or "Settings, Software Update installs it."))
    key = f"{last.get('outcome')}:{last.get('version')}:{last.get('when')}"
    if last.get("outcome") == "installed":
        items.append((key, f"SlateOS was updated to {last.get('version')}", "The version before stays on this computer for a while."))
    elif last.get("outcome") == "rolled-back":
        items.append((key, f"SlateOS {last.get('version')} did not start",
                      "The version from before runs again. Settings, Software Update has the details."))
    return items


def seen():
    try:
        with open(SEEN, encoding="utf-8") as f:
            return [line.strip() for line in f if line.strip()]
    except OSError:
        return []


def remember(keys):
    os.makedirs(os.path.dirname(SEEN), exist_ok=True)
    with open(SEEN, "w", encoding="utf-8") as f:
        f.write("\n".join(keys[-50:]) + "\n")


def notify(title, body):
    """Shows the notification and waits for it; True when it could be shown."""
    for _ in range(6):
        try:
            out = subprocess.run(
                ["notify-send", "-a", "SlateOS", "-i", "slateos", "-t", "20000", "-A", "default=Software Update", title, body],
                capture_output=True, text=True, timeout=120)
        except subprocess.TimeoutExpired:
            return True
        except OSError:
            return False
        if out.returncode == 0:
            if out.stdout.strip() == "default":
                subprocess.Popen(["slate-settings", "update"])
            return True
        time.sleep(3)   # the notification service is not up yet, right after login
    return False


def main():
    try:
        with open(STATE, encoding="utf-8") as f:
            state = json.load(f)
    except (OSError, ValueError):
        return 0
    told = seen()
    for key, title, body in news(state):
        if key in told:
            continue
        if notify(title, body):
            told.append(key)
            remember(told)
    return 0


if __name__ == "__main__":
    sys.exit(main())
