{
  lib,
  pkgs,
  config,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/users")
    (lib.from-root "home/backup")
    (lib.from-root "nixos/systemd/backup")
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
