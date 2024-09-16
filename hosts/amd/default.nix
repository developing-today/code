{
  inputs,
  outputs,
  lib,
  ...
}: # pkgs, ... }:
let
  system = "x86_64-linux";
  stateVersion = "23.11";
in
{
  nixos = lib.nixosSystem {
    modules = lib.lists.flatten [
      (import ../common/modules/desktop {
        inherit
          inputs
          outputs
          lib
          system
          stateVersion
          ;
      })
      ../common/modules/hardware-configuration/framework/13-inch/7040-amd
    ];
    specialArgs = {
      inherit inputs outputs lib;
    };
  };
}
