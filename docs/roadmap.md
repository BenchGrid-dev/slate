# Roadmap

Dates are intentions, not promises. Everything here is open for discussion.

## M0: design (now)

- [x] Architecture document
- [x] Initial decisions recorded
- [ ] RFCs opened for each open question in `architecture.md`
- [ ] OS Skills manifest format v0

## M1: the agent seat (the thing nobody has) — mechanism proven on the dev VM

Goal: on stock sway, an agent drives a GTK app through its own transient seat while a human uses the same desktop, with a visible ghost cursor.

- [x] slate-desktop: create transient seat, bind virtual pointer and keyboard
- [x] slate-desktop: per-window capture via ext-image-copy-capture
- [ ] slate-desktop: AT-SPI2 tree read and action
- [x] slate-desktop: MCP server with windows/screenshot/click/move/scroll/type/key/launch
- [x] Unicode typing via generated keymaps, key combos
- [ ] Demo: Claude Code (headless) fills a form in a GTK app via slate-desktop while the user types in a terminal
- [ ] Demo: same with Codex
- [x] User-seat fallback for first-seat-only toolkits (GTK4), gated as Confirm
- [ ] sway patch: security-context clients see only the agent seat (ADR 0007)
- [ ] Ghost cursor rendering (sway patch)
- [ ] Human takeover: freeze / hand back / show me (device attach on a patched sway)
- [ ] Toolkit compatibility list: Qt, Chromium/Electron, GTK3

## M2: slated core (started; runs on the dev VM)

- [x] Approval broker serving Claude Code's permission-prompt tool
- [x] Tier policy in code (Observe / Reversible / Confirm) with a shell classifier
- [ ] Tier policy file (`~/.config/slate/policy.toml`) and per-skill tiers
- [x] Audit log, `slate audit`, `/audit`
- [x] btrfs snapshot before the first non-observe call of a task, `/undo`, `/undo --preview`
- [x] Snapshot pruning
- [ ] Codex approvals (Codex exec has no approval channel yet; runs under its sandbox)
- [ ] Session context MCP server
- [ ] Memory MCP server (minimal)
- [ ] Agent identity: separate uid / Landlock / cgroup
- [ ] systemd user service for slated

## M3: slash (started first; it needs no Wayland)

- [x] Input routing: bare / `/` / `!` / `@`
- [x] Backend adapter: Claude Code stream-json, session resume, context via --append-system-prompt
- [x] Backend adapter: Codex exec --json, thread resume
- [x] `!` runs in the real shell with the terminal inherited; cwd persists
- [x] `!` output capture for agent context (pty tee) and exported env persistence
- [x] Login-shell compatibility: non-interactive execs the POSIX shell
- [x] Answer-first rendering, tool calls as one-liners, stdout folded (/verbose)
- [x] Streaming partial text
- [x] Model selection (default sonnet) and /model
- [ ] Wire slated: permission-prompt tool, /undo, memory
- [ ] Desktop palette view sharing the session

## M4: Slate OS image

- [ ] Base distribution decided
- [ ] Compositor decided, patches upstreamed or carried
- [ ] Shell layer: panel, approval toasts, agent status, takeover controls
- [ ] Base OS Skills set
- [ ] Installable image, installer

## Later

- Multi-agent, multi-seat
- Headless output for fully hidden agent work
- Skill marketplace / sharing
- Non-wlroots compositor support if the protocols land upstream
