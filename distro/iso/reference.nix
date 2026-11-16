# The system an installation builds, with placeholder answers: template.nix, and the
# hardware configuration nixos-generate-config writes for an encrypted btrfs disk. It
# stands in for "an installed SlateOS" where no person's computer is at hand: the image
# carries its closure so that installations copy instead of build, and a release puts
# what cache.nixos.org lacks of it into SlateOS's binary cache so that updates do
# (.github/workflows/release.yml). Keep it in step with template.nix.
{ self, nixpkgs }:
{ system, apps ? "minimal" }:
import (nixpkgs + "/nixos/lib/eval-config.nix") {
  inherit system;
  modules = [
    self.nixosModules.default
    ({ modulesPath, ... }: {
      imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];
      services.slate = {
        enable = true;
        desktop.enable = true;
        desktop.apps = apps;
        desktop.autologinUser = "user";
        loginShellUsers = [ "user" ];
        update.enable = true;
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
}
