{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  pkgs,
  ...
}:
{
  users = {
    # remove from here?
    defaultUserShell = pkgs.oils-for-unix; # pkgs.nushell; # oils-for-unix; #nushell; # per user?
    mutableUsers = false;
    users = {
      root.hashedPassword = "*"; # Disable root password # Is this needed?

      # todo modules
      user = import (lib.from-root "hosts/users/user") { inherit pkgs config; }; # imports
      backup = import (lib.from-root "hosts/users/backup") { inherit pkgs config; }; # imports
      # git = {
      #     isSystemUser = true;
      #     group = "git";
      #     home = "/var/lib/git-server";
      #     createHome = true;
      #     shell = "${pkgs.git}/bin/git-shell";
      #     openssh.authorizedKeys.keys = [
      #       # FIXME: Add pubkeys of authorized users
      #       "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF38sHxXn/r7KzWL1BVCqcKqmZA/V76N/y5p52UQghw7 example"
      #     ];
      #   };
    };
  };
  sops.secrets."users/backup/passwordHash" = {
    # imports
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/backup/password_backup.yaml";
  };
  sops.secrets."users/user/passwordHash" = {
    # imports
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/user/password_user.yaml";
  };
}
