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
      "kvm"
      "beep"
      "libvirtd"
      "qemu"
      "qemu-libvirtd"
    ];
    packages = with pkgs; [
      firefox
      kdePackages.kate
    ];
  };
}
