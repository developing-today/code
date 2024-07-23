{ inputs, outputs, ... }:
{
  nixos = outputs.lib.nixosSystem {
    modules = (import ./modules) { inherit inputs outputs; };
    specialArgs = { inherit inputs outputs; };
  };
}
