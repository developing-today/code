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
      ../common/modules/hardware-configuration/framework/13-inch/12th-gen-intel
    ];
    specialArgs = {
      inherit inputs outputs lib;
    };
  };
}
# nixosConfigurations = mapAttrs (
#   hostname: host:
#   nixosSystem {
#     specialArgs = {
#       inherit inputs host;
#     };
#     modules = [
#       ./configurations/${hostname}-hardware.nix
#       ./modules/all.nix
#       ./configurations/${hostname}.nix
#     ];
#   }
# ) (import ./hosts.nix);
