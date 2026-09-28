<p align="center">
  <h1 align="center">Slate</h1>
  <p align="center"><b>An AI-native Linux where humans and agents share the same desktop.</b></p>
  <p align="center">
    <a href="#status">Status: design phase</a> ·
    <a href="docs/architecture.md">Architecture</a> ·
    <a href="docs/roadmap.md">Roadmap</a> ·
    <a href="CONTRIBUTING.md">Contributing</a> ·
    <a href="README.zh-CN.md">中文</a>
  </p>
</p>

---

Slate is a Linux distribution and agent runtime built on one idea: **you should not need a shell to use your computer, but the agent should have a better one than you ever did.**

You talk to your machine in natural language. An agent does the work, using your existing Claude Code or Codex subscription as its brain. It gets its own cursor, its own keyboard focus and its own clipboard, so it can drive any app on your desktop in the background while you keep working in the foreground. Every action is audited, every change is snapshotted, and "undo" always works.

zsh and bash are still there. You will just stop opening them.

## Why this does not exist yet

Pieces of it do:

- **Alibaba Cloud Linux Agentic Edition** ships a natural-language default shell and machine-readable OS skills, but it targets cloud servers, not a desktop you sit in front of.
- **Omarchy** is an agent-first desktop distro, but its agents live in terminal windows and share your mouse.
- **Codex on macOS** gives an agent its own cursor and drives apps in the background. It does it by reaching into private SkyLight APIs, and it only runs on a Mac.
- **Doubao Phone** and the other system-level phone agents proved people will hand long, cross-app tasks to an agent, on a platform where you are always watching it work.

Nobody has assembled these into a desktop where a human and several agents genuinely co-exist. On Wayland, the primitives to do it properly (independent seats, per-window capture, virtual input, transient seats) are already standard protocols. Slate is the project that puts them together.

## What it looks like

```
❯ the flight confirmation from last week, put the dates in my calendar and
  forward the pdf to Alice

  ▸ found "Booking confirmation – SFO→NRT" in Thunderbird (Sep 18)
  ▸ creating 2 events in GNOME Calendar (Oct 3 depart, Oct 17 return)
  ▸ Thunderbird: compose → alice@… → attach confirmation.pdf
  ⏸ send email to alice@example.com?  [y] send  [n] cancel  [v] view draft
```

While that runs, Thunderbird and Calendar are being driven by the agent's seat. Your mouse never moves. You can keep typing in your editor, or alt-tab over to watch the ghost cursor work, or say **stop** or **I'll take it from here**.

```
❯ !git status                 # ! runs a line in your real zsh
❯ /agent codex                # / talks to slash itself or the agent backend
❯ /undo                       # roll back the last task's filesystem changes
```

## Architecture in one screen

```
┌──────────────────────────────────────────────────────────────────────┐
│  You                                                                 │
│   ├─ slash   (terminal)      ─┐   same session, two views            │
│   └─ slash   (desktop palette)┘                                      │
├──────────────────────────────────────────────────────────────────────┤
│  slash — the shell / front end                                       │
│   • routes: bare text → agent, /cmd → control, !cmd → zsh            │
│   • renders the backend's event stream, answers first, stdout folded │
│   • owns no agent loop and no model tokens                           │
├──────────────────────────────────────────────────────────────────────┤
│  Agent backend (bring your own)                                      │
│   claude (Claude Code)  │  codex  │  …                               │
│   driven only through their official surfaces:                       │
│   headless mode · hooks · MCP · skills · permission-prompt tool      │
├──────────────────────────────────────────────────────────────────────┤
│  slated — the daemon (the OS-level part)                             │
│   identity ─ approval broker ─ audit log ─ snapshots/undo ─ memory   │
│   skills registry ─ session context ─ MCP servers for the backends   │
├──────────────────────────────────────────────────────────────────────┤
│  slate-desktop — background computer use for Linux                   │
│   agent seat (ext-transient-seat) · virtual pointer/keyboard         │
│   per-window capture · AT-SPI2 tree · ghost cursor · headless output │
├──────────────────────────────────────────────────────────────────────┤
│  Wayland compositor (wlroots-based, patched as needed) · Linux       │
└──────────────────────────────────────────────────────────────────────┘
```

Full detail, including the open questions, is in [docs/architecture.md](docs/architecture.md).

## Principles

