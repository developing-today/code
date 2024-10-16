{
  lib,
  pkgs,
  host,
  ...
}:
{
  imports = lib.make-users host.users;
  users = {
    defaultUserShell = pkgs.oils-for-unix;
    mutableUsers = false;
    users.root.hashedPassword = "*";
  };
}
