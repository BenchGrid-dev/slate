# OS Skills

An OS Skill is a machine-readable manual that tells an agent the correct, boring way to do one kind of task on this system. Skills exist so agents reach for `wpctl` instead of clicking through a settings panel, and so that every such action has a known approval tier and a known undo.

## Format (v0, subject to an RFC)

A skill is a directory:

```
skills/base/<name>/
  SKILL.md        # what, when, preferred path, fallbacks, verify, undo — written for the agent
  manifest.toml   # name, description, tiers, required tools, what it applies to — read by slated
```

## Installing them for the agent

`slate skills install` links every skill under `skills/base` (or a directory you pass) into `~/.claude/skills/slate-<name>`, where Claude Code picks them up as user skills. Skills whose `requires` tools are missing, or whose `applies_to.distro` does not match `/etc/os-release` (`ID` or `ID_LIKE`; SlateOS reports `slateos` and `nixos`), are skipped and listed. Codex reads instructions from `AGENTS.md`; a generated section for it is planned.

## The base set

| Skill | What it teaches |
|---|---|
| `audio-volume` | wpctl for volume and mute |
| `display-brightness` | brightnessctl, and when the display has no backlight |
| `display-settings` | Query, one change at a time, verify by screenshot, roll back; persist only to `~/.config/slate/sway.d/` |
| `desktop-windows` | Arrange, focus, move and close windows through the desktop tools |
| `network-wifi` | nmcli for Wi-Fi and connections |
| `systemd-services` | systemctl for user and system units |
| `slateos-system` | Install packages and change settings by editing the NixOS configuration; rebuild and roll back |
| `undo` | When and how to offer `/undo` |

## Conventions

- Prefer CLI, D-Bus and config files. Say explicitly when a GUI is unavoidable.
- State the approval tier of each step. Unknown defaults to Confirm.
- Always include how to verify success and how to undo.
- One skill per task family, not per command.
- Never include credentials or guess them; say when the user has to supply something.
