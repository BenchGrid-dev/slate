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
      StartupWMClass=dev.benchgrid.slate.Settings
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
  # The password dialog sudo uses when a tool call (no tty) needs root: the person
  # at the desktop types the password, sudo gets it, the agent sees only the outcome.
  slateAskpass = pkgs.stdenv.mkDerivation {
    pname = "slate-askpass";
    version = pkg.version;
    src = ../desktop/slate-askpass.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 pkgs.gobject-introspection ];
    buildInputs = [ pkgs.gtk4 pkgs.libadwaita (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/slate-askpass
      chmod +x $out/bin/slate-askpass
    '';
    meta.mainProgram = "slate-askpass";
  };
  # The session: sway with the headless backend next to DRM, so it can create the agent's
  # background screen. If that combination fails to start on some hardware, plain sway.
  # SwayFX 0.6, based on sway 1.12: 0.5.x (nixpkgs) is based on 1.11, which lacks
  # ext-foreign-toplevel-image-capture-source, and slate-desktop captures single windows
  # through it. 0.6 needs scenefx 0.5 and wlroots 0.20. Sway 1.12's NixOS patches apply.
  scenefx05 = (pkgs.scenefx.override { wlroots_0_19 = pkgs.wlroots_0_20; }).overrideAttrs (old: {
    version = "0.5";
    buildInputs = old.buildInputs ++ [ pkgs.lcms2 ];  # 0.5 colour-manages its renderer
    src = pkgs.fetchFromGitHub {
      owner = "wlrfx";
      repo = "scenefx";
      tag = "0.5";
      hash = "sha256-vUjLG6eubEhJJVa9LPygIcVmNoHwYbSUTJcWEcbxnU4=";
    };
  });
  swayfx06-unwrapped = (pkgs.swayfx-unwrapped.override {
    wlroots_0_19 = pkgs.wlroots_0_20;
    scenefx = scenefx05;
  }).overrideAttrs (finalAttrs: _: {
    version = "0.6";
    src = pkgs.fetchFromGitHub {
      owner = "WillPower3309";
      repo = "swayfx";
      tag = "0.6";
      hash = "sha256-yvVqwgKEZt/JbT4cxyRA95oK1t/KcZ2AvI5/o7gYa0M=";
    };
    patches =
      let sway112 = "${pkgs.path}/pkgs/by-name/sw/sway-unwrapped";
      in [
        "${sway112}/load-configuration-from-etc.patch"
        (pkgs.replaceVars "${sway112}/fix-paths.patch" { inherit (pkgs) swaybg; })
      ]
      ++ lib.optionals (!finalAttrs.isNixOS) [ "${sway112}/sway-config-no-nix-store-references.patch" ]
      ++ lib.optionals finalAttrs.isNixOS [ "${sway112}/sway-config-nixos-paths.patch" ];
  });
  swayfx06 = pkgs.sway.override { sway-unwrapped = swayfx06-unwrapped; };

  # With effects, SwayFX first; should it not start on this machine (it needs GLES2),
  # plain sway with the same configuration. A start that fails within 20 s counts.
  slateosSession = pkgs.writeShellScriptBin "slateos-session" ''
    try() {
      start=$(date +%s)
      "$@"
      code=$?
      if [ $code -eq 0 ] || [ $(( $(date +%s) - start )) -ge 20 ]; then exit $code; fi
    }
    export WLR_BACKENDS=drm,libinput,headless
    try ${config.programs.sway.package}/bin/sway "$@"
    ${lib.optionalString (cfg.desktop.enable && cfg.desktop.effects) ''try ${pkgs.sway}/bin/sway "$@"''}
    unset WLR_BACKENDS
    exec ${pkgs.sway}/bin/sway "$@"
  '';
  # The first thing the session runs: the background screen if the backend gave none,
  # the person's focus on a real screen (sway focuses the first output, which can be the
  # headless one, and the first windows would open there), then the display handed to
  # systemd and the supervised desktop services started.
  slateosSessionStart = pkgs.writeShellScriptBin "slateos-session-start" ''
    outputs() { swaymsg -t get_outputs | grep -o '"name": "[^"]*"' | cut -d'"' -f4; }
    outputs | grep -q '^HEADLESS-' || swaymsg create_output
    sleep 0.5
    for o in $(outputs); do
      case "$o" in HEADLESS-*) ;; *) swaymsg focus output "$o"; break ;; esac
    done
    swaymsg workspace number 1
    # The person's theme, rebuilt from this version's system files (mako, fuzzel, waybar
    # and foot read per-user copies that slate-theme writes).
    ${slateTheme}/bin/slate-theme apply >/dev/null
    systemctl --user import-environment PATH WAYLAND_DISPLAY SWAYSOCK XDG_CURRENT_DESKTOP XDG_SESSION_TYPE
    systemctl --user restart slate-desktop.service slate-panel.service slate-dock.service slate-apps.service
    # The first terminal, exec'd now that the focus is on the person's workspace (sway
    # opens a window on the workspace focused when its command was started).
    sleep 2
    swaymsg exec foot
  '';
  # The lock screen: the desktop, blurred and darkened, a large clock; Super+L. No font
  # size: swaylock-effects then sizes the time and the date from the ring, at any scale.
  slateLock = pkgs.writeShellScriptBin "slate-lock" ''
    exec ${pkgs.swaylock-effects}/bin/swaylock --daemonize \
      --screenshots --effect-blur 14x4 --effect-vignette 0.35:0.65 --fade-in 0.25 \
      --clock --timestr "%H:%M" --datestr "%A, %-d %B" --font "Inter Display" \
      --indicator --indicator-idle-visible --indicator-radius 120 --indicator-thickness 4 \
      --inside-color 00000040 --inside-clear-color 00000040 --inside-ver-color 00000040 --inside-wrong-color 00000040 \
      --ring-color ffffff33 --ring-clear-color ffffff55 --ring-ver-color 7c9cf5ff --ring-wrong-color e5735aff \
      --key-hl-color 7c9cf5ff --bs-hl-color e5a35aff --line-color 00000000 --line-clear-color 00000000 \
      --line-ver-color 00000000 --line-wrong-color 00000000 --separator-color 00000000 \
      --text-color ffffffee --text-clear-color ffffffee --text-ver-color ffffffee --text-wrong-color ffffffee \
      --text-caps-lock-color ffffffee --show-failed-attempts --ignore-empty-password
  '';
  # The session menu: right click on the panel's diamond, or Super+Escape.
  # Before logging out, restarting or shutting down it asks every window to close, as a
  # person closing them would: apps save their state and ask about unsaved work, and none
  # thinks it crashed. Should a window stay open (it is asking), nothing else happens.
  slateSessionMenu = pkgs.writeShellScriptBin "slate-session-menu" ''
    windows() {
      swaymsg -t get_tree | ${pkgs.jq}/bin/jq '[recurse(.nodes[]?, .floating_nodes[]?) | select(.pid?)] | length'
    }
    close_all() {
      swaymsg -q '[all] kill'
      for _ in $(seq 1 40); do
        [ "$(windows)" = 0 ] && return 0
        sleep 0.25
      done
      ${pkgs.libnotify}/bin/notify-send -a SlateOS "$1 canceled" "A window is still open. Close it, then try again."
      return 1
    }
    choice=$(printf '%s\n' "Lock Screen" "Log Out" "Restart" "Shut Down" \
      | ${pkgs.fuzzel}/bin/fuzzel --dmenu --prompt "" --placeholder "SlateOS" --lines 4 --width 22 --no-icons)
    case "$choice" in
      "Lock Screen") exec slate-lock ;;
      "Log Out") close_all "Log out" && exec swaymsg exit ;;
      "Restart") close_all "Restart" && exec systemctl reboot ;;
      "Shut Down") close_all "Shut down" && exec systemctl poweroff ;;
    esac
  '';
  # The appearance switch (dark / light / wallpaper); plain Python, no toolkit.
  slateTheme = pkgs.stdenv.mkDerivation {
    pname = "slate-theme";
    version = pkg.version;
    src = ../desktop/slate-theme.py;
    dontUnpack = true;
    buildInputs = [ pkgs.python3 ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/slate-theme
      chmod +x $out/bin/slate-theme
      patchShebangs $out/bin/slate-theme
    '';
    meta.mainProgram = "slate-theme";
  };
  # The application list, tidied: developer tools and terminal programs hidden, a few
  # plain names; per user, under ~/.local/share/applications (see the script).
  slateApps = pkgs.stdenv.mkDerivation {
    pname = "slate-apps";
    version = pkg.version;
    src = ../desktop/slate-apps.py;
    dontUnpack = true;
    buildInputs = [ pkgs.python3 ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/slate-apps
      chmod +x $out/bin/slate-apps
      patchShebangs $out/bin/slate-apps
    '';
    meta.mainProgram = "slate-apps";
  };
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
      mkdir -p $out/bin $out/share/applications $out/share/icons/hicolor/scalable/apps
      cp $src $out/bin/slate-shell
      chmod +x $out/bin/slate-shell
      cp ${../desktop/icons/slate.svg} $out/share/icons/hicolor/scalable/apps/slate.svg
      # os-release's LOGO (shown by "About" screens and system info tools).
      cp ${../desktop/icons/slate.svg} $out/share/icons/hicolor/scalable/apps/slateos.svg
      cat > $out/share/applications/slate-shell.desktop <<EOF
      [Desktop Entry]
      Type=Application
      Name=Slate
      Comment=Talk to your computer
      Exec=slate-shell
      Icon=slate
      StartupWMClass=dev.benchgrid.slate.Shell
      Categories=Utility;
      EOF
    '';
    meta.mainProgram = "slate-shell";
  };
  # The dock: pinned and open applications, Slate's windows marked; GTK4 + layer-shell.
  slateDock = pkgs.stdenv.mkDerivation {
    pname = "slate-dock";
    version = pkg.version;
    src = ../desktop/slate-dock.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 pkgs.gobject-introspection ];
    buildInputs = [ pkgs.gtk4 pkgs.gtk4-layer-shell (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    preFixup = ''
      gappsWrapperArgs+=(--set LD_PRELOAD "${pkgs.gtk4-layer-shell}/lib/libgtk4-layer-shell.so")
      gappsWrapperArgs+=(--set SLATE_DOCK_ICONS "$out/share/slate-dock")
    '';
    installPhase = ''
      mkdir -p $out/bin $out/share/slate-dock
      cp $src $out/bin/slate-dock
      chmod +x $out/bin/slate-dock
      cp ${../desktop/icons/slate-launcher.svg} $out/share/slate-dock/slate-launcher.svg
      cp ${../desktop/icons/slate.svg} $out/share/slate-dock/slate.svg
    '';
    meta.mainProgram = "slate-dock";
  };
  dockDefaults = pkgs.writeText "dock.json" (builtins.toJSON {
    pinned = [ "slate:launcher" "thunar.desktop" "firefox.desktop" "foot.desktop" ]
      ++ lib.optionals (cfg.desktop.apps == "full") [ "thunderbird.desktop" "writer.desktop" ]
      ++ [ "org.xfce.mousepad.desktop" "slate-settings.desktop" "slate:slate" ];
  });
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

    desktop.apps = lib.mkOption {
      type = lib.types.enum [ "full" "minimal" ];
      default = "full";
      description = ''
        Which applications the desktop profile installs. "minimal" is the shell of a
        desktop: terminal, browser, file manager, text editor, image viewer, volume
        control. "full" adds the everyday suite chosen for agent-friendliness (an
        accessibility tree and a command-line or D-Bus path for the common tasks):
        LibreOffice, Thunderbird (mail, calendar, contacts), zathura (PDF), mpv.
      '';
    };

    desktop.effects = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run the desktop on SwayFX, sway with rounded corners, shadows and frosted
        translucency (same configuration and IPC, same protocols for the agent seat).
        Off: plain sway, square and flat. Translucency can also be turned off per user
        in Settings, Appearance (for machines without a GPU, where blur costs CPU).
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
    system.nixos.vendorName = "BenchGrid";
    system.nixos.vendorId = "benchgrid";
    # The base release stays in VERSION and VERSION_ID (NixOS tools read them); what a
    # person sees names SlateOS and its own version, with SlateOS's logo and links.
    system.nixos.extraOSReleaseArgs = {
      PRETTY_NAME = "SlateOS ${pkg.version} (${config.system.nixos.release})";
      LOGO = "slateos";
      HOME_URL = "https://slate.benchgrid.dev";
      DOCUMENTATION_URL = "https://github.com/BenchGrid-dev/slate#readme";
      SUPPORT_URL = "https://github.com/BenchGrid-dev/slate/issues";
      BUG_REPORT_URL = "https://github.com/BenchGrid-dev/slate/issues";
    };
    networking.hostName = lib.mkDefault "slateos";

    environment.systemPackages = [ pkg slateosTools pkgs.btrfs-progs ] ++ cfg.agents
      ++ lib.optionals cfg.desktop.enable (with pkgs; [
        slateSettings slateShell slateDock slateAskpass slateosSession slateosSessionStart slateTheme slateApps dconf
        slateLock slateSessionMenu swayosd
        waybar fuzzel mako swaybg grim slurp wl-clipboard libnotify
        # The minimal set: every app here exposes an accessibility tree and binds every
        # seat (GTK3), so the agent can work in them without borrowing the user's input.
        firefox thunar mousepad imv pavucontrol wl-mirror
        # The look: GTK3 apps drawn like libadwaita ones, rounded app icons, a clear cursor.
        adw-gtk3 colloid-icon-theme bibata-cursors adwaita-icon-theme gnome-themes-extra
      ])
      ++ lib.optionals (cfg.desktop.enable && cfg.desktop.apps == "full") (with pkgs; [
        libreoffice thunderbird zathura mpv
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
        XCURSOR_THEME = "Bibata-Modern-Classic";
        XCURSOR_SIZE = "24";
        QT_QPA_PLATFORMTHEME = "gtk3";
        # Every toolkit publishes its widget tree on the accessibility bus, which is
        # how agents see and drive applications natively (slate-desktop's a11y tools).
        QT_LINUX_ACCESSIBILITY_ALWAYS_ON = "1";
        GNOME_ACCESSIBILITY = "1";
        ACCESSIBILITY_ENABLED = "1";
        # sudo without a terminal (every agent tool call) asks the person through this
        # dialog; SSH_ASKPASS covers ssh and git the same way.
        SUDO_ASKPASS = "${slateAskpass}/bin/slate-askpass";
        SSH_ASKPASS_REQUIRE = "prefer";
      })
    ];

    users.users = lib.genAttrs cfg.loginShellUsers (_: {
      shell = "/run/current-system/sw/bin/slash";
    });

    programs.ssh.askPassword = lib.mkIf cfg.desktop.enable "${slateAskpass}/bin/slate-askpass";
    # Root with consent: sudo reads SUDO_ASKPASS from the caller's environment and, with
    # no tty (an agent's tool call), uses it instead of failing. Each use asks the person.
    security.sudo.extraConfig = lib.mkIf cfg.desktop.enable ''
      Defaults env_keep += "SUDO_ASKPASS"
      Defaults !requiretty
    '';

    programs.sway = lib.mkIf cfg.sway.enable {
      enable = true;
      package = if cfg.desktop.enable && cfg.desktop.effects then swayfx06 else pkgs.sway;
      wrapperFeatures.gtk = true;
      extraPackages = with pkgs; [ foot grim slurp wl-clipboard wlr-randr swaybg ];
    };

    # The desktop profile.
    fonts.packages = lib.mkIf cfg.desktop.enable (with pkgs; [ inter jetbrains-mono font-awesome noto-fonts noto-fonts-cjk-sans noto-fonts-color-emoji dejavu_fonts ]);
    # Greyscale antialiasing and slight hinting: crisp on HiDPI, no colour fringes.
    fonts.fontconfig.hinting.style = lib.mkIf cfg.desktop.enable "slight";
    fonts.fontconfig.subpixel.rgba = lib.mkIf cfg.desktop.enable "none";
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
          gtk-theme = "adw-gtk3-dark";
          icon-theme = "Colloid-Dark";
          cursor-theme = "Bibata-Modern-Classic";
          cursor-size = lib.gvariant.mkInt32 24;
          font-antialiasing = "grayscale";
          font-hinting = "slight";
          font-name = "Inter 10";
          document-font-name = "Inter 11";
          monospace-font-name = "JetBrains Mono 10";
        };
      }];
    };
    services.gvfs.enable = lib.mkIf cfg.desktop.enable true;
    # The lock screen authenticates through PAM.
    security.pam.services.swaylock = lib.mkIf cfg.desktop.enable { };
    # Brightness keys: swayosd's rule lets the video group (the desktop user) set the backlight.
    services.udev.packages = lib.mkIf cfg.desktop.enable [ pkgs.swayosd ];
    xdg.portal = lib.mkIf cfg.desktop.enable {
      enable = true;
      wlr.enable = true;
      extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
    };
    environment.etc = lib.mkIf cfg.desktop.enable {
      "sway/config".source = lib.mkForce ../desktop/sway/config;
      "xdg/waybar/config.jsonc".source = ../desktop/waybar/config.jsonc;
      "xdg/waybar/style.css".source = ../desktop/waybar/style.css;
      "xdg/waybar/style-light.css".source = ../desktop/waybar/style-light.css;
      "xdg/fuzzel/fuzzel.ini".source = ../desktop/fuzzel/fuzzel.ini;
      "xdg/mako/config".source = ../desktop/mako/config;
      "xdg/foot/foot.ini".source = ../desktop/foot/foot.ini;
      "slate/wallpaper-dark.jpg".source = ../desktop/wallpaper-dark.jpg;
      "slate/wallpaper-light.jpg".source = ../desktop/wallpaper-light.jpg;
      "slate/sway.d/effects.conf" = lib.mkIf cfg.desktop.effects { source = ../desktop/sway/effects.conf; };
      "slate/dock.json".source = dockDefaults;
      "slate/swayosd.css".source = ../desktop/swayosd/style.css;
    };
    services.greetd = lib.mkIf cfg.desktop.enable {
      enable = true;
      settings.default_session =
        if cfg.desktop.autologinUser != null then {
          command = "${slateosSession}/bin/slateos-session";
          user = cfg.desktop.autologinUser;
        } else {
          command = "${pkgs.tuigreet}/bin/tuigreet --time --remember --cmd ${slateosSession}/bin/slateos-session";
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

    # The OS Skills the agent reads (~/.claude/skills/slate-*), linked to this version's
    # copy at every login. The command's store path changes with each version, so an
    # upgrade restarts the unit and the links follow; skills that a version no longer
    # ships are removed.
    systemd.user.services.slate-skills = {
      description = "Link the SlateOS skills of this version for the agent";
      wantedBy = [ "default.target" ];
      path = [ pkg "/run/current-system/sw" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${pkg}/bin/slate skills install";
      };
    };

    # The panel and the dock (one waybar, two bars). Supervised: waybar can crash while
    # it reloads its style (slate-theme asks it to), and a session without a panel
    # and a dock has no way back short of logging out.
    systemd.user.services.slate-panel = lib.mkIf cfg.desktop.enable {
      description = "SlateOS panel and dock";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      # The dock starts applications and the panel runs slate: resolve like the user.
      path = [ pkg "/run/wrappers" "/etc/profiles/per-user/%u" "/run/current-system/sw" ];
      serviceConfig = {
        ExecStart = "${pkgs.waybar}/bin/waybar";
        Restart = "always";
        RestartSec = 1;
        # Its clicks start applications through sway (swaymsg exec), outside this
        # unit; should one ever run as a child, a restart must not take it along.
        KillMode = "process";
      };
    };

    # The tidied application list, at login and after every upgrade (the store path in
    # ExecStart changes with each version, so an upgrade re-runs it).
    systemd.user.services.slate-apps = lib.mkIf cfg.desktop.enable {
      description = "Tidy the application list for the launcher and the dock";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${slateApps}/bin/slate-apps";
      };
    };

    # The dock: same arrangement as the panel (restarted if it crashes; what it starts
    # goes through sway, so a restart never takes an application along).
    systemd.user.services.slate-dock = lib.mkIf cfg.desktop.enable {
      description = "SlateOS dock";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      path = [ pkg "/run/wrappers" "/etc/profiles/per-user/%u" "/run/current-system/sw" ];
      serviceConfig = {
        ExecStart = "${slateDock}/bin/slate-dock";
        Restart = "always";
        RestartSec = 1;
        KillMode = "process";
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
