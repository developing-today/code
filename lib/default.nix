inputs:
let
lib = inputs.nixpkgs.lib.attrsets.recursiveUpdate inputs.nixpkgs.lib inputs.home-manager.lib;
root =
  if builtins.hasAttr "outPath" inputs.self then
    inputs.self.outPath # Flake-based setup
  else
    builtins.toString ./.; # Traditional Nix setup, resolve to project root
pick = attrNames: attrSet: lib.filterAttrs (name: value: lib.elem name attrNames) attrSet;
mkEnv =
  name: value:
  lib.writeText "${name}.env" (lib.concatStringsSep "\n" (lib.mapAttrsToList (n: v: "${n}=${v}") value));
mergeAttrs =
  f: attrs:
  lib.foldlAttrs (
    acc: name: value:
    (lib.recursiveUpdate acc (f name value))
  ) { } attrs;
from-root = path: "${root}/${path}";
public-key = protocol: alias: builtins.readFile (from-root "keys/${protocol}-${alias}.pub");
group-key = alias: public-key "ssh-group" alias;
host-key = alias: public-key "ssh-host" alias;
user-key = alias: public-key "ssh-user" alias;
nixos-user-configuration =
  options: name:
  lib.attrsets.recursiveUpdate rec {
    inherit name;
    enable = true;
    uid = 1000;
    groups = [ "wheel" ];
    keys = [ (lib.user-key name) ];
    email = "nixos-user-${name}@developing-today.com";
    aliases = [
      "hi@developing-today.com"
      "abuse@developing-today.com"
      "dmca@developing-today.com"
      "drewrypope@gmail.com"
    ];
    # a nixos user optionally contains a home manager user?
  } options;
nixos-host-configuration =
  options: name:
  lib.attrsets.recursiveUpdate rec {
    inherit name;
    type = name;
    system = "x86_64-linux";
    stateVersion = "23.11";
    group-key = lib.group-key name;
    email = "nixos-host-${name}@developing-today.com";
    sshKey = lib.host-key name;
    hardware = [ "" ];
    profiles = [ ];
  } options;
default-home-manager-user-configuration = name: rec {
  system = "x86_64-linux";
  stateVersion = "23.11";
  home = rec {
    ide = rec {
      inherit name;
      enable = true;
      email = "home-manager-user-${name}@developing-today.com";
    };
    shell.enable = true;
    user = rec {
      inherit name;
      enable = true;
    };
  };
};
home-manager-user-configuration =
  name: options: lib.attrsets.recursiveUpdate (default-home-manager-user-configuration name) options;
ensure-list = x: if builtins.isList x then x else [x];
# lib.mapAttrs (username: user-generator: user-generator username)
make-paths = strings: basePath:
  map (str: basePath + "/${str}") (ensure-list strings);
# make-paths = strings: basePath:
#   map (str: basePath + "/${str}") (lib.toList strings);
make-hardware-paths = {
  basePath ? from-root "hosts/common/modules/hardware-configuration"
}: strings: make-paths (ensure-list strings) basePath;
make-hardware = make-hardware-paths {};
make-profile-paths = {
  basePath ? from-root "hosts/common/modules"
}: strings: make-paths (ensure-list strings) basePath;
make-profiles = make-profile-paths {};
in let
lib2 = lib.attrsets.recursiveUpdate lib {
  inherit
    lib
    root
    from-root
    public-key
    group-key
    host-key
    user-key
    nixos-user-configuration
    nixos-host-configuration
    default-home-manager-user-configuration
    home-manager-user-configuration
    make-paths
    make-hardware-paths
    make-hardware
    make-profile-paths
    make-profiles
  ;
}; in lib2.attrsets.recursiveUpdate lib2 {
  make-nixos-configurations = lib2.mapAttrs (
    hostname: host-generator:
    let
      host = host-generator hostname;
    in
    lib2.nixosSystem {
      specialArgs = {
        inherit
          inputs
          hostname
          host
          ;
        inherit (host) system stateVersion;
        lib = lib2;
      };
      modules = lib2.lists.flatten [
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
        (make-hardware host.hardware)
        (make-profiles host.profiles)
        # host.hardware-modules
        # host.profile-modules
        # hosts.darwin-profiles
        # hosts.darwin-profile-modules
      ];
    }
  );
}
