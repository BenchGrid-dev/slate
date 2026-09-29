# NixOS module for Slate. Import via the flake:
#   imports = [ slate.nixosModules.default ];
#   services.slate.enable = true;
#   services.slate.loginShellUsers = [ "alice" ];
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.slate;
  pkg = cfg.package;
  # The settings app: Python + GTK4 + libadwaita, no build step, quick to iterate on.
  slateSettings = pkgs.stdenv.mkDerivation {
    pname = "slate-settings";
    version = pkg.version;
    src = ../desktop/slate-settings.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 pkgs.gobject-introspection ];
    buildInputs = [ pkgs.gtk4 pkgs.libadwaita (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    installPhase = ''
      mkdir -p $out/bin $out/share/applications
      cp $src $out/bin/slate-settings
      chmod +x $out/bin/slate-settings
      cat > $out/share/applications/slate-settings.desktop <<EOF
      [Desktop Entry]
      Type=Application
      Name=Settings
      Comment=Slate desktop settings
      Exec=slate-settings
      Icon=preferences-system
      Categories=Settings;System;
      EOF
    '';
    meta.mainProgram = "slate-settings";
  };
  # SlateOS names for the system tools. The nixos-* originals stay (ID_LIKE=nixos, and
  # scripts expect them); these front ends add the SlateOS configuration directory,
  # /etc/slateos, and fall back to /etc/nixos when it does not exist.
  slateosTools = pkgs.runCommand "slateos-tools" { } ''
    mkdir -p $out/bin
    cat > $out/bin/slateos-config-args <<'EOF'
    #!/bin/sh
    # Prints the arguments that point a nixos-* tool at the SlateOS configuration.
    cfg=/etc/slateos
    if [ -e "$cfg/flake.nix" ]; then echo --flake; echo "$cfg"
    elif [ -e "$cfg/configuration.nix" ]; then echo -I; echo "nixos-config=$cfg/configuration.nix"
    fi
    EOF
    for tool in rebuild option install; do
      cat > $out/bin/slateos-$tool <<EOF
    #!/bin/sh
    # SlateOS front end for nixos-$tool: the system configuration lives in /etc/slateos.
    case " \$* " in *" --flake "*|*" -I "*|*" --file "*|*" -f "*) exec nixos-$tool "\$@" ;; esac
    set -- \$(slateos-config-args) "\$@"
    exec nixos-$tool "\$@"
    EOF
    done
    cat > $out/bin/slateos-generate-config <<'EOF'
    #!/bin/sh
    case " $* " in *" --dir "*) exec nixos-generate-config "$@" ;; esac
    exec nixos-generate-config --dir /etc/slateos "$@"
    EOF
    cat > $out/bin/slateos-version <<'EOF'
    #!/bin/sh
    if [ $# -eq 0 ]; then echo "SlateOS $(nixos-version)"; else exec nixos-version "$@"; fi
    EOF
    cat > $out/bin/slateos-enter <<'EOF'
    #!/bin/sh
    exec nixos-enter "$@"
    EOF
    chmod +x $out/bin/*
  '';
  # The floating Slate panel: layer-shell window driving `slash --serve`.
  slateShell = pkgs.stdenv.mkDerivation {
    pname = "slate-shell";
    version = pkg.version;
    src = ../desktop/slate-shell.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 pkgs.gobject-introspection ];
    buildInputs = [ pkgs.gtk4 pkgs.libadwaita pkgs.gtk4-layer-shell (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    # gtk4-layer-shell must be loaded before GTK opens the display.
    preFixup = ''
      gappsWrapperArgs+=(--set LD_PRELOAD "${pkgs.gtk4-layer-shell}/lib/libgtk4-layer-shell.so")
    '';
    installPhase = ''
      mkdir -p $out/bin $out/share/applications
      cp $src $out/bin/slate-shell
      chmod +x $out/bin/slate-shell
      cat > $out/share/applications/slate-shell.desktop <<EOF
      [Desktop Entry]
      Type=Application
      Name=Slate
      Comment=Talk to your computer
      Exec=slate-shell
      Icon=starred
      Categories=Utility;
      EOF
    '';
    meta.mainProgram = "slate-shell";
  };
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
      default = "${pkgs.bashInteractive}/bin/bash";
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
    # The system presents itself as SlateOS. It stays NixOS underneath: os-release keeps
    # ID_LIKE=nixos, and nixos-rebuild and friends are unchanged.
    system.nixos.distroName = "SlateOS";
    system.nixos.distroId = "slateos";
    networking.hostName = lib.mkDefault "slateos";

    environment.systemPackages = [ pkg slateosTools pkgs.btrfs-progs ] ++ cfg.agents
      ++ lib.optionals cfg.desktop.enable (with pkgs; [
        slateSettings slateShell
        waybar fuzzel mako swaybg grim slurp wl-clipboard libnotify
        firefox thunar gnome-text-editor loupe pavucontrol
        papirus-icon-theme adwaita-icon-theme gnome-themes-extra
      ]);
    # A stable path, not the store path: a logged-in session keeps $SHELL from login
    # time, and /run/current-system always resolves to the current build, so new
    # terminals pick up an updated slash without re-login.
    environment.shells = [ "/run/current-system/sw/bin/slash" "${pkg}/bin/slash" ];
    environment.pathsToLink = [ "/share/slate" ];

    # slash finds its fallback shell through this variable when the user has not set one.
    environment.sessionVariables = lib.mkMerge [
      { SLATE_FALLBACK_SHELL = cfg.fallbackShell; }
      (lib.mkIf cfg.desktop.enable {
        XCURSOR_THEME = "Adwaita";
        XCURSOR_SIZE = "24";
        QT_QPA_PLATFORMTHEME = "gtk3";
        # Every toolkit publishes its widget tree on the accessibility bus, which is
        # how agents see and drive applications natively (slate-desktop's a11y tools).
        QT_LINUX_ACCESSIBILITY_ALWAYS_ON = "1";
        GNOME_ACCESSIBILITY = "1";
        ACCESSIBILITY_ENABLED = "1";
      })
    ];

    users.users = lib.genAttrs cfg.loginShellUsers (_: {
      shell = "/run/current-system/sw/bin/slash";
    });

    programs.sway = lib.mkIf cfg.sway.enable {
      enable = true;
      wrapperFeatures.gtk = true;
      extraPackages = with pkgs; [ foot grim slurp wl-clipboard wlr-randr swaybg ];
    };

    # The desktop profile.
    fonts.packages = lib.mkIf cfg.desktop.enable (with pkgs; [ inter jetbrains-mono font-awesome noto-fonts noto-fonts-cjk-sans noto-fonts-color-emoji dejavu_fonts ]);
    fonts.fontconfig.defaultFonts = lib.mkIf cfg.desktop.enable {
      sansSerif = [ "Inter" "Noto Sans" "Noto Sans CJK SC" ];
      monospace = [ "JetBrains Mono" "Noto Sans Mono CJK SC" ];
      emoji = [ "Noto Color Emoji" ];
    };
    # Dark, one accent, everywhere: GTK3 reads these from dconf, GTK4/libadwaita through the portal.
    services.gnome.at-spi2-core.enable = lib.mkIf cfg.desktop.enable true;
    programs.dconf = lib.mkIf cfg.desktop.enable {
      enable = true;
      profiles.user.databases = [{
        settings."org/gnome/desktop/interface" = {
          toolkit-accessibility = true;
          color-scheme = "prefer-dark";
          gtk-theme = "Adwaita-dark";
          icon-theme = "Papirus-Dark";
          cursor-theme = "Adwaita";
          font-name = "Inter 10";
          document-font-name = "Inter 11";
          monospace-font-name = "JetBrains Mono 10";
        };
      }];
    };
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
      "xdg/foot/foot.ini".source = ../desktop/foot/foot.ini;
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
      path = [ pkg pkgs.btrfs-progs pkgs.libnotify pkgs.coreutils ];
      environment = lib.mkIf (cfg.snapshotRoot != null) { SLATE_SNAPSHOT_ROOT = cfg.snapshotRoot; };
      serviceConfig = {
        ExecStart = "${pkg}/bin/slated";
        Restart = "always";
        RestartSec = 1;
      };
    };

    # The agent seat owner. Started by the compositor (it needs WAYLAND_DISPLAY),
    # supervised by systemd so a crash never leaves the session without a seat.
    systemd.user.services.slate-desktop = lib.mkIf cfg.desktop.enable {
      description = "Slate desktop daemon: the agent's own Wayland seat";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      # NixOS gives every unit a minimal PATH; apps the agent launches must resolve
      # like the user's own, so put the system and per-user profiles on it.
      path = [ pkg "/run/wrappers" "/etc/profiles/per-user/%u" "/run/current-system/sw" ];
      serviceConfig = {
        ExecStart = "${pkg}/bin/slate-desktop daemon";
        Restart = "always";
        RestartSec = 1;
      };
    };

    # Nothing here touches the filesystem layout. For undo to work, each user's home must
    # be a btrfs subvolume they own (see docs/decisions/0006-privilege-free-snapshots.md);
    # the installer is responsible for that.
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
  };
}
