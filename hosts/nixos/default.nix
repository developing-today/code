{ inputs, outputs, ... }:
{
  nixos = outputs.lib.nixosSystem {
    modules = (import ./modules) { inherit inputs outputs; };
    #overlays = overlays;
    specialArgs = { inherit inputs outputs; };
  };
}
