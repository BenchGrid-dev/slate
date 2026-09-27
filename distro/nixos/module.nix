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

    desktop.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        The Slate desktop profile: sway configured as a conventional stacking desktop
        (floating windows with title bars), waybar panel, fuzzel launcher, mako
        notifications, wallpaper, fonts and icons, Firefox, Thunar, a text editor,
        and a graphical login into sway.
      '';
    };

    desktop.autologinUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "With desktop.enable, log this user straight into sway instead of showing a greeter.";
    };

    snapshotRoot = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/home/alice";
      description = ''
        A user-owned btrfs subvolume slated snapshots for undo. Defaults to the user's home,
        which must itself be a subvolume owned by the user. Set this when home is not one.
      '';
    };

    agents = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ pkgs.claude-code ];
      description = "Agent backends to install. Slate drives them through their official CLIs only.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ pkg pkgs.btrfs-progs ] ++ cfg.agents
      ++ lib.optionals cfg.desktop.enable (with pkgs; [
        waybar fuzzel mako swaybg grim slurp wl-clipboard
        firefox xfce.thunar gnome-text-editor loupe pavucontrol
        papirus-icon-theme adwaita-icon-theme
      ]);
    environment.shells = [ "${pkg}/bin/slash" ];
    environment.pathsToLink = [ "/share/slate" ];

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

    # The desktop profile.
    fonts.packages = lib.mkIf cfg.desktop.enable (with pkgs; [ noto-fonts noto-fonts-cjk-sans noto-fonts-color-emoji dejavu_fonts ]);
    programs.dconf.enable = lib.mkIf cfg.desktop.enable true;
    services.gvfs.enable = lib.mkIf cfg.desktop.enable true;
    xdg.portal = lib.mkIf cfg.desktop.enable {
      enable = true;
      wlr.enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
    };
    environment.etc = lib.mkIf cfg.desktop.enable {
      "sway/config".source = lib.mkForce ../desktop/sway/config;
      "xdg/waybar/config.jsonc".source = ../desktop/waybar/config.jsonc;
      "xdg/waybar/style.css".source = ../desktop/waybar/style.css;
      "xdg/fuzzel/fuzzel.ini".source = ../desktop/fuzzel/fuzzel.ini;
      "xdg/mako/config".source = ../desktop/mako/config;
      "slate/wallpaper.png".source = ../desktop/wallpaper.png;
    };
    services.greetd = lib.mkIf cfg.desktop.enable {
      enable = true;
      settings.default_session =
        if cfg.desktop.autologinUser != null then {
          command = "${pkgs.sway}/bin/sway";
          user = cfg.desktop.autologinUser;
        } else {
          command = "${pkgs.tuigreet}/bin/tuigreet --time --remember --cmd ${pkgs.sway}/bin/sway";
          user = "greeter";
        };
    };

    # slated per user session: approvals, audit, snapshots, undo.
    systemd.user.services.slated = {
      description = "Slate daemon: approval broker, audit log, snapshots and undo";
      wantedBy = [ "default.target" ];
      environment = lib.mkIf (cfg.snapshotRoot != null) { SLATE_SNAPSHOT_ROOT = cfg.snapshotRoot; };
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
