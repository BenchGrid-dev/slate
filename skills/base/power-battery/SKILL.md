---
name: slate-power-battery
description: Read the battery level and time left, switch the power mode (power saver, balanced, performance), lock the screen, and change when the screen locks or turns off. Use for any request about the battery, power saving, performance mode, or the screen lock.
---

# Battery, power mode and the screen lock

## Battery

```
upower -i /org/freedesktop/UPower/devices/DisplayDevice   # percentage, state, time to empty / full
```

Tier: observe. `power supply: no` or no `percentage`: the computer has no battery.

## Power mode

```
powerprofilesctl list          # the modes this computer offers (performance needs hardware support)
powerprofilesctl get
powerprofilesctl set power-saver    # or balanced, performance
```

Tier: reversible. Undo: set the previous mode, read first with `get`.

## Lock the screen

```
slate-lock
```

Tier: reversible (the person unlocks it with their password). While the screen is locked, the desktop tools refuse keyboard and pointer input: reading and element actions still work. Never type into the lock screen.

## When the screen locks and turns off

`~/.config/slate/idle.json` holds `{"lock_after": SECONDS, "screen_off_after": SECONDS}` (0: never; defaults 600 and 900). Counted from the person's last input; what the agent does does not count. After writing it: `systemctl --user restart slate-idle`. The screen also locks before the computer sleeps. Settings, Power shows the same.

Tier: reversible. Undo: write the previous values back.

## Verify

`powerprofilesctl get` prints the new mode; `systemctl --user status slate-idle` shows the swayidle command line with the new timeouts.
