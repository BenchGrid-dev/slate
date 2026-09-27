# Slate architecture

Status: draft, 2026-09. This document is the source of truth for what Slate is trying to build. Decisions that are settled live in `decisions/`. Anything marked **open** is a real question and a good place to contribute.

## 1. The one-sentence version

A Wayland desktop where agents get their own input seat, driven by the user's existing Claude Code or Codex subscription through official extension surfaces only, with an OS-level daemon that owns identity, approvals, audit, snapshots and memory.

## 2. Layers

```
you ──► slash ──► agent backend (claude | codex) ──► slated ──► slate-desktop ──► compositor ──► apps
                       ▲                               │
                       └──── hooks / MCP / skills ─────┘
```

Read top to bottom: you type into slash, slash hands the request to a backend, the backend reasons and calls tools, every tool that touches the OS is served by slated or slate-desktop, and those go through the compositor to reach real applications. slated also feeds context back into the backend (skills, memory, recent session state) through the same official surfaces.

### 2.1 slash: the shell

slash is the primary interface of the OS. It runs in a terminal and it runs as a desktop palette; both are views onto the same session held by slated.

Input routing:

| Prefix | Goes to | Examples |
|---|---|---|
| (none) | the agent backend | `find the pdf Alice sent and print it` |
| `/` | slash itself, or passed through to the backend if slash does not recognise it | `/agent codex`, `/undo`, `/mcp`, `/skills`, `/audit`, `/snapshot`, `/compact` (passed to Claude Code) |
| `!` | a persistent zsh/bash session | `!git status`, `!vim ~/.config/foo` |
| `@` | file reference inside an agent message | `summarise @report.pdf` |

`!` lines go to a long-lived shell process on a pty so `cd`, environment, aliases and user rc files behave. History expansion in the underlying shell is disabled; slash owns history.

Rendering: the backend's structured event stream (for Claude Code, `--output-format stream-json`) is rendered by slash. The default view shows the agent's progress and answer; raw command output is folded and expandable. The terminal view defaults to showing more of the process, the palette view to showing less.

slash as a login shell: when invoked non-interactively or with `-c`, slash execs the user's configured POSIX shell immediately. Only an interactive tty enters agent mode. This is required for `$SHELL -c` calls from editors and tools to keep working.

slash does **not** contain an agent loop and does **not** hold model credentials.

### 2.2 Agent backends: bring your own

Slate does not call model APIs. It launches the official CLI of a backend and integrates through that backend's documented extension surfaces.

| Backend | Launch | Integration surfaces used |
|---|---|---|
| Claude Code | `claude -p --output-format stream-json`, or interactive | hooks (PreToolUse, PostToolUse, …), MCP servers, `--permission-prompt-tool`, CLAUDE.md, skills, plugins |
| Codex | `codex exec`, or interactive | MCP servers in config.toml, AGENTS.md, hooks, approval modes |

Why: since 2026-02 Anthropic's consumer terms restrict subscription OAuth to Claude.ai, Claude Code and the desktop app; third-party harnesses, including the Agent SDK, must use API keys. The only way to let people use the subscription they already pay for is to run the real binary and stay inside its extension surfaces. Codex is treated the same way for symmetry, and so that policy changes on either side only touch an adapter. See `decisions/0001-bring-your-own-agent.md`.

Consequence: slash and slated are backend-agnostic. A backend adapter is a small module that knows how to launch the binary, how to parse its event stream, how to install hooks and MCP config into it, and how to answer its permission prompts.

### 2.3 slated: the daemon

slated is the part of Slate that is an operating-system component rather than a tool. One instance per user session.

Responsibilities:

