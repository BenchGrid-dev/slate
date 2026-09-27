# Roadmap

Dates are intentions, not promises. Everything here is open for discussion.

## M0: design (now)

- [x] Architecture document
- [x] Initial decisions recorded
- [ ] RFCs opened for each open question in `architecture.md`
- [ ] OS Skills manifest format v0

## M1: the agent seat (the thing nobody has)

Goal: on stock sway, an agent drives a GTK app through its own transient seat while a human uses the same desktop, with a visible ghost cursor.

- [ ] slate-desktop: create transient seat, bind virtual pointer and keyboard
- [ ] slate-desktop: per-window capture via ext-image-copy-capture
- [ ] slate-desktop: AT-SPI2 tree read and action
- [ ] slate-desktop: MCP server with list/capture/tree/click/type/key/scroll
- [ ] Demo: Claude Code (headless) fills a form in a GTK app via slate-desktop while the user types in a terminal
- [ ] Demo: same with Codex
- [ ] Ghost cursor rendering (may require a sway patch)

## M2: slated core

- [ ] Approval broker serving Claude Code's permission-prompt tool and Codex approvals
- [ ] Tier policy file, default policy
- [ ] Audit log, `slate audit`
- [ ] Btrfs snapshot before task, `/undo`
- [ ] Session context MCP server
- [ ] Memory MCP server (minimal)

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
