# OS Skills

An OS Skill is a machine-readable manual that tells an agent the correct, boring way to do one kind of task on this system. Skills exist so agents reach for `wpctl` instead of clicking through a settings panel, and so that every such action has a known approval tier and a known undo.

## Format (v0, subject to an RFC)

A skill is a directory:

```
skills/<name>/
  SKILL.md        # what, when, preferred path, fallbacks, verify, undo
  manifest.toml   # name, description, tiers, required tools, compositor/apps it applies to
```

`SKILL.md` is written for the agent. It should be short, concrete, and prefer commands over prose. `manifest.toml` is read by slated.

slated exposes skills to backends through their native mechanisms: as Claude Code skills, and as sections of AGENTS.md for Codex.

## Conventions

- Prefer CLI, D-Bus and config files. Say explicitly when a GUI is unavoidable.
- State the approval tier of each step. Unknown defaults to Confirm.
- Always include how to verify success and how to undo.
- One skill per task family, not per command.

See `examples/` for the shape.
