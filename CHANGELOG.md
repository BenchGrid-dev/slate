# Changelog

Small releases, often. `0.0.x` are development snapshots; `0.1.0` comes after a full manual pass by the maintainer.

## 0.0.2 — 2026-09-27

Everything since the initial skeleton. Verified end to end on NixOS 26.05 / sway 1.12 with both Claude Code and Codex.

### slash
- Natural-language shell: bare text to the agent, `/` control commands, `!` runs in your real shell, `//` escapes a leading slash.
- `!` commands run in a pty: interactive programs work, output is captured for the agent, cwd and exported env persist.
- Claude Code backend (stream-json, session resume, streaming text) and Codex backend (`exec --json`, thread resume).
- Per-turn context: commands run since the previous turn (with output) and new memories travel inside the user message; static instructions in the system prompt.
- Default model `sonnet`; `/model`, `/agent`, `/new`, `/undo`, `/audit`, `/tasks`, `/remember`, `/memories`, `/context`, `/history`, `/cd`.
- Login-shell safe: non-interactive invocations exec the fallback shell; fallback resolution never picks a path that does not exist.

### slated
- Approval broker for Claude Code's permission tool; approvals answered at the terminal (`y` once, `a` for the task, `n` deny).
- Tier policy (Observe / Reversible / Confirm) with a shell classifier and `~/.config/slate/policy.toml` overrides.
- Append-only audit log; task store.
- Privilege-free btrfs snapshots per task, diff-based `undo` with preview, snapshot pruning.
- Memories: remember / recall / forget, deduplicated, injected into every task.

### slate CLI
- `hook pre-tool-use` / `post-tool-use`, `mcp` (approve, remember, recall, forget), `status`, `audit`, `tasks`, `undo`, `remember`, `memories`, `forget`, `skills install` / `list`.

### slate-desktop
- Agent seat on Wayland: transient seat, virtual pointer and keyboard, Unicode typing via generated keymaps, key combos.
- Per-window and full-output capture to PNG through ext-image-copy-capture.
- MCP tools: windows, screenshot, click, move, scroll, type, key, launch; window-relative coordinates from sway IPC.
- `seat: "user"` fallback for toolkits that bind only the first seat (GTK4), gated as Confirm.

### Distribution
- `flake.nix` with the package and a NixOS module (`services.slate.enable`, login shell users, sway, slated user service, snapshot root).
- Base OS Skills: audio, brightness, Wi-Fi, NixOS system, systemd, undo.

### Docs
- Architecture, roadmap, ADRs 0001–0007, policy file reference, contributing guide.

## 0.0.1 — 2026-09-26

Repository skeleton, architecture document, placeholder crates.
