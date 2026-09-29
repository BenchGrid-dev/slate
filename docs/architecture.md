# Slate architecture

Status: living document, last revised 2026-09-28 (release 0.0.10). It describes what SlateOS is and how the parts fit; where the implementation is behind the design, the section says so. Settled decisions live in `decisions/`. Anything marked **open** is a real question and a good place to contribute.

## 1. The one-sentence version

A Linux desktop you operate by talking: the shell is a conversation, the agent is the user's own Claude Code or Codex subscription driven only through official extension surfaces, it works in the user's apps through its own Wayland seat, and an OS-level daemon owns approvals, audit, snapshots and memory so everything it does is visible and reversible.

## 2. Layers

```
you ──► slash ──► agent backend (claude | codex) ──► slated ──► slate-desktop ──► compositor ──► apps
                       ▲                               │
                       └──── hooks / MCP / skills ─────┘
```

Read top to bottom: you type into slash, slash hands the request to a backend, the backend reasons and calls tools, every tool that touches the OS is served by slated or slate-desktop, and those go through the compositor to reach real applications. slated also feeds context back into the backend (skills, memory, recent session state) through the same official surfaces.

### 2.1 slash: the shell

slash is the primary interface of the OS. It runs in a terminal, and it runs behind the desktop's Slate prompt (`slash --serve`, a JSON-lines protocol the prompt speaks). Both are views of the same kind of session; slated tracks every task either of them starts.

Input routing:

| Prefix | Goes to | Examples |
|---|---|---|
| (none) | the agent backend | `find the pdf Alice sent and print it` |
| `/` | slash itself, or passed through to the backend if slash does not recognise it | `/agent codex`, `/undo`, `/mcp`, `/skills`, `/audit`, `/snapshot`, `/compact` (passed to Claude Code) |
| `!` | a persistent zsh/bash session | `!git status`, `!vim ~/.config/foo` |
| `@` | file reference inside an agent message | `summarise @report.pdf` |

`!` lines go to a long-lived shell process on a pty so `cd`, environment, aliases and user rc files behave. History expansion in the underlying shell is disabled; slash owns history.

Rendering: the backend's structured event stream (for Claude Code, `--output-format stream-json`; for Codex, `exec --json`) is rendered by slash. The default view shows the agent's answer and one line per tool call; `/verbose` shows raw events. The Slate prompt shows one exchange at a time: the question, the answer, the current action and, when the backend streams it, the tail of the model's reasoning.

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

Responsibilities (implemented unless marked otherwise):

