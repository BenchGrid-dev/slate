# SlateOS system configuration. Edit, then `sudo slateos-rebuild switch`.
{ config, pkgs, lib, ... }:
let
  # The Slate runtime and desktop, from the flake pinned at install time.
  slate = builtins.getFlake "path:/etc/slateos/slate";
in
{
  imports = [
    ./hardware-configuration.nix
    slate.nixosModules.default
  ];

  services.slate = {
    enable = true;
    desktop.enable = true;
    desktop.apps = "full";
    desktop.autologinUser = "@USER@";
    loginShellUsers = [ "@USER@" ];
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

  nixpkgs.config.allowUnfree = true; # the agent CLIs
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  system.stateVersion = "26.05";
}
