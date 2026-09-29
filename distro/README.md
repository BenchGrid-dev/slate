# SlateOS on NixOS

SlateOS is built on NixOS. This directory holds the NixOS module (`nixos/module.nix`), the desktop profile (`desktop/`: sway, waybar, fuzzel, mako, foot, wallpaper), the settings app (`desktop/slate-settings.py`) and the Slate prompt (`desktop/slate-shell.py`). There is no installable image yet; the module turns any NixOS machine into SlateOS.

```nix
# flake.nix of your system
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
          services.slate.desktop.autologinUser = "alice"; # or leave null for a greeter
        }
      ];
    };
  };
}
```

## What `services.slate.enable` does

- Installs `slash`, `slated`, `slate` and `slate-desktop`, `btrfs-progs`, and the agents in `services.slate.agents` (default: Claude Code from nixpkgs; Codex is not in nixpkgs, `npm i -g @openai/codex` works and the settings app finds it in `~/.npm-global/bin`).
- Registers `/run/current-system/sw/bin/slash` as a login shell and sets it for `loginShellUsers`. The stable path means a new terminal picks up an updated slash without logging out.
- Runs `slated` as a per-user systemd service (approvals, audit, snapshots, memories).
- Presents the system as SlateOS: `/etc/os-release` says `NAME=SlateOS`, `ID=slateos`, `ID_LIKE=nixos`; boot entries and the getty greeting follow; the default hostname is `slateos`.
- Adds the `slateos-*` system tools: `slateos-rebuild`, `slateos-option`, `slateos-install`, `slateos-generate-config`, `slateos-enter`, `slateos-version`. They are thin front ends for the `nixos-*` originals that point them at `/etc/slateos` (a `configuration.nix` or a flake there) and fall back to `/etc/nixos` when that directory does not exist. The originals stay installed; scripts and documentation written for NixOS keep working.
- Enables sway with the tools slate-desktop needs.

## What `services.slate.desktop.enable` adds

- sway configured as a conventional stacking desktop (floating windows with title bars), a quiet dark theme (Inter, JetBrains Mono, Font Awesome icons, dark GTK/libadwaita via dconf), waybar with the Slate status module, fuzzel, mako, foot.
- Applications, chosen so that the agent can work in them: each exposes an accessibility tree and has a command-line or D-Bus path for the common tasks. `desktop.apps = "minimal"` installs Firefox, Thunar, Mousepad, imv and pavucontrol; `"full"` (the default) adds LibreOffice, Thunderbird (mail, calendar, contacts), zathura (PDF) and mpv. Each has an OS Skill.
- Accessibility on for every toolkit (the bus enabled, `NO_AT_BRIDGE` unset, the Firefox/Chromium/Qt switches), so `desktop_elements` and friends see GTK3, GTK4, Firefox, LibreOffice and Qt applications.
- `slate-desktop daemon` as a supervised user service started first by sway, so the agent seat predates every app.
- The Slate prompt (`slate-shell`, `Mod+s` or the panel button) and the settings app (`slate-settings`, `Mod+comma`).
- greetd: autologin into sway for `desktop.autologinUser`, or tuigreet.

Keys: `Mod+Return` terminal · `Mod+Space` launcher · `Mod+s` Slate prompt · `Mod+w` browser · `Mod+e` files · `Mod+t` editor · `Mod+comma` settings · `Mod+q` close · `Mod+1..5` workspaces · `Mod+f` fullscreen · Esc while the panel blinks "controlling" takes your mouse and keyboard back.

## Root with consent

Agent tool calls run without a terminal, so `sudo` could never ask for a password. The desktop profile sets `SUDO_ASKPASS` (and `SSH_ASKPASS`) to `slate-askpass`, a small dialog: when a task runs `sudo`, the person at the desktop sees "Slate needs your password", types it, and sudo runs the command. The password goes to sudo only; the agent sees the outcome. Every sudo asks again (no tty means no timestamp), so each administrative action is a deliberate answer, on top of Slate's own Confirm approval for the command. sudoers keeps the variable and drops `requiretty`. Cancelling the dialog fails the command.

## Other options

| Option | Default | Meaning |
|---|---|---|
| `fallbackShell` | bash | The POSIX shell slash uses for `!` lines and non-interactive invocations |
| `snapshotRoot` | the user's home | A user-owned btrfs subvolume slated snapshots for undo; set it when home is not one |
| `desktop.apps` | `"full"` | `"full"` or `"minimal"` application set (see above) |
| `agents` | `[ pkgs.claude-code ]` | Agent CLIs to install |
| `sway.enable` | true | Enable sway with the tools slate-desktop relies on |
| `package` | the flake's `slate` | Override the Slate package |

Per-user settings written by the settings app and by agents (display modes, keys) go to `~/.config/slate/sway.d/*.conf`; slash reads `~/.config/slate/slash.toml`; slated reads `~/.config/slate/policy.toml`.

Building the package alone: `nix build github:BenchGrid-dev/slate`.

## Moving an existing configuration to /etc/slateos

```
sudo mv /etc/nixos /etc/slateos && sudo ln -s /etc/slateos /etc/nixos
```

The symlink keeps `nixos-rebuild` and channel-based tooling working; `slateos-rebuild` uses the new path directly.

## What stays NixOS underneath, and why that is fine

The base system, its modules and the `nixos-*` tools come from nixpkgs, which is MIT-licensed: modifying, renaming and redistributing them is permitted as long as the copyright notice travels with the code (SlateOS consumes nixpkgs as a flake input and does not copy it). Nix itself (LGPL 2.1) is used unmodified. "NixOS" is a trademark of the NixOS Foundation (registered in 2026, policy in preparation); SlateOS uses the name only to say what it is built on, ships no NixOS logos, and does not call itself NixOS. Internal markers such as `/etc/NIXOS`, the `nixos` channel name and `ID_LIKE=nixos` stay, because the tooling depends on them.

## What the installer will have to do

- Create every user's home as its own btrfs subvolume, owned by the user (snapshots and undo need it).
- Sign the user in to their agent on first boot (`claude auth login` / `codex login`).
- Ship the patched compositor once the seat-filtering and ghost-cursor patches exist (ADR 0007).
- Give the agent a consented path to root for system changes (open question in `docs/architecture.md`).
- Preinstall the base OS Skills.
