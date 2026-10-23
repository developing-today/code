{
  config,
  pkgs,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:
{
  imports = [
    inputs.microvm.nixosModules.host
    {
      microvm.autostart = [ "prometheus" ];
      microvm.vms = {
        prometheus = {
          pkgs = import inputs.nixpkgs { system = "x86_64-linux"; };
          specialArgs = {
            inherit
              pkgs
              inputs
              hostName # TODO: pass vm host name here
              host # TODO: pass vm host here
              system
              stateVersion
              lib
              ;
          };
          config = import (lib.from-root "nixos/microvm/prometheus") {
            inherit
              config
              pkgs
              inputs
              hostName # TODO: pass vm host name here
              host # TODO: pass vm host here
              system
              stateVersion
              lib
              ;
          };
        };
      };
    }
  ];
}
