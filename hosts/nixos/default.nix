{ inputs, outputs, lib, pkgs, ... }:
{
  nixos = lib.nixosSystem {
    modules = (import ./modules) { inherit inputs outputs pkgs; };
    specialArgs = {
      inherit inputs outputs pkgs;
    };
  };
}
