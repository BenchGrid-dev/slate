# The SlateOS live and installer image: boots straight into the SlateOS desktop with
# the Slate runtime, an autologged-in "slate" user, NetworkManager, and the installer.
{ self, nixpkgs }:
{ config, lib, pkgs, modulesPath, ... }:
let
  installer = pkgs.writeShellApplication {
    name = "slateos-install";
    runtimeInputs = with pkgs; [ parted btrfs-progs dosfstools util-linux coreutils gnused mkpasswd cryptsetup ];
    text = builtins.readFile ./slateos-install.sh;
  };
  # The graphical front end: the welcome (try or install) and the install steps.
  installerApp = pkgs.stdenv.mkDerivation {
    pname = "slateos-installer";
    inherit (config.services.slate.package) version;
    src = ./slateos-installer.py;
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 pkgs.gobject-introspection ];
    buildInputs = [ pkgs.gtk4 pkgs.libadwaita (pkgs.python3.withPackages (ps: [ ps.pygobject3 ])) ];
    installPhase = ''
      mkdir -p $out/bin $out/share/applications $out/lib/slate
      cp ${../desktop/slate_agent.py} $out/lib/slate/slate_agent.py
      substitute $src $out/bin/slateos-installer \
        --subst-var-by lib $out/lib/slate \
        --subst-var-by install ${installer}/bin/slateos-install \
        --subst-var-by mkpasswd ${pkgs.mkpasswd}/bin/mkpasswd \
        --subst-var-by inhibit ${config.systemd.package}/bin/systemd-inhibit
      chmod +x $out/bin/slateos-installer
      cat > $out/share/applications/slateos-installer.desktop <<EOF
      [Desktop Entry]
      Type=Application
      Name=Install SlateOS
      Comment=Put SlateOS on this computer's disk
      Exec=slateos-installer
      Icon=system-software-install
      StartupWMClass=dev.benchgrid.slate.Installer
      Categories=System;
      EOF
    '';
    meta.mainProgram = "slateos-installer";
  };
  # The boot menu's look: our logo, background and colours over the layout of nixpkgs'
  # theme (its entry icons recoloured for the dark background).
  grubTheme = pkgs.runCommand "slateos-grub-theme" {
    nativeBuildInputs = [ pkgs.grub2_efi pkgs.imagemagick ];
  } ''
    mkdir -p $out/icons
    cp ${./grub-theme}/* $out/
    cp ${../desktop/splash.png} $out/background.png
    cp ${pkgs.nixos-grub2-theme}/terminal_*.png $out/
    for f in ${pkgs.nixos-grub2-theme}/icons/*.png; do
      # The icon's alpha as the shape, filled light; GRUB reads 8-bit RGBA only.
      magick "$f" -alpha extract -background '#d9deee' -alpha shape "PNG32:$out/icons/$(basename "$f")"
    done
    for size in 16 22; do
      grub-mkfont -s $size -o $out/inter-$size.pf2 ${pkgs.inter}/share/fonts/truetype/InterVariable.ttf
    done
  '';
  # The system an installation builds, with placeholder answers: template.nix, and the
  # hardware configuration nixos-generate-config writes for an encrypted btrfs disk.
  # It is never booted. Its closure goes on the image (system.extraDependencies below),
  # so that an installation finds there what the installed system is made of and the
  # live system does not happen to run: the boot loader's installer, wpa_supplicant's
  # service, the pieces of an encrypted initrd. What remains to build on the person's
  # machine are the few files with their names in them. Keep it in step with
  # template.nix; the image test's installation without a network notices when it is not.
  installed = import (nixpkgs + "/nixos/lib/eval-config.nix") {
    inherit (pkgs.stdenv.hostPlatform) system;
    modules = [
      self.nixosModules.default
      ({ modulesPath, ... }: {
        imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];
        services.slate = {
          enable = true;
          desktop.enable = true;
          desktop.apps = "minimal";
          desktop.autologinUser = "user";
          loginShellUsers = [ "user" ];
        };
        users.users.user = {
          isNormalUser = true;
          extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
        };
        networking.hostName = "slateos";
        networking.networkmanager.enable = true;
        time.timeZone = "UTC";
        i18n.defaultLocale = "en_US.UTF-8";
        boot.loader.systemd-boot.enable = true;
        boot.loader.efi.canTouchEfiVariables = true;
        boot.initrd.availableKernelModules = [ "xhci_pci" "ahci" "nvme" "usbhid" "usb_storage" "sd_mod" "virtio_pci" "virtio_blk" ];
        boot.initrd.luks.devices.slateos = { device = "/dev/disk/by-label/slateos-luks"; allowDiscards = true; };
        fileSystems."/" = { device = "/dev/mapper/slateos"; fsType = "btrfs"; options = [ "subvol=@" ]; };
        fileSystems."/nix" = { device = "/dev/mapper/slateos"; fsType = "btrfs"; options = [ "subvol=@nix" ]; };
        fileSystems."/home" = { device = "/dev/mapper/slateos"; fsType = "btrfs"; options = [ "subvol=@home" ]; };
        fileSystems."/boot" = { device = "/dev/disk/by-label/BOOT"; fsType = "vfat"; };
        nixpkgs.config.allowUnfree = true;
        nix.settings.experimental-features = [ "nix-command" "flakes" ];
        system.stateVersion = "26.05";
      })
    ];
  };
in
{
  imports = [
    "${modulesPath}/installer/cd-dvd/installation-cd-base.nix"
    self.nixosModules.default
  ];

  # Identity of the image.
  isoImage.volumeID = "SLATEOS";
  # The file name (nixpkgs names images through image.baseName; isoImage.isoName is gone).
  image.baseName = lib.mkForce "slateos-${config.system.nixos.label}-${pkgs.stdenv.hostPlatform.system}";
  isoImage.makeEfiBootable = true;
  isoImage.makeUsbBootable = true;
  isoImage.splashImage = ../desktop/splash.png;
  isoImage.efiSplashImage = ../desktop/splash.png;
  isoImage.grubTheme = grubTheme;
  # The boot menu's first entry reads "Try or Install SlateOS 0.2.2" (the module labels
  # the system with SlateOS's version, which also names the image file): the live
  # desktop then asks which.
  isoImage.prependToMenuLabel = "Try or Install ";
  isoImage.appendToMenuLabel = "";
  # Memtest86+ only exists for x86; elsewhere its entry only led to "file not found".
  boot.loader.grub.memtest86.enable = lib.mkForce pkgs.stdenv.hostPlatform.isx86;
  # No ZFS on the image: SlateOS installs on btrfs, and nixpkgs' installer base would
  # carry ZFS's kernel module (and warn about its import default on 26.05).
  boot.supportedFilesystems.zfs = lib.mkForce false;
  # The text consoles: nixpkgs' installer greeting speaks of a "nixos" account.
  services.getty.helpLine = lib.mkForce ''

    This is the SlateOS live system. Its desktop is on the first screen; to install,
    open Install SlateOS there, or run: sudo slateos-install --help
  '';
  isoImage.squashfsCompression = "zstd -Xcompression-level 6";

  # The desktop, as installed systems get it; the live user needs no password.
  services.slate = {
    enable = true;
    desktop.enable = true;
    desktop.apps = "minimal";
    desktop.autologinUser = "slate";
    # The live user has no password, and the lock screen takes no empty one: a lock
    # (ten minutes away, Super+L) would shut the person out of the session for good.
    desktop.screenLock = false;
    # The installer asks how the agent connects; the live session itself needs no welcome.
    desktop.welcome = false;
    loginShellUsers = [ "slate" ];
  };
  users.users.slate = {
    isNormalUser = true;
    description = "SlateOS live user";
    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
    initialHashedPassword = "";
  };
  users.users.root.initialHashedPassword = "";
  security.sudo.wheelNeedsPassword = false;
  services.getty.autologinUser = lib.mkForce null;

  networking.networkmanager.enable = true;
  networking.wireless.enable = lib.mkForce false;
  networking.hostName = "slateos-live";

  # hiPrio: the system tools have a slateos-install too (the front end for nixos-install
  # on an already prepared /mnt); on the image the name means this disk installer.
  environment.systemPackages = [ (lib.hiPrio installer) installerApp pkgs.gparted pkgs.git ];
  # Like other live systems: at login, a window offers to try SlateOS or install it, and
  # the dock keeps Install SlateOS at hand.
  environment.etc."slate/sway.d/live-installer.conf".text = ''
    exec sleep 4 && slateos-installer --welcome
  '';
  services.slate.desktop.dockFirst = [ "slateos-installer.desktop" ];
  # The source the installer copies into the target, so the installed system builds the
  # same Slate package (already in the image's store) without a network fetch.
  environment.etc."slateos/src".source = self;
  # And the nixpkgs that source is locked to: the new system's configuration is a flake
  # evaluation, which wants this exact tree in the store. Without it the "Minimal, no
  # internet needed" installation fetched it from cache.nixos.org and failed offline
  # (the registry's nixpkgs, below, is the installer's own copy: another store path).
  system.extraDependencies = [ nixpkgs.outPath installed.config.system.build.toplevel ]
    # And what the files that remain are made with. For the large ones, everything
    # their counterparts in that system were built from (install-info for the system
    # path's info directory, the initrd's tools, …); for the small ones, the tools a
    # dry run of an installation asked a binary cache for.
    ++ (with installed.config.system; [ build.toplevel.inputDerivation path.inputDerivation build.etc.inputDerivation build.initialRamdisk.inputDerivation ])
    ++ (with pkgs; [ lndir makeShellWrapper findXMLCatalogs kmod.dev nukeReferences remarshal libxslt.bin ]);
  environment.etc."slateos/template.nix".source = ./template.nix;

  # The agent CLIs (Claude Code, Codex) are unfree packages.
  nixpkgs.config.allowUnfree = true;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.registry.nixpkgs.flake = nixpkgs;
  nix.registry.slate.flake = self;

}
