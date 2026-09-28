---
name: slate-display-settings
description: Change screen resolution, scale / HiDPI, or output layout on the Slate desktop (sway). Use for any request about resolution, display size, HiDPI, retina, blurry or tiny text, or monitors. Includes how to verify and roll back.
---

# Display settings (sway)

A wrong output setting makes the whole desktop unusable, so: query first, apply once, verify with a screenshot, keep a way back.

## 1. Query (tier: observe)

```
swaymsg -t get_outputs
```

Note each output's `name` (e.g. `Virtual-1`), `current_mode`, `scale`, and the `modes` list.

## 2. Apply (tier: reversible)

Everything is one command, applied live:

```
swaymsg 'output <NAME> mode <W>x<H> scale <S>'
```

Quote the whole command as one argument, otherwise swaymsg parses `--custom` itself and fails.

Rules:
- **Scale changes the logical size.** `scale 2` on a 1280x800 output leaves 640x400 logical pixels: everything becomes huge and windows no longer fit. That is almost never what the user wants.
- **HiDPI / "retina" / "crisper":** keep the logical size and double the physical one: `swaymsg 'output <NAME> mode --custom 2560x1600 scale 2'` (a custom mode; the VM or monitor must accept it). In a UTM VM, the user must also enable "Retina Mode" in the VM's display settings.
- **Bigger text only:** prefer a fractional scale like `scale 1.25` over a lower resolution.
- Never apply more than one change before verifying.

## 3. Verify (mandatory)

Take a full-screen screenshot with `desktop_screenshot` (no window). Check that the panel spans the top edge, windows fit, text is readable, and nothing is cut off. If it looks wrong, roll back immediately.

## 4. Roll back

```
swaymsg 'output <NAME> scale 1 mode <previous WxH>'
```

Say what you rolled back to.

## 5. Persist

Only after the user confirms it looks right. Write the same `output ...` line to `~/.config/slate/sway.d/output.conf` (create the directory; the Slate sway profile includes `~/.config/slate/sway.d/*.conf`). Do not edit `/etc/sway/config` or `/etc/nixos`. To undo persistence, delete that file.
