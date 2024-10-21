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
inputs:
let
lib = inputs.nixpkgs.lib.attrsets.recursiveUpdate inputs.nixpkgs.lib inputs.home-manager.lib;

root =
  if builtins.hasAttr "outPath" inputs.self then
    inputs.self.outPath # Flake-based setup
  else
    builtins.toString ./.; # Traditional Nix setup, resolve to project root

pick =
  attrNames: attrSet:
  lib.filterAttrs (name: value: lib.elem name attrNames) attrSet;

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

nixos-host-configuration-base =
  options: name:
  lib.attrsets.recursiveUpdate rec {
    inherit name;
    type = name; # should type allow a list of types?
    # tags?
    system = "x86_64-linux";
    init = from-root "hosts/init";
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
nixos-host-configuration = options: name:
  let
    host = nixos-host-configuration-base options name;
  in
  lib.attrsets.recursiveUpdate host rec {
    wireless-secrets-template = config: "${host.wireless-secrets-template config}\n${make-wireless-template host config}";
  };

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

make-wireless-template = host: config:
  builtins.concatStringsSep "\n" (map
      (i: config.sops.placeholder."wireless_${i}")
      (ensure-list host.wireless));

# TODO: make-sd-card-installer-configurations
# TODO: make-unattended-sd-card-installer-configurations
# TODO: make-sd-card-image-configurations
# https://github.com/NixOS/nixpkgs/tree/master/nixos/modules/installer/sd-card
# https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/installer/sd-card/sd-image.nix
# https://myme.no/posts/2022-12-01-nixos-on-raspberrypi.html
# https://github.com/lucernae/nixos-pi
# nix build .#nixosConfigurations.rpi.config.system.build.sdImage to build the sd card image, and
# nix build .#nixosConfigurations.rpi.config.system.build.toplevel to build (only) the system

# TODO: make-netboot-installer-configurations
# TODO: make-unattended-netboot-installer-configurations
# TODO: make-netboot-image-configurations
# https://github.com/NixOS/nixpkgs/tree/master/nixos/modules/installer/netboot

# TODO: make-vm/cloud-installer-configurations?

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
    };
    modules = ensure-list host.init;
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
    nixos-host-configuration-base
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
    make-wireless-paths
    make-wireless
    make-wireless-template
    make-unattended-installer-configurations
    make-nixos-configurations
  ;
};
in
self
