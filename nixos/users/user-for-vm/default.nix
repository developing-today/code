{ lib, pkgs, ... }:
{
  imports = [
    (lib.from-root "nixos/users")
    (lib.from-root "home/user")
    (lib.from-root "nixos/systemd/user")
  ];
  users.users.user = {
    uid = 1337;
    isNormalUser = true;
    initialPassword = "password";
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
      kdePackages.kate
    ];
  };
}
