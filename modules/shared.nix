{ config, clan-core, ... }:
{
  imports = [
    # Enables the OpenSSH server for remote access
    clan-core.clanModules.sshd
    # Set a root password
    clan-core.clanModules.root-password
    clan-core.clanModules.user-password
    clan-core.clanModules.state-version
  ];

  # Locale service discovery and mDNS
  services.avahi.enable = true;

  # generate a random password for our user below
  # can be read using `clan secrets get <machine-name>-user-password` command
  clan.user-password.user = "user";
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
  programs.adb.enable = true;
  services.udisks2.enable = true;
  programs.gvfs.enable = true;
  environment.systemPackages = with config.nixpkgs; [
    mtpfs
    jmtpfs
  ];
}