1. **Bring your own agent.** Slate never calls a model API. It launches the official `claude` or `codex` binary and integrates through hooks, MCP and skills. Your subscription stays yours and stays within its terms of service. Backends are pluggable.
2. **The agent gets its own seat.** Not your mouse, not your focus, not your clipboard. Co-existence is a compositor-level guarantee, not a UX convention.
3. **Prefer the boring path.** CLI, D-Bus and config files before the accessibility tree; the accessibility tree before screenshots. OS Skills teach the agent the boring path for every part of the system.
4. **Undo is a first-class verb.** Every task starts with a snapshot. Reversible actions run without asking. Destructive ones stop and ask.
5. **Everything is auditable.** Every action an agent takes is logged with what it saw, what it did and which tier of approval it had.
6. **The escape hatch is always open.** `!` gives you real zsh. `chsh` gives you your old life back. Nothing in Slate is load-bearing for the underlying Linux.

## Components

| Crate | What it is | Status |
|---|---|---|
| `crates/slash` | The shell. Terminal and desktop-palette views of the same session. | v0: works with Claude Code and Codex, see [crates/slash](crates/slash) |
| `crates/slated` | The daemon. Approvals, tier policy, audit, snapshots and undo. Identity, memory and skills to come. | v0: approvals, audit and undo work |
| `crates/slate` | The CLI for `slated`, plus the hook and MCP entry points Claude Code calls. | v0 |
| `crates/slate-desktop` | Background computer use: agent seat, per-window capture, input, over MCP. | v0: works on sway, see [crates/slate-desktop](crates/slate-desktop) |
| `crates/slate-proto` | Shared types crossing process boundaries. | placeholder |
| `skills/` | OS Skills: machine-readable manuals for the system. | examples only |
| `distro/` | Image build for SlateOS. Base distribution not yet decided. | empty |

## Status

**Pre-alpha, but real.** As of 2026-09-27 everything below runs on the dev machine (NixOS 26.05, sway 1.12) and is exercised end to end with both Claude Code and Codex:

- **slash**: natural-language shell with `/` and `!`, pty-backed `!` commands whose output the agent can see, Claude Code (stream-json, session resume) and Codex (exec --json) backends, streaming output.
- **slated**: tier policy (Observe / Reversible / Confirm) with a `policy.toml`, approvals routed through Claude Code's permission tool to the human at the terminal, audit log, privilege-free btrfs snapshots with `/undo`, memories (`remember` / `recall` / `forget`).
- **slate-desktop**: an agent seat on Wayland with its own pointer and keyboard, per-window screenshots, Unicode typing, MCP tools that both backends use. Verified: an agent drives a terminal window through its own seat and reads the result from a screenshot. Known gap: GTK4 apps only listen to the first seat (ADR 0007).
- **Desktop (phase 1 + 2)**: `services.slate.desktop.enable` gives a conventional sway desktop with a panel, launcher, notifications, a settings app, the floating **Slate Shell** panel (Mod+s: chat, streamed answers, approvals as buttons, agent status), a supervised agent-seat daemon, a blinking "controlling" indicator with Esc to take your mouse and keyboard back, and focus-verified typing.
- **NixOS module**: `services.slate.enable` installs everything, registers slash as a login shell, runs slated as a user service, and brands the system as SlateOS (NixOS underneath; `ID_LIKE=nixos`).

Not built yet: the desktop shell layer, ghost cursor and human takeover (need compositor patches), accessibility-tree input, agent identity isolation, the installer. See [docs/roadmap.md](docs/roadmap.md).

## Contribute

Slate is in its design phase, so the most useful contributions are arguments, prototypes and skills, not polish. Areas where help matters most:

- **Wayland / wlroots internals** — multi-seat, transient seats, toplevel capture, compositor patching
- **Accessibility on Linux** — AT-SPI2, getting Chromium/Electron/Flatpak apps to expose their trees
- **Claude Code and Codex extension surfaces** — hooks, MCP, headless modes, permission tools
- **Btrfs / NixOS** — snapshot and rollback strategy for the daemon
- **Distro building** — Arch vs NixOS base, image pipeline, installer
- **Writing OS Skills** — the boring, correct way to do every common task on a Linux desktop

See [CONTRIBUTING.md](CONTRIBUTING.md). Issues labelled `good first issue` and `rfc` are the entry points; design changes go through [docs/rfcs](docs/rfcs).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
