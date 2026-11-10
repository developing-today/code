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
    # The merged files' flake-module imports, gathered. lib.merge is
    # a recursiveUpdate fold, which keeps only ONE `imports` list —
    # the last one — so the per-file `imports` lines (home/common's
    # cascade modules, home/openclaw's module) do not take effect for
    # this user on their own; this element is the effective union.
    # (home/root merges home/common alone, where its own line does
    # apply.) Keep this list in step with the files above.
    {
      imports = [
        inputs.nix-openclaw.homeManagerModules.openclaw
        inputs.openchamber.homeManagerModules.openchamber
      ];

      # The OpenChamber browser-UI password file for THIS user (the
      # cascade's per-user setting; home/common stays user-agnostic).
      # The module's start wrapper exports it as
      # OPENCHAMBER_UI_PASSWORD when readable, as the old serve
      # script did.
      services.openchamber.uiPasswordFile = "/home/user/.config/openchamber/ui-password";
    }
  ];
}
