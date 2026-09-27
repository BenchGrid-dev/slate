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

`slate skills install` links every skill under `skills/base` (or a directory you pass) into `~/.claude/skills/slate-<name>`, where Claude Code picks them up as user skills. Codex reads instructions from `AGENTS.md`; a generated section for it is planned.

## Conventions

- Prefer CLI, D-Bus and config files. Say explicitly when a GUI is unavoidable.
- State the approval tier of each step. Unknown defaults to Confirm.
- Always include how to verify success and how to undo.
- One skill per task family, not per command.
- Never include credentials or guess them; say when the user has to supply something.
