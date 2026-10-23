{ pkgs, ... }:
{
  users = {
    # defaultUserShell = pkgs.oils-for-unix;
    # defaultUserShell = pkgs.fish;
    defaultUserShell = pkgs.bash;
    # defaultUserShell = pkgs.zsh;
    mutableUsers = false;
    users.root.hashedPassword = "*";
  };
}
