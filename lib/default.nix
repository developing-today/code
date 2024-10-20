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
    lib.writeText "${name}.env" (
      lib.concatStringsSep "\n" (lib.mapAttrsToList (n: v: "${n}=${v}") value)
    );
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
  nixos-host-configuration = options: name:
    let
      host = nixos-host-configuration-base options name;
    in
    lib.attrsets.recursiveUpdate host rec {
      wireless-secrets-template = config: "${host.wireless-secrets-template config}\n${make-wireless-template host config}";
    };
  nixos-host-configuration-base =
    options: name:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      type = name;
      system = "x86_64-linux";
      stateVersion = "23.11";
      group-key = lib.group-key name;
      # groups =
      # or maybe secretGroups =
      email = "nixos-host-${name}@developing-today.com";
      sshKey = lib.host-key name; # allow multiple ssh keys
      users = [];
      modules = [ ];
      imports = [ ];
      hardware = [ "" ];
      hardware-modules = [ ];
      hardware-imports = [ ];
      networking = "dhcp";
      wireless = [];
      wireless-modules = [ ];
      wireless-imports = [ ];
      wireless-secrets-template = config: "";
      # TODO: wire networking in and allow other networking options,
      #       allow choosing wireless ? nixos only allows one wireless interface ?
      #       check out topology and todo-apu2.nix
      #       allow bridging interfaces
      #       allow nat/packet forwarding
      #       allow firewall things
      #       allow disable ipv6
      #       allow dhcpd and static ip addresses
      profiles = [ ];
      profile-modules = [ ];
      profile-imports = [ ];
      darwin-profiles = [ ];
      darwin-profile-modules = [ ];
      darwin-profile-imports = [ ];
      darwin-modules = [ ];
      darwin-imports = [ ];
      disks = [ ];
      disk-modules = [ ];
      disk-imports = [ ];
      bootstrap = false; # TODO: make this work or delete?
      # users # TODO: make this work host has users which have home-manager-users
    } options;
  default-home-manager-user-configuration = name: rec { # TODO: make this work? integrate into users?
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
  ensure-list = x: if builtins.isList x then x else [ x ];
  make-paths = strings: basePath: map (str: basePath + "/${str}") (ensure-list strings);
  make-hardware-paths =
    {
      basePath ? from-root "hosts/hardware-configuration",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-hardware = make-hardware-paths { };
  make-user-paths =
    {
      basePath ? from-root "hosts/users",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-users = make-user-paths { };
  make-profile-paths =
    {
      basePath ? from-root "hosts",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-profiles = make-profile-paths { };
  make-disk-paths =
    {
      basePath ? from-root "hosts/disks",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-disks = make-disk-paths { };
  make-wireless-paths =
    {
      basePath ? from-root "hosts/networking/wireless",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-wireless = make-wireless-paths { };
  make-wireless-template = host: config: builtins.concatStringsSep "\n" (map (i: config.sops.placeholder."wireless_${i}") (ensure-list host.wireless));
  make-unattended-installer-configurations = # TODO: make-bootstrap-versions
    configurations:
    lib.mapAttrs' (
      name: config:
      lib.nameValuePair "unattended-installer_offline_${name}" (
        inputs.unattended-installer.lib.diskoInstallerWrapper config {
          # https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/installer/cd-dvd/iso-image.nix
          config = {
            # isoImage.squashfsCompression = "gzip -Xcompression-level 1";
            isoImage.squashfsCompression = "zstd -Xcompression-level 6"; # no longer needed? https://github.com/chrillefkr/nixos-unattended-installer/issues/3  https://www.reddit.com/r/NixOS/s/xvUTQmq1NN
            unattendedInstaller = {
              successAction = builtins.readFile (from-root "lib/unattended-installer_successAction.sh");
              preDisko = builtins.readFile (from-root "lib/unattended-installer_preDisko.sh");
              postDisko = builtins.readFile (from-root "lib/unattended-installer_postDisko.sh");
              preInstall = builtins.readFile (from-root "lib/unattended-installer_preInstall.sh");
              postInstall = builtins.readFile (from-root "lib/unattended-installer_postInstall.sh");
            };
          };
        }
      )
    ) configurations;
  make-nixos-configurations = lib.mapAttrs (
    hostName: host-generator:
    let
      host = host-generator hostName;
    in
    lib.nixosSystem {
      specialArgs = {
        inherit
          inputs # maybe put hosts into inputs
          hostName # maybe change hostName to host.name and hosts key is alias or hostid
          host # maybe add host.name to host, maybe add host.id
        ;
        inherit (host) system stateVersion; # maybe just leave these in host?
        lib = self;
        /*
{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  modulesPath,
  lib,
  pkgs,
  ...
}:
        */
      };
      modules = lib.lists.flatten [
        /*
          # TODO: make generic array function and use that, maybe prefix one is enough?
          # TODO: fn to allow optionals for the auto-list below, removed before import
          from-root "hosts/abstract" # maybe don't import all, just ones needed as needed?
          from-root "hosts/hardware-configuration/${hostName}"
          from-root "hosts/{host.type}"
          from-root "hosts/{host.type}/{hostName}"
          from-root "hosts/{host.type}/{hostName}/{profile}" for profile in host.profiles
          from-root "hosts/{host.type}/{profile}" for profile in host.profiles
          from-root "hosts/{host.type}/{profile}/{hostName}" for profile in host.profiles
          from-root "hosts/{hostName}"
          from-root "hosts/{hostName}/{host.type}"
          from-root "hosts/{hostName}/{host.type}/{profile}" for profile in host.profiles
          from-root "hosts/{hostName}/{profile}" for profile in host.profiles
          from-root "hosts/{hostName}/{profile}/{host.type}" for profile in host.profiles
          from-root "hosts/{profile}" for profile in host.profiles
          from-root "hosts/{profile}/{host.type}" for profile in host.profiles
          from-root "hosts/{profile}/{hostName}" for profile in host.profiles
          from-root "hosts/{profile}/{host.type}/{hostName}" for profile in host.profiles
          from-root "hosts/{profile}/{hostName}/{host.type}" for profile in host.profiles
        */
        (ensure-list host.modules)
        (ensure-list host.imports)
        (make-hardware host.hardware)
        (ensure-list host.hardware-modules)
        (ensure-list host.hardware-imports)
        # networking # TODO: make this work
        (make-profiles host.profiles)
        (ensure-list host.profile-modules)
        (ensure-list host.profile-imports)
        (make-disks host.disks)
        (ensure-list host.disk-modules)
        (ensure-list host.disk-imports)
        (make-wireless host.wireless)
        (ensure-list host.wireless-modules)
        (ensure-list host.wireless-imports)
        # (make-darwin-modules host.darwin-profiles)
        (ensure-list host.darwin-profile-modules)
        (ensure-list host.darwin-profile-imports)
        (ensure-list host.darwin-modules)
        (ensure-list host.darwin-imports)
      ];
    }
  );
  self = lib.attrsets.recursiveUpdate lib {
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
      ensure-list
      make-paths
      make-hardware-paths
      make-hardware
      make-user-paths
      make-users
      make-profile-paths
      make-profiles
      make-disk-paths
      make-disks
      make-unattended-installer-configurations
      make-nixos-configurations
    ;
  };
in
self
