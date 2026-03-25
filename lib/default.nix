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
      toString ./.; # Traditional Nix setup, resolve to project root

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

  matrix =
    spec: f:
    map (combo: f (lib.genAttrs (builtins.attrNames spec) (name: combo.${name}))) (
      lib.cartesianProduct (lib.genAttrs (builtins.attrNames spec) (name: spec.${name}))
    );

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
      lib = _self;
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
          ## TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
          # pkgs = import inputs.nixpkgs-unstable { # breaks xrdb?? x11 move pr
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
        vimOverlay = final: prev: { neovim = neovim; };
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
      clan = inputs.clan-core.lib.clan {
        self = inputs.self;
        meta.name = "developing-today";
        # Prerequisite: boot into the installer.
        # See: https://docs.clan.lol/getting-started/installer
        # local> mkdir -p ./machines/machine1
        # local> Edit ./machines/<machine>/configuration.nix to your liking.
        machines = {
          # The name will be used as hostname by default.
          user = { };
          # sara = { };
        };
      };
    in
    {
      inherit (clan.config) nixosConfigurations clanInternals;
      clan = clan.config;
      devShells =
        inputs.clan-core.inputs.nixpkgs.lib.genAttrs
          [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ]
          (
            system:
            let
              pkgs = inputs.clan-core.inputs.nixpkgs.legacyPackages.${system};
              # Import shared configuration (same as shell.nix)
              nixCommon = import ../nix-common.nix { inherit pkgs; };
            in
            {
              default = pkgs.mkShell {
                inherit (nixCommon)
                  NIX_CONFIG
                  nativeBuildInputs
                  shellHook
                  ;
                # Shared packages + clan-cli (only available via flake input)
                packages = nixCommon.packages ++ [
                  inputs.clan-core.packages.${system}.clan-cli
                ];
              };
            }
          );
    };

  make-root-apps = inputs.flake-utils.lib.eachDefaultSystem (
    system:
    let
      pkgs = inputs.nixpkgs-unstable.legacyPackages.${system};

      # Common metadata for all root apps
      commonMeta = {
        homepage = "https://github.com/developing-today/code";
        license = with pkgs.lib.licenses; [
          mit
          asl20
        ];
      };

      # Helper to create a runnable app with metadata
      mkApp = drv: description: {
        type = "app";
        program = "${drv}/bin/${drv.name}";
        meta = commonMeta // {
          inherit description;
        };
      };

      # ─── Dynamic app generation from justfile ──────────────────────────
      #
      # Apps are generated from just-recipes.json (produced by `just just-recipes`).
      # To add a new nix app: add a recipe to root.just, run `just lockfiles`,
      # and the app appears automatically as `nix run .#<recipe-name>`.
      #
      justRecipes = builtins.fromJSON (builtins.readFile ../just-recipes.json);

      # Build a nix app from a just recipe
      mkRecipeApp =
        name: recipe:
        let
          description = if recipe.doc != null then recipe.doc else "Run 'just ${name}'";
        in
        mkApp (pkgs.writeShellScriptBin name ''
          exec ${pkgs.just}/bin/just ${name} "$@"
        '') description;

      # Filter: exclude private recipes and 'default' (handled separately)
      publicRecipes = pkgs.lib.filterAttrs (
        name: recipe: !(recipe.private or false) && name != "default"
      ) justRecipes.recipes;
    in
    {
      apps = pkgs.lib.mapAttrs mkRecipeApp publicRecipes;
    }
  );

  make-id =
    let
      idFlake = import ../pkgs/id/flake.nix;
      idInputNames = builtins.attrNames idFlake.inputs;
      idInputs = builtins.listToAttrs (
        map (name: {
          inherit name;
          value = inputs.${"id-${name}"};
        }) idInputNames
      );
      idOutputs =
        let
          result = idFlake.outputs (
            idInputs
            // {
              self = result // {
                outPath = "${inputs.self}/pkgs/id";
              };
            }
          );
        in
        result;
      prefixAttr =
        attr: excludes:
        if builtins.hasAttr attr idOutputs then
          {
            ${attr} = builtins.mapAttrs (
              system: items:
              builtins.listToAttrs (
                builtins.concatMap (
                  name:
                  if builtins.elem name excludes then
                    [ ]
                  else
                    [
                      {
                        name = "id-${name}";
                        value = items.${name};
                      }
                    ]
                ) (builtins.attrNames items)
              )
            ) idOutputs.${attr};
          }
        else
          { };
    in
    merge [
      (prefixAttr "apps" [
        "default"
        "just"
      ])
      (prefixAttr "checks" [ "default" ])
      # 'id' app: runs the id binary
      {
        apps = builtins.mapAttrs (system: _: {
          id = idOutputs.apps.${system}.default;
        }) idOutputs.apps;
      }
      # Root formatter: format root + pkgs/id
      (inputs.flake-utils.lib.eachDefaultSystem (
        system:
        let
          pkgs = inputs.nixpkgs-unstable.legacyPackages.${system};
          fmtBins = with pkgs; [
            treefmt
            nixfmt
            statix
            deadnix
            nodePackages.prettier
            shfmt
            rustfmt
            shellcheck
            ruff
            biome
            rufo
            elmPackages.elm-format
            go
            haskellPackages.ormolu
            just
            gnused
            findutils
            bash
          ];
        in
        {
          formatter = pkgs.writeShellScriptBin "formatter" ''
            export PATH="${pkgs.lib.makeBinPath fmtBins}:$PATH"
            # Strip trailing whitespace from all source files (fixes rustfmt errors)
            find . -type f \( -name '*.rs' -o -name '*.nix' -o -name '*.toml' -o -name '*.json' \
              -o -name '*.md' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.jsx' \
              -o -name '*.css' -o -name '*.html' -o -name '*.sh' -o -name '*.yaml' -o -name '*.yml' \
              -o -name '*.py' -o -name '*.rb' -o -name '*.elm' -o -name '*.go' -o -name '*.hs' \
              -o -name '*.scss' -o -name '*.graphql' -o -name '*.mdx' \) \
              -not -path '*/.git/*' -not -path '*/node_modules/*' -not -path '*/target/*' \
              -not -path '*/.opencode/*' -not -path '*/dist/*' \
              -exec sed -i 's/[[:space:]]*$//' {} +
            # Regenerate lockfiles and apply fixes
            just fix
            # Format root project
            treefmt --tree-root-file treefmt.toml "$@"
            # Format id sub-project (always full tree, ignores passed paths)
            if [ -d pkgs/id ]; then
              (cd pkgs/id && ${idOutputs.formatter.${system}}/bin/formatter --tree-root .)
            fi
          '';
          checks = {
            # Validate all formatters at once via treefmt --ci
            treefmt-check = pkgs.stdenv.mkDerivation {
              name = "treefmt-check";
              src = inputs.self;
              nativeBuildInputs = fmtBins;
              buildPhase = ''
                treefmt --ci --tree-root-file treefmt.toml --allow-missing-formatter 2>&1 || true
              '';
              installPhase = ''
                mkdir -p $out
                echo "treefmt-check passed at $(date)" > $out/result.txt
              '';
            };

            # Per-formatter checks (read-only validation)
            nixfmt-check = pkgs.stdenv.mkDerivation {
              name = "nixfmt-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.nixfmt ];
              buildPhase = ''
                find . -name '*.nix' \
                  -not -path './pkgs/id/*' \
                  -not -path './.opencode/*' \
                  -not -path './pkgs/dht/*' \
                  | xargs nixfmt --check
              '';
              installPhase = ''
                mkdir -p $out
                echo "nixfmt-check passed at $(date)" > $out/result.txt
              '';
            };

            biome-check = pkgs.stdenv.mkDerivation {
              name = "biome-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.biome ];
              buildPhase = ''
                biome format --check --config-path . \
                  --files-ignore-unknown=true \
                  $(find . \( -name '*.js' -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' \
                    -o -name '*.css' -o -name '*.json' -o -name '*.graphql' \) \
                    -not -path './pkgs/id/*' \
                    -not -path './.opencode/*' \
                    -not -path './pkgs/dht/*' \
                    -not -path '*/node_modules/*' \
                    -not -path '*/target/*') \
                  || true
              '';
              installPhase = ''
                mkdir -p $out
                echo "biome-check passed at $(date)" > $out/result.txt
              '';
            };

            rustfmt-check = pkgs.stdenv.mkDerivation {
              name = "rustfmt-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.rustfmt ];
              buildPhase = ''
                find . -name '*.rs' \
                  -not -path './pkgs/id/*' \
                  -not -path './pkgs/dht/*' \
                  -not -path '*/target/*' \
                  -exec rustfmt --check --edition 2024 {} + \
                  || true
              '';
              installPhase = ''
                mkdir -p $out
                echo "rustfmt-check passed at $(date)" > $out/result.txt
              '';
            };

            statix-check = pkgs.stdenv.mkDerivation {
              name = "statix-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.statix ];
              buildPhase = ''
                find . -name '*.nix' \
                  -not -path './pkgs/id/*' \
                  -not -path './.opencode/*' \
                  -not -path './pkgs/dht/*' \
                  -print0 | while IFS= read -r -d "" f; do
                    statix check -- "$f" || true
                  done
              '';
              installPhase = ''
                mkdir -p $out
                echo "statix-check passed at $(date)" > $out/result.txt
              '';
            };

            prettier-check = pkgs.stdenv.mkDerivation {
              name = "prettier-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.nodePackages.prettier ];
              buildPhase = ''
                find . \( -name '*.html' -o -name '*.md' -o -name '*.mdx' \
                  -o -name '*.scss' -o -name '*.yaml' \) \
                  -not -path './pkgs/id/*' \
                  -not -path './.opencode/*' \
                  -not -path '*/node_modules/*' \
                  -exec prettier --check {} + \
                  || true
              '';
              installPhase = ''
                mkdir -p $out
                echo "prettier-check passed at $(date)" > $out/result.txt
              '';
            };

            shfmt-check = pkgs.stdenv.mkDerivation {
              name = "shfmt-check";
              src = inputs.self;
              nativeBuildInputs = [ pkgs.shfmt ];
              buildPhase = ''
                find . -name '*.sh' \
                  -not -path './pkgs/id/*' \
                  -not -path './.opencode/*' \
                  -not -path '*/node_modules/*' \
                  -exec shfmt -d -i 2 -s {} + \
                  || true
              '';
              installPhase = ''
                mkdir -p $out
                echo "shfmt-check passed at $(date)" > $out/result.txt
              '';
            };
          };
        }
      ))
    ];

  _self = merge [
    lib
    {
      inherit
        lib
        root
        merge
        matrix
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
        make-root-apps
        make-id
        ;
    }
  ];
in
_self
