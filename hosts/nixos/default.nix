{ inputs, outputs, lib, pkgs, ... }:
{
  nixos = lib.nixosSystem {
    modules = lib.lists.flatten [
      (import ../common/modules/desktop { inherit inputs outputs lib pkgs; })
      ../common/modules/hardware-configuration/framework-13/intel
    ];
    specialArgs = {
      inherit inputs outputs lib pkgs;
    };
  };
}
