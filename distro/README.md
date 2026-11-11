# SlateOS on NixOS

SlateOS is built on NixOS. This directory holds the NixOS module (`nixos/module.nix`), the desktop profile (`desktop/`: sway, waybar, fuzzel, mako, foot, wallpaper), the settings app, the Slate prompt, the password dialog, and the live/installer image (`iso/`). The module turns any NixOS machine into SlateOS; the image installs it on a blank disk.

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

- sway configured as a conventional stacking desktop (floating windows with title bars), a quiet dark theme (Inter, JetBrains Mono, Font Awesome icons, dark GTK/libadwaita via dconf), waybar as the top panel (workspaces, clock, network, sound, the Slate status), `slate-dock` as the dock (pinned apps and open windows), fuzzel, mako, foot. With `desktop.effects` (the default) the session runs SwayFX, built with a SlateOS patch that puts minimize, maximize and close buttons at the right of every title bar; apps that draw their own title bar show the same three. Maximize fills the space between the panel and the dock (a double click on a title bar does too); a minimized window waits in sway's scratchpad until its dock icon brings it back.
- Applications, chosen so that the agent can work in them: each exposes an accessibility tree and has a command-line or D-Bus path for the common tasks. `desktop.apps = "minimal"` installs Firefox, Thunar, Mousepad, imv and pavucontrol; `"full"` (the default) adds LibreOffice, Thunderbird (mail, calendar, contacts), zathura (PDF) and mpv. Each has an OS Skill.
- Accessibility on for every toolkit (the bus enabled, `NO_AT_BRIDGE` unset, the Firefox/Chromium/Qt switches), so `desktop_elements` and friends see GTK3, GTK4, Firefox, LibreOffice and Qt applications.
- `slate-desktop daemon` as a supervised user service started first by sway, so the agent seat predates every app.
- The Slate prompt (`slate-shell`, `Mod+s` or the panel button) and the settings app (`slate-settings [page]`, `Mod+comma`): Network (Wi-Fi networks and their passwords, wired, and `nm-connection-editor` for VPN and the rest), Bluetooth (pairing, with codes to confirm or type), Display, Appearance, Sound, Keyboard (input methods), Power (battery, power mode, when the screen locks and turns off), Printers, Slate and AI. The menu bar's icons open their page.
- What a computer needs beyond a VM: redistributable firmware (Wi-Fi, Bluetooth, graphics), compressed swap in memory (zram), NetworkManager, BlueZ, UPower and power-profiles-daemon, CUPS with Avahi and ipp-usb for driverless printers (cups-browsed stays off), and a polkit agent (soteria) started inside the session. The menu bar shows the battery and Bluetooth when the computer has them; `slate-battery` warns at 10 % and 5 %.
- The screen locks after 10 minutes without input on the person's seat and before the computer sleeps, and turns off after 15 (`slate-idle`: swayidle; Settings, Power changes both, `~/.config/slate/idle.json`). The agent's activity does not count. What the agent may do in windows while the screen is locked is the person's choice (`desktop.agentWhileLocked`; Settings, AI, behind their password): look only (the default), element actions and window management, or everything, through a SwayFX patch (`seat <name> lock_exempt yes`) whose exempt seat reaches the windows behind the lock screen and never the lock screen. They can also let the agent keep the screen awake while it works (`desktop.lockWhileAgentWorks`).
- Chromium-based apps (Chromium, Chrome, Electron) the agent opens in the background connect to a Wayland socket of its own (`wp_security_context_v1`), and a SwayFX patch shows them only the agent's seat: they bind a single seat, so they take the agent's keyboard and pointer instead of ignoring them. The person's input does not reach those windows; apps for the person open with `where: "here"`. The lock is swaylock-effects with a patch: swaylock exits when a seat's keyboard has no keymap yet (an input method's or an agent's virtual keyboard), and then nothing is locked.
- Software beyond nixpkgs: Flatpak with Flathub in each person's own installation (no password to install), Bazaar as the store on the dock, AppImages that run when opened (`programs.appimage.binfmt`), and nix-ld for downloaded Linux programs.
- Input methods on demand (`desktop.inputMethods`; Settings, Keyboard installs them): fcitx5 on Wayland's input method protocol, so each seat has its own state and the agent's typing is never converted by the person's Pinyin.
- greetd: autologin into sway for `desktop.autologinUser`, or tuigreet.

