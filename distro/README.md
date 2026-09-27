# Slate OS

The distribution is not decided yet (ADR 0004 is open, leaning NixOS). What exists today is a NixOS module so the runtime can be installed declaratively on any NixOS machine:

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
        }
      ];
    };
  };
}
```

This installs `slash`, `slated`, `slate` and `slate-desktop`, enables sway with the tools slate-desktop needs, runs `slated` as a per-user systemd service, and installs Claude Code from nixpkgs (Codex is not packaged in nixpkgs yet; `npm i -g @openai/codex` works).

Building the package alone: `nix build github:BenchGrid-dev/slate`.

## What the installer will have to do

- Create every user's home as its own btrfs subvolume, owned by the user (snapshots and undo need it).
- Ship the patched compositor once the seat-filtering and ghost-cursor patches exist (ADR 0007).
- Preinstall the base OS Skills.
