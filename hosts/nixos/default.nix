{ inputs, outputs, pkgs, ... }:
{
  nixos = outputs.lib.nixosSystem {
    modules = (import ./modules) { inherit inputs outputs pkgs; };
    specialArgs = {
      inherit inputs outputs pkgs;
    };
  };
}
