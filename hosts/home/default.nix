{
  inputs,
  lib,
  pkgs,
  stateVersion,
  ...
}:
{
  imports = [ inputs.home-manager.nixosModules.home-manager ];
  home-manager.useUserPackages = true;
  home-manager.useGlobalPkgs = true;
  home-manager.backupFileExtension = "backup";
  home-manager.users.user = import (lib.from-root "home/user") {
    inherit
      inputs
      lib
      pkgs
      stateVersion
      ;
  }; # todo use imports
}
