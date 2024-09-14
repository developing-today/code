{ inputs, outputs, lib, pkgs, ... }:
let
  system = "x86_64-linux";
  stateVersion = "23.11";
  overlays = outputs.overlays.${system};
in
{
  nixos = lib.nixosSystem {
    modules = lib.lists.flatten [
      (import ../common/modules/desktop { inherit inputs outputs lib pkgs system stateVersion; })
      ../common/modules/hardware-configuration/framework-13/intel
    ];
    # overlays = overlays;
    specialArgs = {
      inherit inputs outputs lib pkgs;
    };
  };
}
