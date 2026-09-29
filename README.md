<h1 align="center">SlateOS</h1>

<p align="center">
  <strong>A Linux desktop built for humans and AI agents.</strong><br>
  Describe the task. Keep your workflow. Stay in control.
</p>

<p align="center">
  <a href="https://github.com/BenchGrid-dev/slate/actions/workflows/ci.yml"><img src="https://github.com/BenchGrid-dev/slate/actions/workflows/ci.yml/badge.svg" alt="Rust CI"></a>
  <a href="#project-status"><img src="https://img.shields.io/badge/status-pre--alpha-orange" alt="Status: pre-alpha"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue" alt="License: GPL-3.0-or-later"></a>
</p>

<p align="center">
  <a href="#getting-started">Get started</a> ·
  <a href="docs/architecture.md">Architecture</a> ·
  <a href="docs/roadmap.md">Roadmap</a> ·
  <a href="CONTRIBUTING.md">Contribute</a> ·
  <a href="README.zh-CN.md">简体中文</a>
</p>

---

**SlateOS is an open-source, agent-native Linux desktop built on NixOS.** It brings natural-language interaction, background computer use, and task-level controls into one system. Your agent can work across the terminal, desktop applications, and system configuration using the same environment you do.

The central idea is simple: **an agent should have its own place on your desktop.** Slate gives it a dedicated Wayland input seat, connects it to the command line and applications, and provides infrastructure for approvals, audit records, and filesystem recovery. You choose the agent backend; Slate provides the environment in which it works.

The repository contains both the Slate runtime and the SlateOS desktop configuration. You can start with the conversational shell or deploy the integrated desktop.

