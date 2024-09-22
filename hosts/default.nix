{ inputs, outputs, lib, ... }:
{
  nixosConfigurations = (lib.mapAttrs (hostname: host-generator:
    let
      host = host-generator hostname;
    in
    lib.nixosSystem {
      specialArgs = {
        inherit inputs outputs lib hostname host;
        inherit (host) system stateVersion;
      };
      modules = lib.lists.flatten [
      /*
        ok so like, optional, deduped, non-existing removed
        ./modules/
        ./modules/abstract
        ./modules/{host.type}
        ./modules/{host.type}/{hostname}
        ./modules/{hostname}
        ./modules/{profile} for profile in host.profiles
        ./modules/{hostname}/{profile} for profile in host.profiles
        ./modules/{host.type}/${profile} for profile in host.profiles
        ./modules/{host.type}/{hostname}/${profile} for profile in host.profiles
        type*hostname*profile?
        users/<user>
        ++ host.modules (array or function returning array or single import?)
        ++ host.hardware (array or function returning array or single import?)
      */
        # Common modules
        # TODO: ./modules/all.nix
        # Hardware configuration
        # TODO: ./configurations/${hostname}-hardware.nix
        # Host-specific hardware configuration
        host.hardware
        # Host-specific configuration
        # TODO: host specific modules in host struct
        # TODO: ./configurations/${hostname}.nix
        # Desktop configuration
        # TODO: profiles to select in struct, this gets imported
        (import common/modules/desktop {
          inherit inputs outputs lib hostname host;
          inherit (host) system stateVersion;
        })
      ];
    }
  ) (import ../nixos-hosts.nix { inherit lib; }));
}
