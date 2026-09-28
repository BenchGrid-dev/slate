<p align="center">
  <h1 align="center">SlateOS</h1>
  <p align="center"><b>The Linux desktop you talk to.</b></p>
  <p align="center">
    <a href="#status">Status: pre-alpha, runs end to end</a> ·
    <a href="#try-it">Try it</a> ·
    <a href="docs/architecture.md">Architecture</a> ·
    <a href="docs/roadmap.md">Roadmap</a> ·
    <a href="CONTRIBUTING.md">Contribute</a> ·
    <a href="README.zh-CN.md">中文</a>
  </p>
</p>

---

SlateOS is a Linux distribution where the normal way to use the computer is to say what you want. The shell is a conversation. The agent behind it is the Claude Code or Codex subscription you already pay for, not an API key and not a model running on your laptop. It has its own mouse and keyboard, so it can work in your apps while you keep using them, and every step it takes is audited and can be undone.

Three things make it different from "a chat window on Linux":

- **Your subscription is the brain.** SlateOS launches the official `claude` or `codex` binary and integrates through their documented extension points (hooks, MCP, skills, the permission tool). No model API, no tokens to buy, nothing that breaks the terms of your plan.
- **The agent has its own seat.** On Wayland it gets a second pointer and keyboard, its own focus and its own clipboard. It can drive Firefox, a file manager or a settings page in the background while you type somewhere else. When it does need *your* mouse, the panel blinks and Esc takes it back.
- **Undo is a verb.** Every task starts with a filesystem snapshot. Reversible actions run without asking; destructive ones stop and ask; `/undo` puts things back. Everything is in an audit log.

bash and zsh are still there, one keystroke away. You will just open them less.

## What it looks like

Press `Mod+s` (or click the Slate button in the panel) and a prompt drops down from the top right, one exchange at a time:

```
◆  open the report pdf from Downloads in Firefox and fit it to the width

   open the report pdf from Downloads in Firefox and fit it to the width
   Opened ~/Downloads/report.pdf in Firefox and set the zoom to fit width.
   ▸ desktop_key ctrl+0 → "report.pdf — Mozilla Firefox"      11.2s · 4 turns
```

While that runs you can keep working: the agent's clicks go through its own seat. If it must borrow yours (GTK4 apps only listen to the first seat), the panel shows a blinking **controlling** and Esc hands control back.

The same session is available in any terminal, where the shell is `slash`:

```
~ ❯ what is eating the disk in here?
  ▸ Bash: du -sh * | sort -h | tail
  target/ is 4.1G; everything else is under 50M.
  ✓ 6.2s, 2 turns

~ ❯ !git status                # ! runs one line in your real bash/zsh; the agent sees the output
~ ❯ /undo                      # roll back what the last task changed
~ ❯ /auto on                   # skip approvals for this session (still audited, still undoable)
~ ❯ /agent codex               # switch backends
```

## How it is put together

```
┌──────────────────────────────────────────────────────────────────────┐
│  You                                                                 │
│   ├─ slash in a terminal        ─┐  one session, two views           │
│   └─ the Slate prompt (Mod+s)   ─┘  (slash --serve behind it)        │
├──────────────────────────────────────────────────────────────────────┤
│  slash — the shell                                                   │
│   bare text → agent · /cmd → slash or the agent · !cmd → bash/zsh    │
│   renders the agent's event stream; holds no model credentials       │
├──────────────────────────────────────────────────────────────────────┤
│  Agent backend (bring your own): Claude Code · Codex                 │
│   driven only through headless mode, hooks, MCP, skills,             │
│   the permission-prompt tool                                          │
├──────────────────────────────────────────────────────────────────────┤
│  slated — the daemon                                                 │
│   approval tiers · audit log · btrfs snapshots and undo · memories   │
├──────────────────────────────────────────────────────────────────────┤
│  slate-desktop — background computer use                             │
│   agent seat · virtual pointer and keyboard · per-window capture     │
│   window management · focus-verified typing · takeover indicator     │
├──────────────────────────────────────────────────────────────────────┤
│  sway (wlroots) · NixOS                                              │
└──────────────────────────────────────────────────────────────────────┘
```

The full picture, including what is still open, is in [docs/architecture.md](docs/architecture.md). Settled decisions are in [docs/decisions](docs/decisions).

## Principles

1. **Bring your own agent.** SlateOS never calls a model API. It runs the official `claude` or `codex` binary and stays inside their extension surfaces. Backends are pluggable.
2. **The agent gets its own seat.** Not your mouse, not your focus, not your clipboard. Co-existence is a compositor-level guarantee, not a UX convention.
3. **Prefer the boring path.** CLI, D-Bus and config files before the GUI; OS Skills teach the agent the boring path for every part of the system.
4. **Undo is a first-class verb.** Snapshot first; reversible actions run without asking; destructive ones stop and ask.
5. **Everything is auditable.** Every action is logged with what the agent did and which tier of approval it had.
6. **The escape hatch is always open.** `!` gives you a real shell. `chsh` gives you your old login shell back. Nothing in Slate is load-bearing for the Linux underneath.

