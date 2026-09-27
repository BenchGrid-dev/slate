# NixOS module for Slate. Import via the flake:
#   imports = [ slate.nixosModules.default ];
#   services.slate.enable = true;
#   services.slate.loginShellUsers = [ "alice" ];
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.slate;
  pkg = cfg.package;
in
{
  options.services.slate = {
    enable = lib.mkEnableOption "the Slate agent runtime (slash, slated, slate-desktop)";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.slate;
      description = "The Slate package providing slash, slated, slate and slate-desktop.";
    };

    loginShellUsers = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = ''
        Users whose login shell becomes slash. slash execs the user's fallback shell for
        non-interactive invocations, so tools that run `$SHELL -c` keep working.
      '';
    };

    fallbackShell = lib.mkOption {
      type = lib.types.str;
      default = "${pkgs.zsh}/bin/zsh";
      description = "The POSIX shell slash delegates to for `!` lines and non-interactive use.";
    };

    sway.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable sway with the tools slate-desktop relies on. Slate needs a wlroots-based compositor.";
    };

    agents = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ pkgs.claude-code ];
      description = "Agent backends to install. Slate drives them through their official CLIs only.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ pkg pkgs.btrfs-progs ] ++ cfg.agents;
    environment.shells = [ "${pkg}/bin/slash" ];

    # slash finds its fallback shell through this variable when the user has not set one.
    environment.sessionVariables.SLATE_FALLBACK_SHELL = cfg.fallbackShell;

    users.users = lib.genAttrs cfg.loginShellUsers (_: {
      shell = "${pkg}/bin/slash";
    });

    programs.sway = lib.mkIf cfg.sway.enable {
      enable = true;
      wrapperFeatures.gtk = true;
      extraPackages = with pkgs; [ foot grim slurp wl-clipboard wlr-randr swaybg ];
    };

    # slated per user session: approvals, audit, snapshots, undo.
    systemd.user.services.slated = {
      description = "Slate daemon: approval broker, audit log, snapshots and undo";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        ExecStart = "${pkg}/bin/slated";
        Restart = "on-failure";
        RestartSec = 2;
      };
    };

    # Nothing here touches the filesystem layout. For undo to work, each user's home must
    # be a btrfs subvolume they own (see docs/decisions/0006-privilege-free-snapshots.md);
    # the installer is responsible for that.
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
  };
}
