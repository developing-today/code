{ pkgs, config }:
{
  isNormalUser = true;
  hashedPasswordFile = config.sops.secrets."users/user/passwordHash".path;
  description = "user";
  extraGroups = [
    "trusted-users"
    "networkmanager"
    "wheel"
    "docker"
    "video"
    "network"
    "kvm"
    "beep"
  ];
  packages = with pkgs; [
    firefox
    kate
  ];
}
