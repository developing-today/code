/*
  {
    config,
    inputs,
    host,
    hostName,
    system,
    stateVersion,
    lib,
    modulesPath,
    pkgs,
    ...
  }:
*/
inputs:
let
  lib = merge [
    inputs.nixpkgs.lib
    inputs.home-manager.lib
  ];

  root =
    if builtins.hasAttr "outPath" inputs.self then
      inputs.self.outPath # Flake-based setup
    else
      builtins.toString ./.; # Traditional Nix setup, resolve to project root

  pick = attrNames: attrSet: lib.filterAttrs (name: value: lib.elem name attrNames) attrSet;

  merge =
    array:
    let
      flattenDeep = x: if builtins.isList x then builtins.concatMap flattenDeep x else [ x ];
      # mergeWithConcat = a: b:
      #   let
      #     merge = path: l: r:
      #       if builtins.isList l && builtins.isList r
      #       then l ++ r
      #       else if builtins.isAttrs l && builtins.isAttrs r
      #       then inputs.nixpkgs.lib.attrsets.recursiveUpdateWith (merge (path + ".")) l r
      #       else r;
      #   in merge "" a b;

    in
    array |> flattenDeep |> builtins.foldl' inputs.nixpkgs.lib.attrsets.recursiveUpdate { };

  mkEnv =
    name: value:
    lib.writeText "${name}.env" (
      lib.concatStringsSep "\n" (lib.mapAttrsToList (n: v: "${n}=${v}") value)
    );

  mergeAttrs =
    f: attrs:
    lib.foldlAttrs (
      acc: name: value:
      (merge [
        acc
        (f name value)
      ])
    ) { } attrs;

  from-root = path: "${root}/${path}";

  public-key = protocol: alias: builtins.readFile (from-root "keys/${protocol}-${alias}.pub");
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;

  nixos-user-configuration =
    options: name:
    merge [
      rec {
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
      }
      options
    ];

  nixos-host-configuration-base =
    options: name:
    merge [
      rec {
        inherit name;
        type = name; # should type allow a list of types?
        # tags?
        system = "x86_64-linux";
        init = from-root "nixos/init";
        stateVersion = "23.11";
        group-key = lib.group-key name;
        # groups =
        # or maybe secretGroups =
        email = "nixos-host-${name}@developing-today.com";
        sshKey = lib.host-key name; # allow multiple ssh keys
        users = [ ];
        user-modules = [ ];
        user-imports = [ ];
        modules = [ ];
        imports = [ ];
        hardware = [ "" ];
        hardware-modules = [ ];
        hardware-imports = [ ];
        networking = "dhcp";
        wireless = [ ];
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
        vm = false;
        # users # TODO: make this work host has users which have home-manager-users
      }
      options
    ];
  nixos-host-configuration =
    options: name:
    let
      host = nixos-host-configuration-base options name;
    in
    merge [
      host
      {
        wireless-secrets-template =
          config: "${host.wireless-secrets-template config}\n${make-wireless-template host config}";
      }
    ];

  default-home-manager-user-configuration = name: {
    # TODO: make this work? integrate into users?
    system = "x86_64-linux";
    stateVersion = "23.11";
    home = {
      ide = rec {
        inherit name;
        enable = true;
        email = "home-manager-user-${name}@developing-today.com";
      };
      shell.enable = true;
      user = {
        inherit name;
        enable = true;
      };
    };
  };
  home-manager-user-configuration =
    name: options:
    merge [
      (default-home-manager-user-configuration name)
      options
    ];

  ensure-list = x: if builtins.isList x then x else [ x ];

  make-paths = strings: basePath: map (str: basePath + "/${str}") (ensure-list strings);

  make-hardware-paths =
    {
      basePath ? from-root "nixos/hardware-configuration",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-hardware = make-hardware-paths { };

  make-user-paths =
    {
      basePath ? from-root "nixos/users",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-users = make-user-paths { };

  make-profile-paths =
    {
      basePath ? from-root "nixos",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-profiles = make-profile-paths { };

  make-disk-paths =
    {
      basePath ? from-root "nixos/disks",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-disks = make-disk-paths { };

  make-wireless-paths =
    {
      basePath ? from-root "nixos/networking/wireless",
    }:
    strings: make-paths (ensure-list strings) basePath;
  make-wireless = make-wireless-paths { };

  make-wireless-template =
    host: config:
    builtins.concatStringsSep "\n" (
      map (i: config.sops.placeholder."wireless_${i}") (ensure-list host.wireless)
    );

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

  make-vm-configurations =
    hosts:
    lib.mapAttrs' (
      hostName: host-generator:
      let
        vmName = "vm_" + hostName;
      in
      lib.nameValuePair vmName (
        make-nixos-from-config (
          make-host-config vmName (
            name:
            merge [
              (host-generator name)
              { vm = true; }
            ]
          )
        )
      )
    ) hosts;

  make-host-config =
    hostName: host-generator:
    let
      host = host-generator hostName;
    in
    {
      inherit inputs;
      inherit host hostName; # move hostName into host?
      inherit (host) system stateVersion; # move into host?
      lib = self;
    };

  make-nixos-from-config =
    config:
    lib.nixosSystem {
      system = null;
      specialArgs = {
        inherit (config)
          inputs
          host
          hostName
          system
          stateVersion
          lib
          ;
      };
      modules = ensure-list config.host.init;
    };

  make-nixos-configurations = lib.mapAttrs (
    hostName: host-generator: make-nixos-from-config (make-host-config hostName host-generator)
  );

  make-vim =
    let
      enablePkgs =
        { ... }@args:
        builtins.mapAttrs (
          n: v:
          merge [
            v
            { enable = true; }
          ]
        ) args;
      enablePlugins =
        attrSet:
        if attrSet ? plugins then
          merge [
            attrSet
            { plugins = enablePkgs attrSet.plugins; }
          ]
        else
          attrSet;
      enableLspServers =
        attrSet:
        if attrSet ? lsp && attrSet.lsp ? servers then
          merge [
            attrSet
            {
              lsp = merge [
                attrSet.lsp
                { servers = enablePkgs attrSet.lsp.servers; }
              ];
            }
          ]
        else
          attrSet;
      enableColorschemes =
        attrSet:
        if attrSet ? colorschemes then
          merge [
            attrSet
            { colorschemes = enablePkgs attrSet.colorschemes; }
          ]
        else
          attrSet;
      enableModules = attrSet: enableColorschemes (enableLspServers (enablePlugins attrSet));
    in
    inputs.flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import inputs.nixpkgs {
          inherit system;
          config = {
            allowBroken = true;
            allowUnfree = true;
            allowUnfreePredicate = _: true;
            permittedInsecurePackages = [
              "olm-3.2.16"
              "electron"
              "qtwebkit-5.212.0-alpha4"
            ];
          };
          overlays = [ inputs.neovim-nightly-overlay.overlays.default ];
        };
        module = import (from-root "pkgs/vim/config") { inherit enableModules pkgs; };
        neovim = inputs.nixvim.legacyPackages.${system}.makeNixvimWithModule {
          inherit pkgs;
          module = module;
        };
        nixosModules = inputs.nixvim.nixosModules.nixvim;
        homeManagerModules = inputs.nixvim.homeManagerModules.nixvim;
      in
      {
        packages = {
          default = neovim;
        };
        nixosModules = nixosModules; # unsure how to overlay nightly here.
        homeManagerModules = homeManagerModules; # unsure how to overlay nightly here.
        overlay = final: prev: { neovim = neovim; };
        enableModules = enableModules;
        enableColorschemes = enableColorschemes;
        enableLspServers = enableLspServers;
        enablePkgs = enablePkgs;
        enablePlugins = enablePlugins;
      }
    );

  make-clan = # hosts:
    let
      # Usage see: https://docs.clan.lol
      clan = inputs.clan-core.lib.buildClan {
        directory = self;
        meta.name = "devtoday";
        # meta.name = "developing-today";

        # Prerequisite: boot into the installer.
        # See: https://docs.clan.lol/getting-started/installer
        # local> mkdir -p ./machines/machine1
        # local> Edit ./machines/<machine>/configuration.nix to your liking.
        machines = {
          # The name will be used as hostname by default.
          jon = { };
          sara = { };
        };
      };
    in
    {
      # All machines managed by Clan.
      inherit (clan) clanInternals nixosConfigurations; # nixosConfigurations clanInternals;
      # Add the Clan cli tool to the dev shell.
      # Use "nix develop" to enter the dev shell.
      devShells =
        inputs.clan-core.inputs.nixpkgs.lib.genAttrs
          [
            "x86_64-linux"
            # "aarch64-linux"
            # "aarch64-darwin"
            # "x86_64-darwin"
          ]
          (system: {
            default = inputs.clan-core.inputs.nixpkgs.legacyPackages.${system}.mkShell {
              packages = [ inputs.clan-core.packages.${system}.clan-cli ];
            };
          });
    };

  self = merge [
    lib
    {
      inherit
        lib
        root
        merge
        from-root
        public-key
        group-key
        host-key
        user-key
        pick
        mkEnv
        mergeAttrs
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
        make-vm-configurations
        make-unattended-installer-configurations
        make-nixos-from-config
        make-host-config
        make-nixos-configurations
        make-vim
        make-clan
        ;
    }
  ];
in
self
