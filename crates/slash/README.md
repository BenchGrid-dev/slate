# slash

The Slate shell. See the top-level README for what it is for.

## Status

v0. Works as a standalone program on macOS and Linux with Claude Code or Codex installed. Not yet safe as a login shell in daily use (no `slated`, so permissions are whatever `--permission-mode` you configure).

## Run

```
cargo run -p slash
```

```
~/proj ❯ what's eating disk space in here?
  ▸ Bash: du -sh * | sort -h | tail
The target/ directory is 4.1G; everything else is under 50M.
✓ 6.2s, 2 turns

~/proj ❯ !cargo clean
~/proj ❯ /agent codex
~/proj ❯ //compact          # send a literal "/compact" to the agent
```

## Config

`~/.config/slate/slash.toml`, all keys optional:

```toml
backend = "claude"            # or "codex"
fallback_shell = "/bin/zsh"   # default: $SHELL unless that is slash
context_commands = 20         # manual commands kept as agent context

[claude]
bin = "claude"
permission_mode = "acceptEdits"   # headless Claude Code has no prompt; "default" denies most tools
allowed_tools = []                # e.g. ["Bash(git:*)", "Read"]
extra_args = []

[codex]
bin = "codex"
sandbox = "workspace-write"
extra_args = []
```

## What it does not do yet

- Capture `!` output for the agent (commands inherit the terminal, so interactive programs work; the trade-off is no capture). Planned: pty tee.
- Persist environment variables across `!` lines. cwd persists.
- Talk to `slated`: approvals, snapshots, `/undo`, memory. Those land with `slated`.
- Stream partial tokens. Text appears per assistant message.
