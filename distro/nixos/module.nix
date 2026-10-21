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
    # A hidden entry: the dock shows the dialog with a padlock.
    installPhase = ''
      mkdir -p $out/bin $out/share/applications
      cp $src $out/bin/slate-askpass
      chmod +x $out/bin/slate-askpass
      cat > $out/share/applications/dev.benchgrid.slate.Askpass.desktop <<EOF
      [Desktop Entry]
      Type=Application
      Name=Password
      Exec=slate-askpass
      Icon=dialog-password
      NoDisplay=true
      EOF
    '';
    meta.mainProgram = "slate-askpass";
  };
  # The session: sway with the headless backend next to DRM, so it can create the agent's
  # background screen. If that combination fails to start on some hardware, plain sway.
  # SwayFX 0.6, based on sway 1.12: 0.5.x (nixpkgs) is based on 1.11, which lacks
  # ext-foreign-toplevel-image-capture-source, and slate-desktop captures single windows
  # through it. 0.6 needs scenefx 0.5 and wlroots 0.20. Sway 1.12's NixOS patches apply.
  # Ours add minimize, maximize and close buttons to the title bars of floating windows
  # and let windows be maximized (sway ignores maximize requests), and fit a window that
  # opens larger than the space between the menu bar and the dock into it.
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
      ++ lib.optionals finalAttrs.isNixOS [ "${sway112}/sway-config-nixos-paths.patch" ]
      ++ [ ./patches/swayfx-titlebar-buttons.patch ./patches/swayfx-fit-new-windows.patch ];
  });
  swayfx06 = pkgs.sway.override { sway-unwrapped = swayfx06-unwrapped; };

  # swaylock exits when a seat's keyboard has no keymap, and then nothing is locked: the
  # input method's virtual keyboard has none until it first forwards a key, and an
  # agent seat's keyboard none until it first types. The patch ignores such a seat.
  swaylockPatched = pkgs.swaylock-effects.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [ ./patches/swaylock-no-keymap.patch ];
  });

  # Thunar draws the Trash in its side pane from the file, blurred and off-centre on
  # HiDPI screens; the patch gives it a themed icon like the other places.
  thunarPatched = pkgs.thunar-unwrapped.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [ ./patches/thunar-trash-icon.patch ];
  });

  # With effects, SwayFX first; should it not start on this machine (it needs GLES2),
  # plain sway with the same configuration. A start that fails within 20 s counts.
  # The session's services end with it: slateos-session.target, which the session starts,
  # is stopped when sway exits.
  slateosSession = pkgs.writeShellScriptBin "slateos-session" ''
    ended() { systemctl --user stop slateos-session.target 2>/dev/null; }
    try() {
      start=$(date +%s)
      "$@"
      code=$?
      ended
      if [ $code -eq 0 ] || [ $(( $(date +%s) - start )) -ge 20 ]; then exit $code; fi
    }
    export WLR_BACKENDS=drm,libinput,headless
    try ${config.programs.sway.package}/bin/sway "$@"
    ${lib.optionalString (cfg.desktop.enable && cfg.desktop.effects) ''try ${pkgs.sway}/bin/sway "$@"''}
    unset WLR_BACKENDS
    ${pkgs.sway}/bin/sway "$@"
    code=$?
    ended
    exit $code
  '';
  slatePolkitAgent = pkgs.writeShellScript "slate-polkit-agent" ''
    while :; do ${pkgs.soteria}/bin/soteria; sleep 2; done
  '';
  # The supervised services of a session, (re)started when it begins.
  sessionUnits = [ "slate-desktop.service" "slate-panel.service" "slate-dock.service" "slate-apps.service"
    "slate-idle.service" "slate-battery.service" "slate-mirror.service" ]
    ++ lib.optional (inputMethods != [ ]) "slate-input-method.service";
  # The first thing the session runs: the background screen if the backend gave none,
  # the person's focus on a real screen (sway focuses the first output, which can be the
  # headless one, and the first windows would open there), then the display handed to
  # systemd and the supervised desktop services started.
  slateosSessionStart = pkgs.writeShellScriptBin "slateos-session-start" ''
    outputs() { swaymsg -t get_outputs | grep -o '"name": "[^"]*"' | cut -d'"' -f4; }
    outputs | grep -q '^HEADLESS-' || swaymsg create_output
    # Both screens, before anything opens on them: on a slow machine they take a moment
    # (at most 15 s).
    for _ in $(seq 1 60); do
      outputs | grep -q '^HEADLESS-' && outputs | grep -qv '^HEADLESS-' && break
      sleep 0.25
    done
    screen() {
      for o in $(outputs); do
        case "$o" in HEADLESS-*) ;; *) swaymsg focus output "$o"; break ;; esac
      done
      swaymsg workspace number 1
    }
    screen
    # The person's theme, rebuilt from this version's system files (mako, fuzzel, waybar
    # and foot read per-user copies that slate-theme writes).
    ${slateTheme}/bin/slate-theme apply >/dev/null
    systemctl --user import-environment PATH WAYLAND_DISPLAY SWAYSOCK XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_ID
    # graphical-session.target, for the units NixOS starts with every graphical session.
    systemctl --user start slateos-session.target
    # The polkit agent, in the session (see security.polkit); started again should
    # it stop.
    swaymsg exec ${slatePolkitAgent}
    systemctl --user restart ${lib.concatStringsSep " " sessionUnits}
    # The first terminal, exec'd with the focus on the person's workspace (sway opens a
    # window on the workspace focused when its command was started).
    sleep 2
    screen
    swaymsg exec foot
  '';
  # The lock screen: the desktop, blurred and darkened, a large clock; Super+L. No font
  # size: swaylock-effects then sizes the time and the date from the ring, at any scale.
  # `--idle` (the idle timer): input in the first five seconds dismisses it unasked.
  slateLock = pkgs.writeShellScriptBin "slate-lock" ''
    grace=""
    [ "$1" = --idle ] && grace="--grace 5"
    exec ${swaylockPatched}/bin/swaylock --daemonize $grace \
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
  # Away from the computer: the lock after `lock_after` seconds without input on the
  # person's seat (seat0; the agent's activity does not count), the screens off after
  # `screen_off_after`, and the lock before the computer sleeps. Both from
  # ~/.config/slate/idle.json (Settings, Power); 0 turns one off.
  slateIdle = pkgs.writeShellScriptBin "slate-idle" ''
    conf="$HOME/.config/slate/idle.json"
    lock=600 off=900
    if [ -r "$conf" ]; then
      lock=$(${pkgs.jq}/bin/jq -r '.lock_after // 600' "$conf")
      off=$(${pkgs.jq}/bin/jq -r '.screen_off_after // 900' "$conf")
    fi
    set -- -w -S seat0
    [ "$lock" -gt 0 ] 2>/dev/null && set -- "$@" timeout "$lock" 'slate-lock --idle'
    [ "$off" -gt 0 ] 2>/dev/null && set -- "$@" timeout "$off" 'slate-screens off' resume 'slate-screens on'
    exec ${pkgs.swayidle}/bin/swayidle "$@" before-sleep slate-lock after-resume 'slate-screens on' lock slate-lock
  '';
  # The person's screens on or off; the agent's background screen stays on (its apps
  # keep drawing).
  slateScreens = pkgs.writeShellScriptBin "slate-screens" ''
    case "$1" in on|off) ;; *) echo "usage: slate-screens on|off" >&2; exit 2 ;; esac
    for o in $(swaymsg -t get_outputs | ${pkgs.jq}/bin/jq -r '.[] | select(.name | startswith("HEADLESS-") | not) | .name'); do
      swaymsg -q output "$o" power "$1"
    done
  '';
  # Low battery notifications (the menu bar shows the level).
  slateBattery = pkgs.stdenv.mkDerivation {
    pname = "slate-battery";
    version = pkg.version;
    src = ../desktop/slate-battery.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsNoGuiHook pkgs.gobject-introspection ];
    buildInputs = [ pkgs.glib (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/slate-battery
      chmod +x $out/bin/slate-battery
    '';
    meta.mainProgram = "slate-battery";
  };
  # System-wide choices Settings makes (input methods), applied as root with a rebuild.
  slateosSettings = pkgs.stdenv.mkDerivation {
    pname = "slateos-settings";
    version = pkg.version;
    src = ../desktop/slateos-settings.py;
    dontUnpack = true;
    buildInputs = [ pkgs.python3 ];
    installPhase = ''
      mkdir -p $out/bin
      cp $src $out/bin/slateos-settings
      chmod +x $out/bin/slateos-settings
      patchShebangs $out/bin/slateos-settings
    '';
    meta.mainProgram = "slateos-settings";
  };
  # Input methods Settings can install, by the name fcitx5's profile gives them.
  inputMethodAddons = {
    pinyin = pkgs.qt6Packages.fcitx5-chinese-addons;  # Pinyin, Shuangpin, Wubi, Cangjie tables
    rime = pkgs.fcitx5-rime;
    mozc = pkgs.fcitx5-mozc;
    hangul = pkgs.fcitx5-hangul;
  };
  inputMethods = lib.unique cfg.desktop.inputMethods;
  # What Settings chose for the whole system (see settingsFile).
  settingsFromFile =
    if cfg.settingsFile != null && builtins.pathExists cfg.settingsFile
    then lib.importJSON cfg.settingsFile else { };
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
      ++ [ "org.xfce.mousepad.desktop" "io.github.kolunmi.Bazaar.desktop" "slate-settings.desktop" "slate:slate" ];
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

    desktop.inputMethods = lib.mkOption {
      type = lib.types.listOf (lib.types.enum (builtins.attrNames inputMethodAddons));
      default = settingsFromFile.inputMethods or [ ];
      defaultText = lib.literalMD "the `inputMethods` of `settingsFile`, else none";
      example = [ "pinyin" ];
      description = ''
        Input methods for languages typed by composition, through fcitx5 on Wayland's own
        input method protocol: "pinyin" (Chinese: Pinyin, Shuangpin, Wubi, Cangjie),
        "rime" (Chinese with Rime: Traditional, Zhuyin and more), "mozc" (Japanese),
        "hangul" (Korean). None by default; Settings, Keyboard installs them for the
        person. Ctrl+Space switches between the keyboard and the input methods.
      '';
    };

    desktop.autologinUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "With desktop.enable, log this user straight into sway instead of showing a greeter.";
    };

    settingsFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default =
        if !(builtins ? currentSystem) then null
        else if builtins.pathExists "/etc/slateos" then "/etc/slateos/settings.json"
        else "/etc/nixos/settings.json";
      defaultText = lib.literalMD ''
        `/etc/slateos/settings.json` (`/etc/nixos/settings.json` without `/etc/slateos`)
        when the configuration is evaluated impurely, as a `configuration.nix` is; null
        for a flake, which cannot read files outside it
      '';
      description = ''
        The JSON file where Settings (through `slateos-settings`) keeps the system-wide
        choices it makes for the person: today `inputMethods`, the default of
        `desktop.inputMethods`. A value the configuration sets wins. A flake in
        /etc/slateos can set this to `./settings.json`.
      '';
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
        slateLock slateSessionMenu swayosd slateIdle slateScreens slateBattery slateosSettings
        # Advanced network settings (VPN, 802.1X, static addresses), from Settings, Network.
        networkmanagerapplet
        # An app store for Flathub.
        bazaar
        soteria
        waybar fuzzel mako swaybg grim slurp wl-clipboard libnotify
        # The minimal set: every app here exposes an accessibility tree and binds every
        # seat (GTK3), so the agent can work in them without borrowing the user's input.
        firefox (lib.hiPrio thunarPatched) mousepad imv pavucontrol wl-mirror
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
        # Windows that draw their own title bar (GTK4, libadwaita) show the same three
        # buttons as the compositor's, at the right.
        settings."org/gnome/desktop/wm/preferences".button-layout = ":minimize,maximize,close";
      }];
    };
    services.gvfs.enable = lib.mkIf cfg.desktop.enable true;

    # A computer, not only a VM: firmware for Wi-Fi, Bluetooth and graphics chips,
    # compressed swap in memory (small machines run a browser and an agent), and the
    # services behind Settings' Network, Bluetooth, Power and Printers pages.
    hardware.enableRedistributableFirmware = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    zramSwap.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    networking.networkmanager.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    hardware.bluetooth = lib.mkIf cfg.desktop.enable {
      enable = lib.mkDefault true;
      powerOnBoot = lib.mkDefault true;
    };
    services.upower.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    services.power-profiles-daemon.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    # Printing: printers that speak IPP Everywhere (most made in the last decade, on the
    # network or over USB) need no driver; CUPS finds network ones through Avahi itself,
    # so cups-browsed, which listens on the network, stays off. Older printers need a
    # driver in services.printing.drivers.
    services.printing = lib.mkIf cfg.desktop.enable {
      enable = lib.mkDefault true;
      browsed.enable = lib.mkDefault false;
    };
    services.ipp-usb.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    services.avahi = lib.mkIf cfg.desktop.enable {
      enable = lib.mkDefault true;
      nssmdns4 = lib.mkDefault true;
      openFirewall = lib.mkDefault true;
    };
    programs.system-config-printer.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    services.system-config-printer.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    # Asks the person when an application needs a privilege (polkit): a Flatpak installed
    # for every user, a printer added, a disk mounted (sudo has slate-askpass). Started by
    # the session (slateos-session-start): polkit only takes an agent from inside the
    # login session, and a user service runs outside it.
    security.polkit.enable = lib.mkIf cfg.desktop.enable true;

    # Software beyond nixpkgs: Flathub apps (Bazaar, the store; each person's apps go in
    # their own installation, so installing needs no password), AppImages that run when
    # opened, and downloaded Linux programs that expect a usual system (nix-ld).
    services.flatpak.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    programs.appimage = lib.mkIf cfg.desktop.enable {
      enable = lib.mkDefault true;
      binfmt = lib.mkDefault true;
    };
    programs.nix-ld.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);

    # Input methods, when the person installed one (Settings, Keyboard). The Wayland
    # frontend: apps take composed text through the compositor (text-input), each seat
    # on its own, so the agent's typing never goes through the person's Pinyin.
    i18n.inputMethod = lib.mkIf (cfg.desktop.enable && inputMethods != [ ]) {
      enable = true;
      type = "fcitx5";
      fcitx5.waylandFrontend = true;
      fcitx5.addons = map (m: inputMethodAddons.${m}) inputMethods;
      # The keyboard first, then the input methods in the order they were added.
      fcitx5.settings.inputMethod = {
        GroupOrder."0" = "Default";
        "Groups/0" = { Name = "Default"; "Default Layout" = "us"; DefaultIM = lib.head inputMethods; };
        "Groups/0/Items/0" = { Name = "keyboard-us"; Layout = ""; };
      } // lib.listToAttrs (lib.imap1 (i: m:
        lib.nameValuePair "Groups/0/Items/${toString i}" { Name = m; Layout = ""; }) inputMethods);
      # The candidate window follows the appearance and uses the desktop's font.
      fcitx5.settings.addons.classicui.globalSection = {
        Theme = "default";
        DarkTheme = "default-dark";
        UseDarkTheme = "True";
        UseAccentColor = "True";
        Font = "Inter 11";
        MenuFont = "Inter 10";
        TrayFont = "Inter Medium 10";
      };
    };
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
      "slate/system-settings.json".text = builtins.toJSON {
        file = cfg.settingsFile;
        inputMethods = { available = builtins.attrNames inputMethodAddons; enabled = inputMethods; };
      };
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

    # The graphical session, from slateos-session-start until sway exits. It holds
    # graphical-session.target, which our services are part of and which other units
    # (the polkit agent) start with.
    systemd.user.targets.slateos-session = lib.mkIf cfg.desktop.enable {
      description = "SlateOS graphical session";
      bindsTo = [ "graphical-session.target" ];
      wants = [ "graphical-session-pre.target" ];
      after = [ "graphical-session-pre.target" ];
    };

    # Locks the screen when the person is away and before sleep (slate-idle).
    systemd.user.services.slate-idle = lib.mkIf cfg.desktop.enable {
      description = "Lock the screen when idle and before sleep";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      path = [ slateLock slateScreens config.programs.sway.package ];
      serviceConfig = {
        ExecStart = "${slateIdle}/bin/slate-idle";
        Restart = "always";
        RestartSec = 1;
      };
    };

    systemd.user.services.slate-battery = lib.mkIf cfg.desktop.enable {
      description = "Low battery notifications";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      path = [ pkgs.libnotify ];
      serviceConfig = {
        ExecStart = "${slateBattery}/bin/slate-battery";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };

    # The AI workspace's live view of the agent's screen (sway assigns the window there),
    # started with the session once that screen exists, and again should it stop.
    systemd.user.services.slate-mirror = lib.mkIf cfg.desktop.enable {
      description = "Live view of the agent's screen on the AI workspace";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      path = [ config.programs.sway.package ];
      script = ''
        out=$(swaymsg -t get_outputs | ${pkgs.jq}/bin/jq -r '[.[] | select(.name | startswith("HEADLESS-")) | .name][0] // empty')
        [ -n "$out" ] || exit 1
        exec ${pkgs.wl-mirror}/bin/wl-mirror "$out"
      '';
      serviceConfig = {
        Restart = "on-failure";
        RestartSec = 3;
      };
    };

    # The input method, when one is installed. Under sway, which runs no XDG autostart.
    systemd.user.services.slate-input-method = lib.mkIf (cfg.desktop.enable && inputMethods != [ ]) {
      description = "Input method (fcitx5)";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = "${config.i18n.inputMethod.package}/bin/fcitx5 --replace";
        Restart = "on-failure";
        RestartSec = 2;
      };
    };

    # Flathub for each person's own installation: the store installs there, without a
    # password. Needs the network once; tried again at every login until it worked.
    systemd.user.services.slate-flathub = lib.mkIf cfg.desktop.enable {
      description = "Add Flathub to the person's Flatpak installation";
      wantedBy = [ "default.target" ];
      path = [ config.services.flatpak.package ];
      script = ''
        flatpak remote-list --user --columns=name | grep -qx flathub && exit 0
        for _ in 1 2 3 4 5 6; do
          flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo && exit 0
          sleep 20
        done
        exit 1
      '';
      serviceConfig.Type = "oneshot";
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
