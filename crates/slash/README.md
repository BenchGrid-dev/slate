# slash

The Slate shell. See the top-level README for what it is for.

## Status

Works, and is the login shell on the SlateOS dev machine. Runs on Linux and macOS with Claude Code or Codex installed; on Linux it uses `slated` for approvals, audit, undo and memories, and `slate-desktop` for the GUI when a Wayland display is present. Without `slated` (macOS, or the daemon off), permissions are whatever `--permission-mode` you configure.

`slash --serve` speaks a JSON-lines protocol on stdin/stdout (`prompt`, `approve`, `command`, `quit` in; `ready`, `turn_start`, `text_delta`, `text`, `thinking`, `tool_start`, `tool_end`, `approval_needed`, `approval_resolved`, `note`, `done`, `error` out). The desktop's Slate prompt is a client of it.

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
auto_approve = false          # true: start every session as /auto on (audited, undoable)
verbose = false               # true: start in /verbose mode

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

## Approvals, audit and undo (with slated)

When `slated` is reachable (slash starts it if needed), every agent turn is a task:

- Claude Code runs with `--permission-mode default`; a PreToolUse hook asks slated for the tier of each call. Observe and Reversible calls run without asking; the first Reversible call takes a btrfs snapshot.
- Confirm-tier calls go to the permission tool, and slash asks you at the terminal: `[y]` once, `[a]` always for this task, `[n]` deny.
- `/undo` rolls back what the last task changed in the directories it worked in; `/undo --preview` shows the plan first. Needs the snapshot root (your home, or `SLATE_SNAPSHOT_ROOT`) to be a btrfs subvolume you own.
- `/audit` and `/tasks` show what happened.
- `/remember <text>` stores a memory; the agent also has `remember` / `recall` / `forget` tools. Memories are shown to the agent at the start of every task.

Codex runs inside its own sandbox; slated audits its tasks but cannot yet approve individual calls.

The settings app's AI page writes this file (backend, models, verbose, auto_approve).

## What it does not do yet

- Job control (`Ctrl-Z`) inside `!` commands.
- Persist unexported shell variables, aliases or functions across `!` lines.
- Conversation history across prompt sessions.
