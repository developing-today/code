{ config, pkgs }:
{
  isNormalUser = true;
  hashedPasswordFile = config.sops.secrets."users/backup/passwordHash".path;
  description = "backup";
  extraGroups = [
    "trusted-users"
    "networkmanager"
    "wheel"
    "docker"
    "video"
    "kvm"
    "beep"
  ];
  packages = with pkgs; [
    firefox
    kate
  ];
}