- **Identity.** Agent tasks run as a separate Linux user (or a dedicated session under the user's uid with its own cgroup, Landlock ruleset and polkit identity). **Open:** separate uid vs same uid with sandboxing; trade-offs are file ownership friction vs weaker isolation.
- **Approval broker.** Implements Claude Code's permission-prompt MCP tool and Codex's approval flow. Classifies every requested action into a tier (below) and either allows, snapshots-then-allows, or surfaces a prompt to the user via slash or the desktop shell.
- **Audit log.** Append-only log of every tool call: backend, session, what the agent saw (hash of screenshot / a11y snapshot), what it did, tier, outcome. Queryable via `slate audit`.
- **Snapshots and undo.** Before each task, snapshot the relevant state. Filesystem via btrfs subvolume snapshots or NixOS generations (**open**, see `decisions/0004-base-distribution.md`). `/undo` and "undo that" roll back the last task. Non-filesystem side effects (emails sent, network calls) are unrecoverable and are therefore always tier Confirm.
- **Memory.** User preferences, task history, things the user explicitly asked to remember. Exposed to backends as an MCP server and injected into CLAUDE.md / AGENTS.md as summaries.
- **Session context.** Recent slash history, cwd, last outputs, active windows. Given to the backend at session start and refreshed via MCP.
- **Skills registry.** Loads OS Skills (below), makes them discoverable to backends.
- **MCP servers.** slated exposes memory, snapshots, audit, skills and session context as MCP tools. slate-desktop exposes computer use separately.

Approval tiers:

| Tier | Behaviour | Examples |
|---|---|---|
| Observe | run silently | read files, list windows, take a screenshot, query D-Bus |
| Reversible | snapshot, then run silently; undo available | edit config, move files, change settings, install a package |
| Confirm | stop and ask | delete outside a snapshot's reach, send email/message, network POST, anything touching credentials or payment |

Classification comes from a policy file that skills and users can extend. Unknown actions default to Confirm.

### 2.4 slate-desktop: background computer use for Linux

This is the piece that does not exist anywhere else on Linux. It gives an agent a way to see and drive GUI applications without taking the human's mouse, keyboard, focus or clipboard.

Mechanism, in order of preference for any given action:

1. **Non-GUI path.** If an OS Skill says the task can be done via CLI, D-Bus, a config file or an app's own API, do that. Cheapest, most reliable, fully auditable.
2. **Accessibility tree.** AT-SPI2 gives a structured tree for GTK, Qt and (when enabled) Chromium/Electron apps. Agents act on named controls, not pixels. Far fewer tokens than screenshots.
3. **Pixels and virtual input.** Per-window capture plus synthesized pointer and keyboard events, when the tree is missing or wrong.

Compositor-level primitives used:

- `ext-transient-seat-v1` to create an independent seat per agent task, with its own keyboard focus, pointer focus and data device (clipboard).
- `zwlr-virtual-pointer-v1` and `zwp-virtual-keyboard-v1`, bound to the agent seat.
- `ext-image-copy-capture-v1` with a foreign-toplevel source for per-window capture, including occluded windows.
- `ext-foreign-toplevel-list-v1` / `zwlr-foreign-toplevel-management-v1` for window enumeration and control.
- Optionally a headless output: agent windows live on a virtual output the user cannot see, and are moved to a visible output when the user wants to look or take over.
- A per-seat cursor rendered as a distinct "ghost" cursor so the user can see where the agent is acting.

Human takeover: the user can at any time say "stop" (freeze the agent seat), "I'll take it" (move the window's focus to the user's seat and end the task), or "show me" (bring the agent's windows to a visible output without ending the task).

API shape: slate-desktop is an MCP server exposing window-scoped operations (list windows, capture window, get a11y tree, click, type, key, scroll, drag, set clipboard) so that both Claude Code and Codex can use it without either vendor shipping Linux computer use. The API is modelled on the shape of existing background computer-use tools on macOS so prompts and skills transfer.

Compositor support: these protocols are implemented by wlroots-based compositors, not by GNOME or KDE. The prototype targets sway, which has the most mature multi-seat implementation. Slate OS will ship a wlroots-based compositor, patched where needed. **Open:** sway vs Hyprland vs niri vs a thin compositor of our own on wlroots or smithay.

Known gaps: AT-SPI2 coverage is weaker than macOS accessibility. Chromium/Electron need accessibility enabled explicitly; Flatpak sandboxing can block AT-SPI without portal support; Wine and games have no tree at all. Expect heavier screenshot use than macOS tools, and lean on OS Skills to avoid the GUI entirely where possible.

### 2.5 OS Skills

Machine-readable manuals that tell an agent the correct, boring way to do things on this system. A skill is a directory with a manifest and markdown: what the task is, the preferred non-GUI path, fallbacks, what tier the actions are, how to verify success, how to undo.

They are loaded by slated and exposed to backends through their native skills mechanisms (Claude Code skills, AGENTS.md sections for Codex). Slate ships a base set; users and the community add more. Format is in `skills/README.md`.

### 2.6 The desktop shell

Slate OS ships a wlroots-based compositor with a shell layer (panel, launcher, notifications, approval toasts, agent status) built as ordinary Wayland clients. Applications are standard GTK, Qt, Electron and Chromium; Slate does not require apps to be modified.

Agent-aware affordances in the shell: an "agent is working here" marker on windows owned by an agent seat, a live status line, non-modal approval toasts, "hand this window to the agent", "ask about this selection", "remember this screenshot".

**Open:** shell toolkit (Quickshell, AGS, or custom).

### 2.7 Slate OS: the distribution

The distribution exists so the compositor, slated, slash and the skills are installed, configured and privileged correctly out of the box. Nothing in Slate requires the distribution; every component should run on any wlroots-based Wayland desktop with reduced guarantees.

**Open:** base. NixOS gives declarative, diffable, rollback-able system state, which is a natural fit for agents that change the system. Arch gives a larger app and community surface and a validated precedent (Omarchy). See `decisions/0004-base-distribution.md`.

## 3. A task, end to end

1. User types in slash: "install the font from the zip in Downloads and set it as the terminal font".
2. slash sends the message to the active backend session (Claude Code, headless) with session context from slated.
3. Backend reads the `fonts` OS Skill: preferred path is unzip to `~/.local/share/fonts` and `fc-cache`, then edit the terminal config. No GUI needed.
4. Backend calls `Bash`. Claude Code's PreToolUse hook asks slated. slated classifies as Reversible, takes a snapshot, records the audit entry, allows.
5. Backend edits the terminal config. Same flow.
6. Backend reports done. slash renders the answer; stdout of `fc-cache` is folded.
7. User says "undo, it looks bad". slated rolls back the snapshot; the audit log records the rollback.

A GUI task differs only at step 3 and 4: the skill says the app has no CLI, the backend calls slate-desktop's MCP tools, slate-desktop creates a transient seat, drives the app, and the ghost cursor is visible if the user looks.

## 4. Non-goals

- Training or hosting models. Slate has no model of its own.
- Replacing bash or zsh. They stay, unmodified.
- Modifying applications. Standard apps must work as-is.
- Supporting X11 as a first-class target. The seat model needs Wayland.
- Being a cloud or server OS. That space is covered.

## 5. Open questions

Each of these deserves an RFC. Open an issue labelled `rfc` or a PR under `docs/rfcs/`.

- Agent identity: separate uid vs sandboxed same uid.
- Filesystem snapshots: btrfs vs NixOS generations vs both.
- Base distribution: NixOS vs Arch.
- Compositor: sway vs Hyprland vs niri vs own.
- IPC between slash, slated and slate-desktop: D-Bus vs varlink vs plain unix sockets with a JSON protocol.
- Shell toolkit for the desktop layer.
- OS Skills manifest format, and how much to align with the Alibaba Agentic OS skills format for reuse.
- Multi-agent: several backends or several sessions of one backend at once, each with its own seat. Design for it from day one or add later?
- How much of the session context to give the backend by default, given subscription rate limits.

## 6. Prior art and references

- Alibaba Cloud Linux 4 Agentic Edition: cosh, OS Skills, AgentSecCore, AgentSight. github.com/alibaba/anolisa
- Omarchy: agent-first Arch desktop. omarchy.org
- Codex on macOS computer use: background operation with own cursor via SkyLight and AX.
- open-codex-computer-use: open-source background computer use for macOS, MCP-shaped API.
- Wayland protocols: ext-transient-seat-v1, zwlr-virtual-pointer-v1, zwp-virtual-keyboard-v1, ext-image-copy-capture-v1, ext-foreign-toplevel-list-v1.
- Doubao Phone and system-level phone agents: user expectations for cross-app long tasks and memory.
