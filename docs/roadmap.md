# Roadmap

Dates are intentions, not promises. Everything here is open for discussion. Releases are small and frequent (`0.0.x`); `0.1.0` comes after a full manual pass by the maintainer.

## M0: design — done for v0

- [x] Architecture document
- [x] Initial decisions recorded (ADR 0001–0007)
- [ ] RFCs opened for the remaining open questions in `architecture.md`
- [ ] OS Skills manifest format v1 (v0 is in use)

## M1: the agent seat — mechanism proven, in daily use on the dev machine

- [x] slate-desktop: transient seat with virtual pointer and keyboard
- [x] Per-window capture via ext-image-copy-capture, cropped to the window geometry
- [x] MCP server: windows / screenshot / click / move / scroll / type / key / close / focus / window_set / arrange / launch / seats / status / takeover_cancel
- [x] Unicode typing via generated keymaps; ASCII through a real US keymap (Firefox ignores made-up keycodes); key combos
- [x] Long-lived agent seat: `slate-desktop daemon` as a supervised user service (apps only honour seats present at their start)
- [x] User-seat fallback for first-seat-only toolkits (GTK4), gated as Confirm
- [x] Takeover indicator: panel blinks "controlling", sway `controlling` mode, Esc hands control back and blocks retries
- [x] Focus-aware input: per-seat focus from the compositor, focus-then-verify before typing, post-action screenshots
- [x] Demos: Claude Code and Codex drive a terminal and Firefox through the agent seat from slash
- [x] AT-SPI2: elements, read, element actions and EditableText over the bus; GTK3, GTK4, Firefox verified
- [ ] sway patch: security-context clients see only the agent seat (ADR 0007)
- [ ] Ghost cursor rendering (sway patch)
- [ ] Toolkit compatibility list: Qt, Chromium/Electron, Flatpak (GTK3 and foot verified; GTK4 needs the user seat)

## M2: slated — core done

- [x] Approval broker serving Claude Code's permission-prompt tool
- [x] Tier policy (Observe / Reversible / Confirm), command-aware shell classifier, `~/.config/slate/policy.toml`
- [x] `/auto on|off`: per-session bypass, audited
- [x] Audit log, `slate audit`, `/audit`, `/tasks`
- [x] btrfs snapshot before the first non-observe call of a task, `/undo`, `/undo --preview`, pruning
- [x] Memory: remember / recall / forget MCP tools, `/remember` and `/memories`, listed at the start of every task
- [x] systemd user service, notifications for terminal tasks, quiet tasks for the prompt
- [ ] Root for system changes with the user's consent (the agent cannot run `nixos-rebuild` today)
- [ ] Codex approvals (Codex exec has no approval channel yet; runs under its sandbox)
- [ ] Session context server (active windows, recent outputs); context is a per-turn delta today
- [ ] Agent identity: separate uid / Landlock / cgroup

## M3: slash — done for v0

- [x] Input routing: bare / `/` / `!` / `@`
- [x] Backend adapters: Claude Code (stream-json, session resume, hooks, permission tool, MCP) and Codex (exec --json, thread resume, MCP)
- [x] `!` in the real shell on a pty; output captured for the agent; cwd and exported env persist
- [x] Login-shell compatibility: non-interactive execs the POSIX shell; installed as `/run/current-system/sw/bin/slash`
- [x] Answer-first rendering, tool calls as one-liners, `/verbose`, streaming text, `thinking` events when the backend streams reasoning
- [x] Model selection (default sonnet), `/model`, `/agent`
- [x] `slash --serve`: JSON-lines session protocol for GUI clients
- [x] Identity prompt: slash is the shell of SlateOS; per-turn context delta
- [ ] Conversation history across prompt sessions

## M3b: the desktop

- [x] Phase 1: sway as a stacking desktop, waybar, fuzzel, mako, wallpaper, greetd autologin, `services.slate.desktop.enable`
- [x] Phase 2: the Slate prompt (Siri-style overlay: one exchange at a time, approvals as buttons, passive reappearance, click-elsewhere dismiss), panel status module, settings app (display with confirm-or-revert and a HiDPI preset, sound, network, memories)
- [x] Phase 3: SlateOS identity, quiet dark theme (Inter, JetBrains Mono, Font Awesome), AI settings page (backend, models, sign-in, verbose, bypass approvals)
- [ ] Slate's own notification surface for terminal tasks (mako today)
- [ ] Rust rewrite of the prompt and settings once the design settles
- [ ] Agent-aware window markers and "hand this window to the agent"

## M4: SlateOS as a product

- [x] NixOS module + flake: `services.slate.enable`, login shell, user services, desktop profile, SlateOS branding
- [x] Base distribution: NixOS (ADR 0004)
- [ ] Root/polkit story for system changes (shared with M2)
- [ ] Compositor patches upstreamed or carried (seat filtering, ghost cursor)
- [ ] Base OS Skills: browser, files, mail, calendar, printing, Bluetooth, power
- [ ] Installable image and installer (homes as user-owned btrfs subvolumes, skills preinstalled, agent sign-in on first boot)
- [ ] `0.1.0` after the maintainer's full manual pass

## Later

- Multi-agent, multi-seat
- Headless output for fully hidden agent work
- Skill sharing
- Non-wlroots compositor support if the protocols land upstream
