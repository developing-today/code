{ lib, pkgs, config, ... }:
{
  imports = [
    (lib.from-root "hosts/users")
    (lib.from-root "home/backup")
  ];
  sops.secrets."users/backup/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/backup/password_backup.yaml";
  };
  users.users.backup = {
    # uid = auto;
    hashedPasswordFile = config.sops.secrets."users/backup/passwordHash".path;
    isNormalUser = true;
    description = "backup";
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
  };
}
