---
name: slate-system
description: Install packages, change system settings, rebuild, and roll back on SlateOS by editing the system configuration in /etc/slateos. Use whenever the user wants software installed or a system setting changed. SlateOS (or plain NixOS) only.
---

# SlateOS system changes

SlateOS describes the whole system in one place: `/etc/slateos/configuration.nix` (or a flake in `/etc/slateos`). Installing software or changing a system setting means editing that file and rebuilding. SlateOS is built on NixOS, so the format is the NixOS module language; the `nixos-*` tools still exist, but use the `slateos-*` front ends, which know where the configuration lives. Never use `nix-env -i` for system software; it bypasses the configuration.

If `/etc/slateos` does not exist (a plain NixOS machine running Slate), the same commands work on `/etc/nixos`.

## Try a package without installing it

```
nix shell nixpkgs#ripgrep --command rg --version
```

Tier: observe (nothing persists).

## Install a package system-wide

1. Read the current config: `cat /etc/slateos/configuration.nix` (tier: observe).
2. Add the package to `environment.systemPackages`. Tier: reversible (it is a file edit; it may need root, see below).
3. Rebuild: `sudo slateos-rebuild switch`. Tier: confirm. Tell the user what will change before asking.

## Root

Editing `/etc/slateos` and rebuilding need root. Run the command with plain `sudo` (never `sudo -S`, never `echo password |`, never ask for a password in chat). On the SlateOS desktop, sudo opens a password dialog for the person at the screen; they type it, the command runs, and you only see the result. Tell the user before the first sudo of a task what it will run, since the dialog does not show the command. If sudo fails with "no askpass program" or "a terminal is required", there is no desktop session: give the user the exact command to run themselves.

## Undo

Every rebuild is a generation. `sudo slateos-rebuild switch --rollback` returns to the previous one. Tier: confirm. `slateos-rebuild list-generations` shows them (observe).

## Verify

`which <program>` after the rebuild, or `systemctl status <unit>` for services. `slateos-version` prints the running system version.
