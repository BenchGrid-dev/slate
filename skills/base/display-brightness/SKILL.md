---
name: slate-display-brightness
description: Read or set screen brightness with brightnessctl. Use for any request about the screen being too bright or too dim.
---

# Display brightness

Use `brightnessctl`. Do not open a settings GUI for this.

```
brightnessctl get                 # current value
brightnessctl max                 # maximum
brightnessctl set 40%             # absolute
brightnessctl set 10%+            # relative
brightnessctl set 10%-
```

Tier: reversible. Undo: `brightnessctl set <previous>` (read it first with `get`).

## Verify

`brightnessctl get` prints the new value. On a VM or headless system there may be no backlight device; say so instead of trying GUI paths.
