#!/usr/bin/env python3
"""slate-battery: tell the person when the battery runs low.

A notification at 10 % and an urgent one at 5 % while the computer runs on battery,
once each until it charges again. Reads UPower's display device; exits on a machine
without a battery. The menu bar shows the level itself.
"""
import subprocess
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

DEVICE = "/org/freedesktop/UPower/devices/DisplayDevice"
DISCHARGING = 2
BATTERY = 2
LEVELS = ((5, "critical", "Battery critically low", "Plug in the computer now; it will sleep soon."),
          (10, "normal", "Battery low", "Plug in the computer soon."))


class Watcher:
    def __init__(self, proxy):
        self.proxy = proxy
        self.told = set()

    def prop(self, name):
        v = self.proxy.get_cached_property(name)
        return v.unpack() if v is not None else None

    def check(self, *_):
        if self.prop("State") != DISCHARGING:
            self.told.clear()
            return
        pct = self.prop("Percentage") or 100
        for level, urgency, title, body in LEVELS:
            if pct <= level and level not in self.told:
                self.told.update(lv for lv, *_ in LEVELS if lv >= level)
                minutes = (self.prop("TimeToEmpty") or 0) // 60
                left = f" About {minutes} minutes left." if minutes else ""
                subprocess.run(["notify-send", "-a", "SlateOS", "-u", urgency, "-i", "battery-caution",
                                f"{title} ({pct:.0f} %)", body + left], check=False)
                break


def main():
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        proxy = Gio.DBusProxy.new_sync(bus, Gio.DBusProxyFlags.NONE, None, "org.freedesktop.UPower",
                                       DEVICE, "org.freedesktop.UPower.Device", None)
    except GLib.Error as e:
        print(f"slate-battery: UPower: {e.message}", file=sys.stderr)
        return 0
    w = Watcher(proxy)
    if not w.prop("IsPresent") or w.prop("Type") != BATTERY:
        print("slate-battery: no battery")
        return 0
    proxy.connect("g-properties-changed", w.check)
    w.check()
    GLib.MainLoop().run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
