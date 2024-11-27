{
  lib,
  pkgs,
  host,
  ...
}:
{
  users = {
    defaultUserShell = pkgs.oils-for-unix;
    mutableUsers = false;
    users.root.hashedPassword = "*";
  };
}
