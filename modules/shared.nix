{ config, ... }:
{
  # The services this file used to import from clan-core
  # (sshd, root-password, user-password, state-version) were removed
  # upstream. At the locked clan-core rev they are clanServices enabled
  # as inventory instances in lib/default.nix (make-clan): sshd runs the
  # OpenSSH server, and the users service generates the root and "user"
  # passwords. state-version needs no replacement: clanCore enables
  # clan.core.settings.state-version by default. See #159.

  # Locale service discovery and mDNS
  services.avahi.enable = true;

  # A generated password for the user below is provided by the users
  # clanService instance in lib/default.nix; read it with
  # `clan vars get user user-password-user/user-password`.
  users.users.user = {
    isNormalUser = true;
    extraGroups = [
      "wheel"
      "networkmanager"
      "video"
      "input"
      "adbusers"
    ];
    uid = 1000;
    openssh.authorizedKeys.keys = config.users.users.root.openssh.authorizedKeys.keys;
  };

  # Enable Android/ADB and MTP support
  # programs.adb.enable = true;
  services.udisks2.enable = true;
  services.gvfs.enable = true;
}