> **Project status:** Pre-alpha · current version **0.0.11**. The core workflow runs end to end on the development system. Hardware and application compatibility are still being expanded; an installable ISO is planned. See [project status](#project-status) for current boundaries.

## Why SlateOS

### A shared desktop, independent input

The agent gets its own pointer, keyboard, and focus through a dedicated Wayland seat. In compatible applications, it can work while you continue typing elsewhere. It sees applications through the accessibility tree first (elements by role and name, text as text, actions without a pointer) and through per-window screenshots second; focus-verified typing lets it inspect the result of an action.

For applications that require the user's seat, Slate provides an explicit fallback: the panel displays **controlling**, and **Esc** takes control back. Application support and backend approval differences are documented [below](#project-status).

### Your agent, integrated into the system

Slate launches the official **Claude Code** or **Codex** CLI and integrates through their headless interfaces and available extension points. Authentication and model access remain with your chosen backend; Slate does not call model APIs directly. Switch backends from the shell or Settings → AI.

### A shell for both intent and commands

`slash` accepts natural-language tasks, built-in commands, and ordinary shell commands in the same interface. Your bash or zsh remains available, and output from manual commands becomes context for the next agent turn. The desktop prompt uses the same runtime through `slash --serve`.

### Controls that live alongside execution

`slated` provides task records, configurable approval tiers, persistent memories, and btrfs-based file recovery. OS Skills describe how to perform common system tasks, favoring CLI tools, D-Bus, and configuration files before GUI interaction. The current level of enforcement depends on the backend; see [execution and recovery](#execution-and-recovery).

## The workflow

Open the desktop prompt with **Super+s**, or start `slash` in a terminal. Example requests:

```text
What is using the most disk space in this directory?
Open the report in Downloads with Firefox.
Arrange Firefox and the terminal side by side.
Set the volume to 30%.
```

In the terminal, the input format determines how a line is handled:

| Input | Behavior |
| --- | --- |
| Plain text | Send a task to the active agent. |
| `!git status` | Run a command in your underlying shell; retain its output as context. |
| `/help` | Show shell commands. |
| `/agent codex` | Switch to the Codex backend (`/agent claude` switches back). |
| `/model` | Inspect or change the backend model. |
| `/audit` | Inspect recent audit records from `slated`. |
| `/undo --preview` | Preview file recovery for the most recent undoable task. |
| `/undo` | Apply that recovery plan. |

The desktop prompt streams activity, presents supported approval requests as buttons, and shows results without taking keyboard focus. Settings (**Super+,**) covers display, sound, network, memories, and agent preferences.

## Getting started

### Try the shell from source

To explore the conversational interface, use Linux or macOS with a stable Rust toolchain, a C toolchain/linker, and an installed, authenticated Claude Code or Codex CLI on `PATH`.

```sh
git clone https://github.com/BenchGrid-dev/slate.git
cd slate
cargo build --workspace
./target/debug/slash
```

The default backend is Claude Code. If you use Codex, enter `/agent codex` before your first task. Type `/help` to explore, or `/exit` to leave.

This runs the shell without changing your login shell or desktop. `slash` starts the companion `slated` daemon when available. Desktop control requires a compatible Linux Wayland session; file recovery requires the btrfs setup described [below](#execution-and-recovery).

For backend options and shell behavior, see the [slash reference](crates/slash/README.md). On Linux with Nix, `nix develop` provides the repository's development environment, and `nix build` builds the runtime package.

### Run the SlateOS desktop

The integrated desktop is distributed as a **NixOS flake module**, with package outputs for `x86_64-linux` and `aarch64-linux`. Use an existing NixOS machine or VM.

Add the Slate input to your system's existing `flake.nix`:

```nix
inputs.slate.url = "github:BenchGrid-dev/slate";
```

Include `slate` in your `outputs` arguments and add the following entries to the `modules` list of your existing `nixosSystem` configuration, keeping your hardware and system modules:

```nix
slate.nixosModules.default
{
  services.slate = {
    enable = true;
    desktop.enable = true;
    loginShellUsers = [ "alice" ]; # Replace with an existing user.
  };
}
```

Rebuild using your usual NixOS workflow and log in to the sway session. This enables the SlateOS desktop profile, installs the runtime and default Claude Code backend, starts the user services, and sets `slash` as the selected user's login shell. Sign in to the backend from **Settings → AI**, then press **Super+s**.

For Claude Code, install the bundled OS Skills from within `slash`:

```text
!slate skills install
```

Skills are selected according to the tools and distribution available on the machine. Automatic skill installation for Codex is not yet implemented.

See the [NixOS setup guide](distro/README.md) for all module options, agent installation, snapshot roots, and system configuration paths.

## Architecture

Slate separates the user interface, agent backend, execution policy, and desktop control into cooperating processes:

```text
Terminal                         Desktop prompt
   │                                   │
   └────────── slash / slash --serve ───┘
                         │
                Claude Code or Codex
                         │
              Backend integration points
                  ╱               ╲
      slate hooks / MCP       slate-desktop MCP
               │                      │
            slated             Wayland agent seat
      Policy · audit · undo    Capture · input · windows
               │                      │
        User-owned btrfs          sway / wlroots

          NixOS module · desktop profile · OS Skills
```

`slash` also talks directly to `slated` for task lifecycle, memories, and the approval UI. The terminal and desktop use the same runtime and shared daemon services; each starts its own conversation session. Hook-based policy integration is currently implemented for Claude Code.

| Component | Responsibility |
| --- | --- |
| [`slash`](crates/slash) | Conversational shell, backend adapters, event rendering, and the JSON-lines interface for desktop clients. |
| [`slated`](crates/slated) | Per-user daemon for policy, approvals, task records, audit logs, snapshots, and memories. |
| [`slate`](crates/slate) | Management CLI, agent hooks, MCP tools, and OS Skills installation. |
| [`slate-desktop`](crates/slate-desktop) | Wayland seat ownership, window capture, input, window management, and desktop MCP tools. |
| [`slate-proto`](crates/slate-proto) | Shared protocol types for local process communication. |
| [`skills/base`](skills/base) | Thirteen task guides: audio, brightness, displays, windows and applications, networking, services, system configuration, undo, office documents, mail and calendar, PDFs, media playback, text editing. |
| [`distro`](distro) | NixOS module, sway profile, panel, launcher, notifications, the application suite, and Python/GTK4 prompt and settings applications. |

The runtime is written in **Rust**. Local services communicate over Unix sockets; agent-facing tools use **MCP**. See the [architecture document](docs/architecture.md) and [architecture decisions](docs/decisions/README.md) for design rationale and open questions.

## Execution and recovery

With Claude Code and `slated` connected, tool calls are classified into three policy tiers:

| Tier | Default behavior |
| --- | --- |
| **Observe** | Allow read-oriented operations without an approval prompt. |
| **Reversible** | Attempt a task snapshot, then allow the operation. |
| **Confirm** | Request approval before proceeding, unless approval has already been granted or bypassed. |

Rules can be customized in `~/.config/slate/policy.toml`; unknown tools default to Confirm. Tool checks and results are recorded through the Claude Code hooks. Codex currently uses its configured sandbox with task-level records in `slated`; this repository does not yet provide equivalent per-tool approvals, auditing, or automatic snapshots for that backend.

**Undo restores local files within a recorded task's scope.** It requires a successful snapshot of a user-owned btrfs subvolume, normally the home directory or `SLATE_SNAPSHOT_ROOT`. It does not reverse remote actions, messages, or arbitrary application state. Recovery compares scoped directories with the snapshot, so later edits in those directories may also appear in the plan; use `/undo --preview` first.

Tasks can still run when snapshots are unavailable. The current policy layer is not an isolation boundary: a separate agent identity and stronger sandboxing remain on the roadmap. `/auto on` explicitly bypasses Slate approval prompts for the session.

Read the [policy reference](docs/policy.md) and [snapshot design](docs/decisions/0006-privilege-free-snapshots.md) for details.

## Project status

The current development baseline is **NixOS 26.05 with sway 1.12**. The repository includes unit tests and real-desktop end-to-end suites; automated CI runs Rust formatting, Clippy, builds, and unit tests. Desktop and agent integration tests require a configured machine and are run separately.

| Area | Available today | Next steps |
| --- | --- | --- |
| Agent interface | Claude Code and Codex adapters; terminal and desktop prompt; session resume within a running shell. | Conversation history across prompt sessions. |
| Desktop control | Dedicated agent seat, accessibility-tree elements and actions (AT-SPI2), per-window capture, Unicode input, focus verification, window arrangement, and user-seat fallback. | Ghost cursor, compositor seat filtering, broader app coverage (Qt, Chromium, Flatpak). |
| Task controls | Claude Code approval broker and tool audit, task records, memories, btrfs snapshots, scoped undo, and consented root (sudo asks the person through a dialog). | Codex approval integration, agent identity isolation, a polkit agent for GUI privilege requests. |
| Distribution | NixOS module, desktop profile, settings app, and SlateOS system command wrappers. | Installable image, installer, and first-run setup. |

**Application compatibility matters.** The agent seat has been verified with foot, Firefox, and GTK3/Thunar when the seat exists before the application starts. Tested GTK4/libadwaita applications require the user-seat fallback. Qt, Chromium/Electron, and Flatpak coverage is still being investigated. The current desktop integration targets sway and its required Wayland protocols; it is not a drop-in GNOME, KDE, or X11 integration.

The next milestones focus on compatibility, execution boundaries, accessibility, and installation. See the [full roadmap](docs/roadmap.md), [toolkit compatibility notes](crates/slate-desktop/README.md#toolkit-compatibility), and [changelog](CHANGELOG.md).

## Contributing

SlateOS brings together systems programming, desktop design, agent integration, and practical Linux knowledge. Contributions are especially useful in these areas:

- **Compatibility:** test real hardware, display scaling, and application toolkits; report the environment and reproducible behavior.
- **Desktop infrastructure:** improve Wayland input, compositor integration, and AT-SPI2 accessibility.
- **Agent integration:** extend backend support, approval flows, and task context.
- **OS Skills:** document reliable ways to complete everyday tasks, including verification and recovery. No Rust required.
- **Product and distribution:** improve the prompt, settings, NixOS packaging, onboarding, and documentation.

Start with the [contribution guide](CONTRIBUTING.md) and [open issues](https://github.com/BenchGrid-dev/slate/issues). Substantial design changes use the [RFC process](docs/rfcs/README.md).

To run the Rust checks used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

Changes affecting the shell, daemon, or desktop also need the [end-to-end tests](tests/e2e/README.md) on a configured desktop. Community participation follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Documentation

| Guide | Contents |
| --- | --- |
| [NixOS setup](distro/README.md) | Installation, module options, desktop configuration, and system tools. |
| [Shell reference](crates/slash/README.md) | Backend configuration, shell escapes, and session behavior. |
| [Desktop reference](crates/slate-desktop/README.md) | MCP tools, Wayland requirements, and toolkit compatibility. |
| [Policy reference](docs/policy.md) | Approval tiers and rule overrides. |
| [OS Skills](skills/README.md) | Skill format, installation, and contribution conventions. |
| [Architecture](docs/architecture.md) · [Decisions](docs/decisions/README.md) | System design and its rationale. |
| [Roadmap](docs/roadmap.md) · [Changelog](CHANGELOG.md) | Planned work and release history. |

## License

Slate is licensed under **GPL-3.0-or-later**. See [LICENSE](LICENSE).
