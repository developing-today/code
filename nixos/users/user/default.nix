{
  lib,
  pkgs,
  config,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/users")
    (lib.from-root "home/user")
    (lib.from-root "nixos/systemd/user")
  ];
  sops.secrets."users/user/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/user/password_user.yaml";
  };
  users.groups.plugdev = { };
  users.users.user = {
    uid = 1337;
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
      "wpa_supplicant" # group for wpa_cli/wpa_gui control (was userControlled.group="network"; now fixed upstream)
      "kvm"
      "beep"
      "libvirtd"
      "qemu"
      "qemu-libvirtd"
      "adbusers" # For Android phone connectivity
      "dialout" # Serial consoles, programmers and development boards
      "plugdev" # SDRs and vendor USB debug hardware
      "wireshark" # USB and network protocol capture
    ];
    packages = with pkgs; [
      firefox
      kdePackages.kate
    ];
  };
}
