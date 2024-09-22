{ lib, self ? builtins.getFlake "self" }:
let
  get-base-path = if builtins.hasAttr "outPath" self
    then self.outPath  # Flake-based setup
    else builtins.toString ../.; # Traditional Nix setup, resolve to project root
  resolve-path = path: "${get-base-path}/${path}";
  public-key = protocol: alias: builtins.readFile (resolve-path "keys/${protocol}-${alias}.pub");
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
      hardware = (resolve-path "hosts/common/modules/hardware-configuration");
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
  home-manager-user-configuration = name: options:
    lib.attrsets.recursiveUpdate (default-home-manager-user-configuration name) options;
in lib.attrsets.recursiveUpdate
  lib
  {
    inherit
      get-base-path
      resolve-path
      public-key
      group-key
      host-key
      user-key
      nixos-user-configuration
      nixos-host-configuration
      default-home-manager-user-configuration
      home-manager-user-configuration
    ;
  }
