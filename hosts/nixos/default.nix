{ inputs, outputs, lib, pkgs, ... }:
{
  nixos = lib.nixosSystem {
    modules = lib.lists.flatten [
      (import ../common/modules/desktop { inherit inputs outputs lib pkgs;
        stateVersion = "23.11";
        system = "x86_64-linux";
      })
      ../common/modules/hardware-configuration/framework-13/intel
    ];
    specialArgs = {
      inherit inputs outputs lib pkgs;
    };
  };
}