- **Identity.** **Not started.** The design: agent tasks run as a separate Linux user, or as a dedicated session under the user's uid with its own cgroup, Landlock ruleset and polkit identity. **Open:** separate uid vs same uid with sandboxing; trade-offs are file ownership friction vs weaker isolation. Related and more urgent: the agent has no way to obtain root for system changes today (no password prompt, no polkit agent); a Slate-mediated privilege path is the next item on the roadmap.
- **Approval broker.** Implements Claude Code's permission-prompt MCP tool and Codex's approval flow. Classifies every requested action into a tier (below) and either allows, snapshots-then-allows, or surfaces a prompt to the user via slash or the desktop shell.
- **Audit log.** Append-only log of every tool call: backend, session, what the agent saw (hash of screenshot / a11y snapshot), what it did, tier, outcome. Queryable via `slate audit`.
- **Snapshots and undo.** On the first non-observe tool call of a task, take a read-only btrfs snapshot of the user's home (which must be a user-owned subvolume; see `decisions/0006-privilege-free-snapshots.md`). `/undo` diffs the snapshot against the live tree inside the directories the task touched and restores, deletes or recreates files accordingly. No privileges are needed. Non-filesystem side effects (emails sent, network calls) are unrecoverable and are therefore always tier Confirm. System-level state on NixOS (generations) is a separate, later concern.
- **Memory.** Things the user asked to remember (`/remember`, or the agent's `remember` tool), per user, not per directory. Exposed to backends as MCP tools (`remember` / `recall` / `forget`) and listed at the start of every task.
- **Session context.** The shell commands the user ran by hand since the agent's last turn (exit code, directory, output) and new memories, sent as a delta at the start of each user message. A richer context server (active windows, recent outputs) is planned.
- **Skills registry.** `slate skills install` links the OS Skills (below) into the backend's own skills directory, filtered by what the machine has (`requires`, `applies_to`).
- **MCP servers.** `slate mcp` serves approvals and memory to the backends. slate-desktop serves computer use separately.

Approval tiers:

| Tier | Behaviour | Examples |
|---|---|---|
| Observe | run silently | read files, list windows, take a screenshot, query D-Bus |
| Reversible | snapshot, then run silently; undo available | edit config, move files, change settings, install a package |
| Confirm | stop and ask | delete outside a snapshot's reach, send email/message, network POST, anything touching credentials or payment |

Classification lives in `slated/src/policy.rs`, extended by `~/.config/slate/policy.toml` (see `docs/policy.md`). Unknown actions default to Confirm. `/auto on` bypasses Confirm for a session; the bypass is recorded in the audit log and snapshots still happen.

How it is wired for Claude Code: slash launches `claude` with `--permission-mode default`, a PreToolUse hook (`slate hook pre-tool-use`) that asks slated for the tier and answers `allow` / `ask`, and `--permission-prompt-tool mcp__slate__approve`, an MCP tool served by `slate mcp` that forwards Confirm-tier calls to slated, which asks the human through whichever slash session started the task. The agent binary never sees anything but its own documented extension points.

### 2.4 slate-desktop: background computer use for Linux

This is the piece that does not exist anywhere else on Linux. It gives an agent a way to see and drive GUI applications without taking the human's mouse, keyboard, focus or clipboard.

Mechanism, in order of preference for any given action:

1. **Non-GUI path.** If an OS Skill says the task can be done via CLI, D-Bus, a config file or an app's own API, do that. Cheapest, most reliable, fully auditable. (Implemented: this is what the skills teach.)
2. **Accessibility tree.** AT-SPI2 gives a structured tree for GTK, Qt, Firefox, Chromium and LibreOffice. Agents list a window's elements by role and name, read its text, and activate elements or set their text through the bus, with no pointer or keyboard involved, which also sidesteps toolkits that ignore extra seats. (Implemented: `desktop_elements`, `desktop_read`, `desktop_element_click`, `desktop_element_set_text`; verified on GTK3, GTK4 and Firefox.)
3. **Pixels and virtual input.** Per-window capture plus synthesized pointer and keyboard events, for elements without actions and windows without a tree (terminals, canvases). (Implemented.)

Compositor-level primitives used:

- `ext-transient-seat-v1` to create an independent seat per agent task, with its own keyboard focus, pointer focus and data device (clipboard).
- `zwlr-virtual-pointer-v1` and `zwp-virtual-keyboard-v1`, bound to the agent seat.
- `ext-image-copy-capture-v1` with a foreign-toplevel source for per-window capture, including occluded windows.
- `ext-foreign-toplevel-list-v1` / `zwlr-foreign-toplevel-management-v1` for window enumeration and control.
- Sway IPC for window geometry, per-seat focus and window management (move, resize, arrange, focus, close).
- Planned: a headless output where agent windows live until the user wants to look; a distinct "ghost" cursor for the agent seat (compositor patch).

What runs today: `slate-desktop daemon` owns one agent seat for the whole session (toolkits only accept input from seats that existed when they started, so the seat must predate every app; ADR 0007). Typing and key presses take a target window, focus it for the seat in use, verify the focus through the compositor, then type, and report which window received the input. Every action returns a screenshot so the agent checks its own work.

Toolkits that only bind the first seat (GTK4 today) can be driven through the user's own seat instead. That is a Confirm-tier action: the panel blinks **controlling**, sway enters a `controlling` mode, and Esc hands control back and refuses further user-seat input for a minute so the agent has to ask again.

Human takeover beyond Esc ("freeze the agent seat", "hand me this window", "show me the agent's windows") is planned with the ghost cursor and headless output.

API shape: slate-desktop is an MCP server exposing window-scoped operations (list windows, capture window, get a11y tree, click, type, key, scroll, drag, set clipboard) so that both Claude Code and Codex can use it without either vendor shipping Linux computer use. The API is modelled on the shape of existing background computer-use tools on macOS so prompts and skills transfer.

Compositor support: these protocols are implemented by wlroots-based compositors, not by GNOME or KDE. SlateOS ships sway, which has the most mature multi-seat implementation, configured as a conventional stacking desktop. **Open:** whether to stay on sway with carried patches (seat filtering, ghost cursor) or move to a thin compositor of our own once the patches are known.

Known gaps: AT-SPI2 coverage is weaker than macOS accessibility. Chromium/Electron need accessibility enabled explicitly; Flatpak sandboxing can block AT-SPI without portal support; Wine and games have no tree at all. Expect heavier screenshot use than macOS tools, and lean on OS Skills to avoid the GUI entirely where possible.

### 2.5 OS Skills

Machine-readable manuals that tell an agent the correct, boring way to do things on this system. A skill is a directory with a manifest and markdown: what the task is, the preferred non-GUI path, fallbacks, what tier the actions are, how to verify success, how to undo.

`slate skills install` links them into the backend's native skills directory (Claude Code today; an AGENTS.md section for Codex is planned), skipping skills whose `requires` tools or `applies_to` distro do not match the machine. Slate ships a base set (`skills/base`); users and the community add more. Format is in `skills/README.md`.

### 2.6 The desktop shell

SlateOS ships sway with a shell layer built as ordinary Wayland clients: waybar (with a Slate status module: idle / working / controlling), fuzzel, mako, a settings app (display and HiDPI with confirm-or-revert, sound, network, memories, and the AI page for backend, models, sign-in, verbose and approval defaults), and the Slate prompt: a layer-shell overlay in the top-right corner that shows one exchange at a time, approvals as buttons, and reappears with results without taking the keyboard. Applications are standard GTK, Qt, Electron and Chromium; nothing needs to be modified.

The prompt and settings app are Python/GTK4 (quick to iterate on). **Open:** when to rewrite them in Rust, and which agent-aware affordances to add next ("agent is working here" markers on windows, "hand this window to the agent", "ask about this selection").

### 2.7 SlateOS: the distribution

The distribution exists so the compositor, slated, slash, the desktop and the skills are installed, configured and privileged correctly out of the box. Nothing in Slate requires the distribution; every component runs on any wlroots-based Wayland desktop with reduced guarantees.

SlateOS is built on NixOS. The flake's NixOS module installs everything, makes slash the login shell, runs the daemons as user services, ships the desktop profile, and presents the system as SlateOS (`ID=slateos`, `ID_LIKE=nixos`; the system configuration lives in `/etc/slateos` and the `slateos-*` tools front the `nixos-*` ones, which stay available). Declarative, diffable, rollback-able system state is a natural fit for an agent that changes the system. ADR 0004 records the alternatives; an Arch base is possible if someone wants to build it. No installable image exists yet.

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

- Root for system changes: how the agent obtains privileges for `nixos-rebuild` and friends with the user's consent (a Slate approval that unlocks polkit? a scoped sudo rule written by the installer?).
- Agent identity: separate uid vs sandboxed same uid.
- Filesystem snapshots for system state: btrfs (home, done) vs NixOS generations (system) vs both.
- Compositor: keep sway with carried patches, or a thin compositor of our own.
- Shell toolkit: when to move the prompt and settings from Python/GTK4 to Rust.
- OS Skills manifest format, and how much to align with the Alibaba Agentic OS skills format for reuse.
- Multi-agent: several backends or several sessions of one backend at once, each with its own seat.
- How much session context to give the backend by default, given subscription rate limits.

Settled since the first draft: base distribution (NixOS, ADR 0004), IPC (unix sockets with a JSON envelope, `slate-proto`), the prefix grammar (ADR 0003), privilege-free snapshots (ADR 0006), the long-lived agent seat (ADR 0007).

## 6. Prior art and references

- Alibaba Cloud Linux 4 Agentic Edition: cosh, OS Skills, AgentSecCore, AgentSight. github.com/alibaba/anolisa
- Omarchy: agent-first Arch desktop. omarchy.org
- Codex on macOS computer use: background operation with own cursor via SkyLight and AX.
- open-codex-computer-use: open-source background computer use for macOS, MCP-shaped API.
- Wayland protocols: ext-transient-seat-v1, zwlr-virtual-pointer-v1, zwp-virtual-keyboard-v1, ext-image-copy-capture-v1, ext-foreign-toplevel-list-v1.
- Doubao Phone and system-level phone agents: user expectations for cross-app long tasks and memory.