Keys: `Mod+Return` terminal · `Mod+Space` launcher · `Mod+s` Slate prompt · `Mod+grave` / `Mod+0` the AI workspace (a live view of the agent's screen) · `Mod+b` show the agent's background windows · `Mod+w` browser · `Mod+e` files · `Mod+t` editor · `Mod+comma` settings · `Mod+q` close · `Mod+1..5` workspaces · `Mod+f` fullscreen · `Mod+l` lock · `Mod+Escape` lock, log out, restart, shut down · Esc while the panel blinks "controlling" takes your mouse and keyboard back.

## Appearance

`slate-theme dark`, `slate-theme light` and `slate-theme wallpaper PATH` (or Settings → Appearance) switch the whole desktop at once and persist in `~/.config/slate/theme.json`: the colour-scheme and GTK theme keys (which GTK3, libadwaita, foot and the portal follow), sway's window colours and background (`~/.config/slate/sway.d/theme.conf`), waybar's style (`~/.config/waybar/style.css`), mako and fuzzel. The system files under `/etc/xdg` stay untouched.

## The agent's background screen

The session starts sway with the headless backend next to the real display (`slateos-session`) and creates a second output, `HEADLESS-1`, placed far outside the real layout so the cursor cannot reach it; workspace `agent` lives there. Windows the agent opens for a task go to that screen by default, so nothing appears over what you are doing; per-window capture, the accessibility tree and the agent's own pointer all work there. When the agent works on something you are using, it acts on your screen instead. `Mod+b` or the panel's middle click brings background windows over; the panel shows "N in background". If sway cannot start with the headless backend on some hardware, the session falls back to plain sway and the agent works on your screen.

## Root with consent

Agent tool calls run without a terminal, so `sudo` could never ask for a password. The desktop profile sets `SUDO_ASKPASS` (and `SSH_ASKPASS`) to `slate-askpass`, a small dialog: when a task runs `sudo`, the person at the desktop sees "Slate needs your password", types it, and sudo runs the command. The password goes to sudo only; the agent sees the outcome. Every sudo asks again (no tty means no timestamp), so each administrative action is a deliberate answer, on top of Slate's own Confirm approval for the command. sudoers keeps the variable and drops `requiretty`. Cancelling the dialog fails the command.

## Other options

| Option | Default | Meaning |
|---|---|---|
| `fallbackShell` | bash | The POSIX shell slash uses for `!` lines and non-interactive invocations |
| `snapshotRoot` | the user's home | A user-owned btrfs subvolume slated snapshots for undo; set it when home is not one |
| `desktop.apps` | `"full"` | `"full"` or `"minimal"` application set (see above) |
| `desktop.inputMethods` | from `settingsFile`, else none | Any of `"pinyin"`, `"rime"`, `"mozc"`, `"hangul"`; Ctrl+Space switches |
| `desktop.agentWhileLocked` | null (Settings decides; else `"pause"`) | `"pause"`, `"elements"` or `"continue"` (with `effects`): what the agent may do in windows while the screen is locked |
| `desktop.lockWhileAgentWorks` | null (Settings decides; else true) | False: the idle lock and screen-off wait until the agent has not used the screen for a minute |
| `settingsFile` | `/etc/slateos/settings.json` (impure evaluation), else null | Where Settings keeps the system-wide choices it makes (`slateos-settings`); the configuration wins. A flake sets it to `./settings.json` |
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

## The live and installer image

```
nix build .#iso                       # the image for the building machine's architecture
ls result/iso/                        # slateos-<label>-<arch>.iso
```

Both `x86_64-linux` and `aarch64-linux` are defined (`nixosConfigurations.iso-<arch>`); the file is named after the Slate version (`slateos-0.1.1-aarch64-linux.iso`). The boot menu (SlateOS's own GRUB theme) starts **Try or Install SlateOS**, with the usual troubleshooting entries under Options. The image boots into the SlateOS desktop as the live user `slate` (no password, so the screen never locks: `desktop.screenLock = false`), with NetworkManager, the Slate runtime, the minimal application set and the installer. Nothing on disk is touched until you install.

At login a window offers **Try SlateOS** or **Install SlateOS**; Install SlateOS also stays first in the dock. The installer (`slateos-installer`, GTK 4) asks for the disk (whole disks of at least 20 GB, not the one the image runs from; it is erased), the user name, password and computer name, the time zone, the application set (everyday apps, which need the internet, or the minimal set the image carries) and whether to encrypt the disk, shows a summary, asks once more before erasing, then runs `slateos-install` and follows its progress. Its last page restarts the computer; remove the image first (in UTM, clear the CD/DVD drive).

From a terminal, the same installation:

```
sudo slateos-install --disk /dev/nvme0n1 --user alice --host mybox --tz Europe/Berlin [--encrypt] [--apps minimal]
```

It asks for the disk name again, for the user's password and, with `--encrypt`, for a disk passphrase (`--yes`, `--password-hash` and `--passphrase-file` answer them for scripts, as the graphical installer does: the password reaches it as a hash only), then partitions the disk (GPT: a 512 MiB EFI system partition and a btrfs volume, inside LUKS2 with `--encrypt`, with the subvolumes `@`, `@nix`, `@home` and `@home/alice`, so the user's home is its own subvolume and undo works from the first boot), writes `/etc/slateos/configuration.nix` from `iso/template.nix` with the chosen application set (`full` unless `--apps minimal`), copies the Slate source next to it so the installed system evaluates the same package the image already carries, and runs `nixos-install`. An encrypted system asks for the passphrase at every boot, before the desktop logs in; that prompt needs a display the firmware can drive, which UTM's default virtio display is not, so do not encrypt there. The installed system boots into the desktop as that user; sign in to the agent from Settings → AI and press Super+s.

The boot theme's images come from `iso/grub-theme.py` (committed, like the wallpapers); the background, the entry icons (nixpkgs' theme, recoloured) and the fonts (Inter) are added when the image is built.

Tested so far: the aarch64 image, in QEMU on the development system (no hardware acceleration, 4 cores, 3.5 GB), installed through the graphical installer end to end (minimal applications, no encryption; about 40 minutes under emulation), and the installed disk boots into the desktop as its user. Encrypted installation has been run with `slateos-install` from the development system. The x86_64 image is evaluated by CI but has not been booted yet, and no image has been tried on real hardware.

## What the installer does not do yet

- Dual boot.
- Agent sign-in on first boot (Settings → AI does it after login).
- Ship the patched compositor once the seat-filtering and ghost-cursor patches exist (ADR 0007).
