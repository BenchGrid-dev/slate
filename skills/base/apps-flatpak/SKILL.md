---
name: slate-apps-flatpak
description: Install, update and remove desktop applications for the person from Flathub with flatpak (WeChat, QQ, VS Code, Chrome, Spotify, Telegram, Zoom, Obsidian and thousands more), and run AppImages or downloaded Linux programs. Use whenever the person wants a desktop app that is not installed. SlateOS only.
---

# Desktop applications

Where an app comes from:

| The app | Get it from |
|---|---|
| A desktop app, especially a proprietary one (chat, office suites, browsers, music, video calls, IDEs) | **Flathub**, for the person (`--user`): no password, no system rebuild, its own updates |
| A command-line tool, a service, a driver, anything system-wide | nixpkgs, through the system configuration (see the slateos-system skill) |
| An `.AppImage` the person downloaded | run it directly: `chmod +x file.AppImage && ./file.AppImage` |
| Another downloaded Linux program | usually runs as is (nix-ld provides the usual libraries); if it does not, look for it on Flathub or in nixpkgs |

## Find

```
flatpak search <words>                       # name, ID, version, branch
flatpak remote-info --user flathub <ID>      # fails if Flathub has no build for this computer's architecture
```

Tier: observe. Some proprietary apps exist only for x86_64 (Chrome, WPS Office, Spotify); on an ARM computer say so and offer an alternative.

## Install

```
flatpak install --user -y --noninteractive flathub <ID>      # e.g. com.tencent.WeChat
```

Tier: confirm. Say what it is and how big the download is (the first app of a kind also downloads its runtime, often several hundred MB). The app appears in the launcher and can be pinned to the dock; start it with `desktop_launch` and `flatpak run <ID>` as the command. The person can also browse Flathub in the Software app (Bazaar) on the dock.

## Use a Flatpak app

The desktop tools work on Flatpak apps as on any other; what they can do depends on the app's toolkit, not on Flatpak:

- GTK, Qt and Electron apps (Flatseal, Obsidian, VS Code) have an accessibility tree: start with `desktop_elements` and `desktop_element_click`. GTK4 and Electron apps ignore the agent seat's keyboard and pointer; where an element action is not enough (typing into an Electron text field), use `seat: "user"`.
- Apps that draw their own interface (WeChat, most games) have no tree: work from window screenshots with `desktop_click` and `desktop_type`, and retry with `seat: "user"` when input has no visible effect.
- An app's files live in `~/.var/app/<ID>`; files elsewhere reach it only through the file chooser or the folders it was granted (`flatpak info --show-permissions <ID>`).

## Update / remove

```
flatpak update --user -y
flatpak list --user --app
flatpak uninstall --user -y <ID>             # its data in ~/.var/app/<ID> stays
```

Tier: update reversible; uninstall confirm.

## Verify

`flatpak list --user --app` shows the ID.
