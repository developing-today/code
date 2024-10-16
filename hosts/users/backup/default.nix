{ lib, pkgs, config, ... }:
{
  imports = [
    (lib.from-root "home/backup")
  ];
  sops.secrets."users/backup/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/backup/password_backup.yaml";
  };
  users.users.backup = {
    hashedPasswordFile = config.sops.secrets."users/backup/passwordHash".path;
    isNormalUser = true;
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
  };
}
