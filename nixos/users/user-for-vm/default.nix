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
      "wpa_supplicant" # group for wpa_cli/wpa_gui control (was userControlled.group="network"; now fixed upstream)
      "kvm"
      "beep"
    ];
    packages = with pkgs; [
      firefox
      kdePackages.kate
    ];
  };
}
