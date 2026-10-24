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
      # "trusted-users"
      #
      # Commented out: this group is never defined. There is no
      # `users.groups.trusted-users` anywhere in this config, so the membership
      # was inert -- update-users-groups.pl only warns on unknown groups, it
      # does not fail, which is why it went unnoticed since 2024-11.
      #
      # It would not have granted nix trust even if the group did exist. Nix
      # reads `nix.settings.trusted-users`, which takes usernames or "@group"
      # references; being a member of a group named "trusted-users" means
      # nothing on its own. That mismatch is why builds logged
      # "ignoring the client-specified setting ... you are not a trusted user"
      # and silently fell back to building instead of using the substituters.
      #
      # Trust is now granted in flake.nix nixConfig via
      # `trusted-users = [ "root" "@wheel" ]`, and this user is in wheel below.
      #
      # TO RESTORE the original intent instead of using @wheel, all three are
      # required: uncomment this line, add `users.groups.trusted-users = { };`,
      # and add "@trusted-users" to trusted-users in flake.nix nixConfig.
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
