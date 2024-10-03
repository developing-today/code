{
  inputs,
  ...
}:
let
  lib = inputs.self.lib;
in
  lib.mapAttrs (
    hostname: host-generator:
    let
      host = host-generator hostname;
    in
    lib.nixosSystem {
      specialArgs = {
        inherit
          inputs
          lib
          hostname
          host
          ;
        inherit (host) system stateVersion;
      };
      modules = lib.lists.flatten [
        /*
          ok so like, optional, deduped, non-existing removed
          ./hosts/modules
          ./hosts/modules/${hostname}
          ./hosts/modules/hardware-configuration
          ./hosts/modules/hardware-configuration/${hostname}
          ./hosts/modules/abstract
          ./hosts/modules/{host.type}
          ./hosts/modules/{host.type}/{hostname}
          ./hosts/modules/{hostname}
          ./hosts/modules/{profile} for profile in host.profiles
          ./hosts/modules/{hostname}/{profile} for profile in host.profiles
          ./hosts/modules/{host.type}/${profile} for profile in host.profiles
          ./hosts/modules/{host.type}/{hostname}/${profile} for profile in host.profiles
          ./hosts/users
          lib.make-users host.users
        */
        (lib.make-hardware host.hardware)
        (lib.make-profiles host.profiles)
        # host.hardware-modules
        # host.profile-modules
        # hosts.darwin-profiles
        # hosts.darwin-profile-modules
      ];
    }
  ) inputs.self.hosts
