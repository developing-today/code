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
  imports = [ inputs.home-manager.nixosModules.home-manager ];
  home-manager.useUserPackages = true;
  home-manager.useGlobalPkgs = true;
  home-manager.backupFileExtension = "backup";
  home-manager.users.user = import (lib.from-root "home/user") {
    inherit
      inputs
      lib
      stateVersion
      pkgs
      ;
  }; # todo use imports
  home-manager.users.backup = import (lib.from-root "home/backup") {
    inherit
      inputs
      lib
      stateVersion
      pkgs
      ;
  }; # todo use imports
  home-manager.users.root = import (lib.from-root "home/root") {
    inherit
      inputs
      lib
      stateVersion
      pkgs
      ;
  }; # todo use imports
}
