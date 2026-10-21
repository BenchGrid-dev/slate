# SlateOS system configuration. Edit, then `sudo slateos-rebuild switch`.
{ config, pkgs, lib, ... }:
let
  # The Slate runtime and desktop, from the flake pinned at install time, next to this
  # file (/etc/slateos/slate; under /mnt while the installer builds the system).
  slate = builtins.getFlake "path:${toString ./slate}";
in
{
  imports = [
    ./hardware-configuration.nix
    slate.nixosModules.default
  ];

  services.slate = {
    enable = true;
    desktop.enable = true;
    desktop.apps = "@APPS@";
    desktop.autologinUser = "@USER@";
    loginShellUsers = [ "@USER@" ];
    # What Settings chooses for the whole system (input methods), next to this file.
    settingsFile = ./settings.json;
  };

  users.users."@USER@" = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
    initialHashedPassword = "@HASH@";
  };

  networking.hostName = "@HOST@";
  networking.networkmanager.enable = true;
  time.timeZone = "@TZ@";
  i18n.defaultLocale = "en_US.UTF-8";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  @LUKS@

  nixpkgs.config.allowUnfree = true; # the agent CLIs
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  system.stateVersion = "26.05";
}
