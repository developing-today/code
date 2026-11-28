{
  lib,
  inputs,
  stateVersion,
  pkgs,
  system,
  ...
}:
{
  imports = [ (lib.from-root "nixos/home") ];
  home-manager.users.root = lib.merge [
    (import (lib.from-root "home/common") {
      inherit
        lib
        inputs
        stateVersion
        pkgs
        system
        ;
    })
  ];
}