## Components

| Path | What it is | State (0.0.10) |
|---|---|---|
| `crates/slash` | The shell: terminal view and `--serve` mode for the desktop prompt. Claude Code and Codex backends. | works; see [crates/slash](crates/slash) |
| `crates/slated` | The daemon: approval tiers and policy file, audit log, btrfs snapshots and undo, memories. | works; agent identity isolation not started |
| `crates/slate` | CLI for the daemon, plus the hook and MCP entry points the agents call. Installs OS Skills. | works |
| `crates/slate-desktop` | The agent seat as a supervised daemon, per-window capture, input, window management, MCP server. | works on sway; see [crates/slate-desktop](crates/slate-desktop) |
| `crates/slate-proto` | Wire types shared by the processes. | works |
| `skills/base` | OS Skills: volume, brightness, Wi-Fi, systemd, display settings, windows, undo, SlateOS system changes. | 8 skills |
| `distro/` | NixOS module and flake, the desktop profile (sway, panel, launcher, notifications, theme), the settings app and the Slate prompt. | installs on any NixOS; no ISO yet |

## Status

**Pre-alpha, but it runs end to end.** As of 2026-09-28 (release 0.0.10) everything below runs on the development machine (NixOS 26.05, sway 1.12) with both Claude Code and Codex, and is covered by the end-to-end suites in `tests/e2e/`:

- **slash**: natural-language shell with `/` and `!`; `!` runs in a pty so the agent sees its output; Claude Code (stream-json, session resume) and Codex (exec --json) backends; streaming answers; `/auto`, `/undo`, `/remember`, `/audit`, `/model`, `/agent`.
- **slated**: Observe / Reversible / Confirm tiers with `policy.toml`, approvals routed through the agent's permission tool to you, audit log, privilege-free btrfs snapshots and `/undo`, memories.
- **slate-desktop**: a long-lived agent seat on Wayland with its own pointer and keyboard, per-window screenshots, Unicode typing, window arrangement, focus-verified typing that reports which window received the input, a user-seat fallback for toolkits that ignore extra seats, and a blinking "controlling" indicator with Esc to take control back.
- **Desktop**: a conventional sway desktop (panel, launcher, notifications, dark theme), the Slate prompt in the top-right corner (one exchange at a time, approvals as buttons, results reappear without stealing your keyboard), and a settings app for display and HiDPI, sound, network, memories, and the AI page (backend, models, sign-in, verbose, bypass approvals).
- **SlateOS on NixOS**: `services.slate.enable` installs everything, makes slash the login shell, runs the daemons as user services, and presents the system as SlateOS.

Not there yet: system changes that need root (the agent cannot ask for your password), a ghost cursor and per-app seat filtering (compositor patches), the accessibility tree as an input path, agent identity isolation, an installable image. See [docs/roadmap.md](docs/roadmap.md).

## Try it

You need a NixOS machine (a VM is fine) and a Claude Code or Codex login. Add the flake module:

```nix
{
  inputs.slate.url = "github:BenchGrid-dev/slate";
  outputs = { nixpkgs, slate, ... }: {
    nixosConfigurations.mybox = nixpkgs.lib.nixosSystem {
      modules = [
        slate.nixosModules.default
        {
          services.slate.enable = true;
          services.slate.loginShellUsers = [ "alice" ];   # alice's shell becomes slash
          services.slate.desktop.enable = true;           # the SlateOS desktop
          services.slate.desktop.autologinUser = "alice";
        }
      ];
    };
  };
}
```

Rebuild, log in, sign in to the agent once (`claude auth login` or `codex login`, also from Settings → AI), then press `Mod+s` or open a terminal. Details, options and what the installer will eventually do: [distro/README.md](distro/README.md).

## Contribute

The mechanism works; what it needs now is exposure to more hardware, more apps and more people. Useful contributions:

- **Run it and report.** Different GPUs, HiDPI setups, toolkits (Qt, Chromium/Electron, Flatpak) and whether the agent seat reaches them.
- **OS Skills.** The correct, boring way to do every common task on a Linux desktop. No Rust needed. See [skills/README.md](skills/README.md).
- **Compositor work.** Seat filtering per app and a ghost cursor for the agent seat (sway patches; see ADR 0007).
- **Accessibility.** AT-SPI2 as an input path so agents act on named controls instead of pixels.
- **The desktop shell.** The prompt and settings app are Python/GTK4 today; a Rust rewrite is planned once the design settles.
- **Agent surfaces.** Claude Code and Codex hooks, MCP, headless modes, permission tools, and the root/polkit story for system changes.

See [CONTRIBUTING.md](CONTRIBUTING.md). Design changes go through [docs/rfcs](docs/rfcs).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
