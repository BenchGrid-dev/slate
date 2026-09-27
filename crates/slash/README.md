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
shell_interactive = false     # true: run ! lines with -i so aliases/functions load (slower)
context_commands = 20         # manual commands kept as agent context

[claude]
bin = "claude"
model = "sonnet"                  # alias or full id; /model switches at runtime
permission_mode = "acceptEdits"   # headless Claude Code has no prompt; "default" denies most tools
allowed_tools = []                # e.g. ["Bash(git:*)", "Read"]
extra_args = []

[codex]
bin = "codex"
# model = "..."                   # default: Codex's own
sandbox = "workspace-write"
extra_args = []
```

## How `!` works

Each `!` line runs in your shell inside a pty. slash forwards your keystrokes and tees the output to the screen and to a buffer, so vim, sudo and less work, and the agent can later see what the command printed (last 30 lines per command, cleaned of escape codes, within a total budget). Working directory and exported environment variables persist across lines. Aliases and functions from your rc files do not, unless `shell_interactive = true`.

## What it does not do yet

- Talk to `slated`: approvals, snapshots, `/undo`, memory. Those land with `slated`.
- Job control (`Ctrl-Z`) inside `!` commands.
- Persist unexported shell variables, aliases or functions across `!` lines.
