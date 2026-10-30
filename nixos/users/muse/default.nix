{
  lib,
  config,
  ...
}:
{
  imports = [
    (lib.from-root "nixos/users")
  ];
  sops.secrets."users/muse/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/muse/password_muse.yaml";
  };
  users.users.muse = {
    uid = 1338;
    isNormalUser = true;
    hashedPasswordFile = config.sops.secrets."users/muse/passwordHash".path;
    description = "Muse (AI agent)";
    extraGroups = [
      "wheel"
    ];
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKQhcm1Z0TGjZT9JgIugIxJmc6xtcVrTDSI0HweJIDtA hatch"
    ];
  };
}
