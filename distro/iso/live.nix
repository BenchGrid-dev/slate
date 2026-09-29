# The SlateOS live and installer image: boots straight into the SlateOS desktop with
# the Slate runtime, an autologged-in "slate" user, NetworkManager, and the installer.
{ self, nixpkgs }:
{ config, lib, pkgs, modulesPath, ... }:
let
  installer = pkgs.writeShellApplication {
    name = "slateos-install";
    runtimeInputs = with pkgs; [ parted btrfs-progs dosfstools util-linux coreutils gnused mkpasswd ];
    text = builtins.readFile ./slateos-install.sh;
  };
in
{
  imports = [
    "${modulesPath}/installer/cd-dvd/installation-cd-base.nix"
    self.nixosModules.default
  ];

  # Identity of the image.
  isoImage.volumeID = "SLATEOS";
  isoImage.isoName = lib.mkForce "slateos-${config.system.nixos.label}-${pkgs.stdenv.hostPlatform.system}.iso";
  isoImage.makeEfiBootable = true;
  isoImage.makeUsbBootable = true;
  isoImage.splashImage = ../desktop/wallpaper.png;
  isoImage.efiSplashImage = ../desktop/wallpaper.png;
  isoImage.appendToMenuLabel = " Live";
  isoImage.squashfsCompression = "zstd -Xcompression-level 6";

  # The desktop, as installed systems get it; the live user needs no password.
  services.slate = {
    enable = true;
    desktop.enable = true;
    desktop.apps = "minimal";
    desktop.autologinUser = "slate";
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

  environment.systemPackages = [ installer pkgs.gparted pkgs.git ];
  # The source the installer copies into the target, so the installed system builds the
  # same Slate package (already in the image's store) without a network fetch.
  environment.etc."slateos/src".source = self;
  environment.etc."slateos/template.nix".source = ./template.nix;

  # The agent CLIs (Claude Code, Codex) are unfree packages.
  nixpkgs.config.allowUnfree = true;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.registry.nixpkgs.flake = nixpkgs;
  nix.registry.slate.flake = self;

}
