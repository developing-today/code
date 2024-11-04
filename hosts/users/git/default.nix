{
  lib,
  pkgs,
  config,
  ...
}:
{
  # import [
  #   (lib.from-root "hosts/users")
  # ]
  #   users.users.git = {
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
}
