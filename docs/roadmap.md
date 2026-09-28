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
- [x] Demo: Claude Code (headless) drives a terminal app via slate-desktop from slash, reads the result from a screenshot
- [x] Demo: same with Codex (MCP server passed via -c overrides)
- [ ] Demo: fill a form in a GTK4 app (blocked on ADR 0007 compositor work or the user-seat fallback)
- [x] User-seat fallback for first-seat-only toolkits (GTK4), gated as Confirm
- [x] Long-lived agent seat: `slate-desktop daemon` as a supervised user service (apps only honour seats present at their start)
- [x] Takeover indicator: panel blinks "controlling", sway `controlling` mode, Esc hands control back and blocks retries
- [x] Focus-aware input: per-seat focus from the compositor, focus-then-verify before typing, post-action screenshots
- [ ] sway patch: security-context clients see only the agent seat (ADR 0007)
- [ ] Ghost cursor rendering (sway patch)
- [ ] Toolkit compatibility list: Qt, Chromium/Electron, GTK3 (GTK3 verified: works when the seat predates the app)

## M2: slated core (started; runs on the dev VM)

- [x] Approval broker serving Claude Code's permission-prompt tool
- [x] Tier policy in code (Observe / Reversible / Confirm) with a shell classifier
- [ ] Tier policy file (`~/.config/slate/policy.toml`) and per-skill tiers
- [x] Audit log, `slate audit`, `/audit`
- [x] btrfs snapshot before the first non-observe call of a task, `/undo`, `/undo --preview`
- [x] Snapshot pruning
- [ ] Codex approvals (Codex exec has no approval channel yet; runs under its sandbox)
- [ ] Session context MCP server (context is injected per turn today)
- [x] Memory: remember/recall/forget MCP tools, /remember and /memories in slash, injected into every task's context
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

## M3b: the desktop shell (phase 2)

- [x] `slash --serve`: JSON-lines session protocol for GUI clients (events, approval round trip, commands)
- [x] Slate Shell: floating layer-shell panel (top-right; Mod+s or the panel button) with streamed answers, tool lines, approval buttons, status, takeover warning
- [x] Settings app: display (with confirm-or-revert), sound, network, Slate
- [x] Panel status module (idle / working / controlling), notifications that focus the slash window
- [ ] Slate's own notifications inside the panel instead of mako
- [ ] Conversation history across panel sessions
- [ ] Rust rewrite of the panel and settings once the design settles

## M4: SlateOS image

- [x] NixOS module + flake: `services.slate.enable`, slash as login shell, slated user service
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
