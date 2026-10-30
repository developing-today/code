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
  home-manager.users.user = lib.merge [
    (import (lib.from-root "home/common") {
      inherit
        lib
        inputs
        stateVersion
        pkgs
        system
        ;
    })
    # OpenClaw gateway (peer intelligence source) — home/openclaw.
    (import (lib.from-root "home/openclaw") {
      inherit
        lib
        inputs
        pkgs
        system
        ;
    })
  ];
}
