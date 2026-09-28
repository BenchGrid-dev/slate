---
name: slate-system
description: Install packages, change system settings, rebuild, and roll back on SlateOS (a NixOS-based system) by editing configuration.nix. Use whenever the user wants software installed or a system setting changed. SlateOS or NixOS only.
---

# SlateOS system changes

SlateOS is built on NixOS, so the system is described by `/etc/nixos/configuration.nix` (or a flake). Installing software or changing a setting means editing that file and rebuilding. Never use `nix-env -i` for system software; it bypasses the configuration.

## Try a package without installing it

```
nix shell nixpkgs#ripgrep --command rg --version
```

Tier: observe (nothing persists).

## Install a package system-wide

1. Read the current config: `sudo cat /etc/nixos/configuration.nix` (tier: observe).
2. Add the package to `environment.systemPackages`. Tier: reversible (it is a file edit).
3. Rebuild: `sudo nixos-rebuild switch`. Tier: confirm. Tell the user what will change before asking.

## Undo

Every rebuild is a generation. `sudo nixos-rebuild switch --rollback` returns to the previous one. Tier: confirm. `nixos-rebuild list-generations` shows them (observe).

## Verify

`which <program>` after the rebuild, or `systemctl status <unit>` for services.
