{ inputs, outputs, lib, pkgs, ... }:
{
  nixos = lib.nixosSystem {
    modules = (import ./modules) { inherit inputs outputs lib pkgs; };
    specialArgs = {
      inherit inputs outputs lib pkgs;
    };
  };
}
