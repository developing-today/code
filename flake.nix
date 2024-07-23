{
  description = "developing.today NixOS configuration";
  inputs = {
    #nixpkgs.url = "github:NixOS/nixpkgs";
    nixpkgs.url = "github:dezren39/nixpkgs/master";

    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.05";

    hardware.url = "github:nixos/nixos-hardware";
    nix-colors.url = "github:misterio77/nix-colors";

    # Third party programs, packaged with nix
    firefox-addons = {
      url = "gitlab:rycee/nur-expressions?dir=pkgs/firefox-addons";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-gaming = {
      url = "github:fufexan/nix-gaming";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2305.491756.tar.gz"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "github:DeterminateSystems/nixpkgs/nix_2_18_1";
    nix-topology.url = "github:oddlama/nix-topology";
    sops-nix = {
      url = "github:mic92/sops-nix";
      #inputs.nixpkgs-stable.follows ="nixpkgs";
      #inputs.nixpkgs.follows ="nixpkgs";
    };
    impermanence = {
      url = "github:Nix-community/impermanence";
    };
    home-manager = {
      url = "github:nix-community/home-manager"; # */
      #url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
    };
    #  hardware.url = "github:nixos/nixos-hardware"; # todo figure out how to use this
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
    flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects";
      inputs.flake-parts.follows = "flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
    };
    # must type follows all out every time
    # because flake inputs are basically static
    # can't make a let var function closure thing around it or whatever
    zig-overlay = {
      url = "github:mitchellh/zig-overlay"; # url = "github:developing-today-forks/zig-overlay/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
    };
    alejandra = {
      url = "https://flakehub.com/f/kamadorueda/alejandra/*.tar.gz"; # */ # url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flakeCompat.follows = "flake-compat";
    };
    #nix-software-center = {
    #  url = "github:snowfallorg/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "github:vlinkz/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "github:vlinkz/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "https://flakehub.com/f/vlinkz/nix-software-center/*.tar.gz"; #*/ # https://github.com/vlinkz/nix-software-center/pull/50
    #  inputs.nixpkgs.follows = "nixpkgs";
    #  inputs.utils.follows = "flake-utils";
    #};
    nixvim = {
      url = "github:nix-community/nixvim";
      #url = "github:developing-today-forks/nix-community_nixvim";
      #url = "github:developing-today-forks/nixvim-flake";
      #inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./pkgs/vim";
      #url = "github:developing-today/code?dir=src/vim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixvim.follows = "nixvim";
      inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
    };
    # nix-rice = https://github.com/bertof/nix-rice # todo fork and rename this garbage

    systems.url = "github:nix-systems/default-linux";

#     nix-inspect = {
#       url = "github:bluskript/nix-inspect";
#       inputs.nixpkgs.follows = "nixpkgs";
#     };
#     home-manager = {
#       url = "github:nix-community/home-manager";
#       inputs.nixpkgs.follows = "nixpkgs";
#     };
#     nixos-wsl = {
#       url = "github:nix-community/NixOS-WSL";
#       inputs.nixpkgs.follows = "nixpkgs";
#     };

  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      zig-overlay,
      alejandra,
      sops-nix,
      vim,
      home-manager,
      #      nix-software-center,
      nix-topology,
      systems,
      ...
    }@inputs:
    let # all let vars should be added to inherit below.
      #       inherit (self) outputs;
      stateVersion = "23.11";
      system = "x86_64-linux";
      supportedSystems = [ "x86_64-linux" ]; # "x86_64-darwin" "aarch64-linux" "aarch64-darwin" ];
      lib = nixpkgs.lib // home-manager.lib;
      forEachSystem = f: lib.genAttrs supportedSystems;
      pkgsFor = forEachSystem (
        system:
        import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        }
      );
      #inherit lib;
      overlays = {
      x86_64-linux = [
        # import ./overlays { inherit inputs outputs;};

        #     zig-overlay.overlays.default
        #alejandra.overlay
        #nix-software-center.overlay
        vim.overlay.x86_64-linux # .${system}
        #nix-topology.overlays.default
      ];};
    in
    rec {
      inherit
        stateVersion
        system
        supportedSystems
        lib
        forEachSystem
        pkgsFor
        overlays
        ; # all let vars should be added here.
      nixosConfigurations =
        {}//
        (import ./hosts/nixos) { inherit inputs; outputs = self.outputs; };
      #laptop-framework = lib.nixosSystem {
#       {
#         nixos = lib.nixosSystem {
#           #           modules = nixosModules;
#           modules = (import ./hosts/nixos/modules) {
#             inherit inputs;
#             outputs = self.outputs;
#           };
#           #overlays = overlays;
#           specialArgs = {
#             inherit inputs;
#             outputs = self.outputs;
#           };
#         };
#       };
      /*
        overlays = [
          zig-overlay.overlays.default
          alejandra.overlay
          #nix-software-center.overlay
          vim.overlay.${system}
          nix-topology.overlays.default
        ];
      */
      #nixosModules = import ./modules/nixos;
      #homeManagerModules = import ./modules/home-manager;
      #overlays = import ./overlays {inherit inputs outputs;};
      #hydraJobs = import ./hydra.nix {inherit inputs outputs;};
      #packages = forEachSystem (pkgs: import ./pkgs {inherit pkgs;});
      #devShells = forEachSystem (pkgs: import ./shell.nix {inherit pkgs;});
      #formatter = forEachSystem (pkgs: pkgs.alejandra);

      /*
        homeConfigurations = {
          # Standalone HM only
          "user@laptop-framework" = lib.homeManagerConfiguration {
            modules = [ ./home/user/user.nix ];
            pkgs = pkgsFor.x86_64-linux;
            extraSpecialArgs = { inherit inputs; outputs = self.outputs; };
          };
        };
      */
    };
  # }
  /*
    nix-topology.nixosModules.default
         {
           nixpkgs = {
             overlays = overlays; # are overlays needed in home manager? document which/why?
             config = {
               allowUnfree = true;
               permittedInsecurePackages = [
                 "electron" # le sigh
               ];
             };
           };
           system.stateVersion = stateVersion;
         }
         ./lib/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
         #sops-nix.nixosModules.sops
         ./modules/nixos/cachix.nix
         ./hosts/laptop-framework/hardware-configuration/laptop-framework.nix
       ];
       # overlayNixosModules = ?
       hyprlandNixosModules = [
         (import ./modules/nixos/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
         #       (import ./modules/nm-applet.nix)
       ];
       system = "x86_64-linux";
       pkgs = import nixpkgs {
         system = system;
         overlays = overlays;
       };
       lib = nixpkgs.lib;
       #zed-editor = pkgs.callPackage "${nixpkgs}/pkgs/by-name/ze/zed-editor/package.nix" { };

       #zed-fhs = pkgs.buildFHSUserEnv {
       #  name = "zed";
       #  targetPkgs = pkgs: [ zed-editor ];
       #  runScript = "zed";
       #};

       homeManagerNixosModules = [
         (
           { ... }:
           {
             imports = [
               home-manager.nixosModules.home-manager
               vim.nixosModules.${system}
             ];

             home-manager.useUserPackages = true;
             home-manager.useGlobalPkgs = true;
             home-manager.backupFileExtension = "backup";
             home-manager.users.user = import ./home/user/user.nix {
               inherit stateVersion;
               pkgs = import nixpkgs {
                 inherit system;
                 overlays = overlays;
                 config = {
                   allowUnfree = true;
                   permittedInsecurePackages = [
                     "electron" # le sigh
                   ];
                 };
               };
             };
           }
         )
       ];

       devShellInner = pkgs.mkShell { buildInputs = [ /*zed-fhs
  */
  /*
    ]; };

        in
        {
          inherit lib pkgs devShellInner;
          nixosConfigurations.nixos = lib.nixosSystem {
            inherit system;
            modules = systemNixosModules ++ hyprlandNixosModules ++ homeManagerNixosModules;
          };
          specialArgs = {
            inherit inputs outputs;
          };
          devShell.${system} = devShellInner;
          # Repeat this for each system where you want to build your topology.
          # You can do this manually or use flake-utils.
          topology.x86_64-linux = import nix-topology {
            inherit pkgs; # Only this package set must include nix-topology.overlays.default
            modules = [
              ## Your own file to define global topology. Works in principle like a nixos module but uses different options.
              #./topology.nix
              # Inline module to inform topology of your existing NixOS hosts.
              { nixosConfigurations = self.nixosConfigurations; }
            ];
          };
        };
      # hydra
      # content addressible
    }
  */
  nixConfig = {
    # This will add each flake input as a registry
    # To make nix3 commands consistent with your flake
    #     registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs);

    # This will additionally add your inputs to the system's legacy channels
    # Making legacy nix commands consistent as well, awesome!
    #     nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry;
    #     settings = {
    experimental-features = [
      "auto-allocate-uids"
      "ca-derivations"
      "cgroups"
      "dynamic-derivations"
      "fetch-closure"
      "flakes"
      "git-hashing"
      # "local-overlay-store" # look into this
      # "mounted-ssh-store" # look into this
      "nix-command"
      # "no-url-literals" # <- removed no-url-literals for flakehub testing
      "parse-toml-timestamps"
      "read-only-local-store"
      "recursive-nix"
      "verified-fetches"
    ];
    #       trusted-users = [ "user" ];
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    #       substituters = [ "https://cache.nixos.org" ];
    #       trusted-public-keys = [ "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" ];

    trusted-users = [ "root" ];
    #     trusted-user = "root";
    substituters = [
      "https://cache.nixos.org"
      #"https://hydra.nixos.org"
      "https://nix-community.cachix.org"
      "https://numtide.cachix.org"
      "https://colmena.cachix.org"
      "https://nix-gaming.cachix.org"
      "https://nrdxp.cachix.org"
      "https://cache.m7.rs"
      "https://sylvorg.cachix.org"
    ];
    extra-substituters = [
      #       "https://cache.nixos.org"
      #       #"https://hydra.nixos.org"
      #       "https://nix-community.cachix.org"
      #       "https://numtide.cachix.org"
      #       "https://colmena.cachix.org"
      #       "https://nix-gaming.cachix.org"
      #       "https://nrdxp.cachix.org"
      #       "https://cache.m7.rs"
      #       "https://sylvorg.cachix.org"
    ];
    trusted-substituters = [
      "https://cache.nixos.org"
      #"https://hydra.nixos.org"
      "https://nix-community.cachix.org"
      "https://numtide.cachix.org"
      "https://colmena.cachix.org"
      "https://nix-gaming.cachix.org"
      "https://nrdxp.cachix.org"
      "https://cache.m7.rs"
      "https://sylvorg.cachix.org"
    ];
    extra-trusted-substituters = [
      #       "https://cache.nixos.org"
      #       #"https://hydra.nixos.org"
      #       "https://nix-community.cachix.org"
      #       "https://numtide.cachix.org"
      #       "https://colmena.cachix.org"
      #       "https://nix-gaming.cachix.org"
      #       "https://nrdxp.cachix.org"
      #       "https://cache.m7.rs"
      #       "https://sylvorg.cachix.org"
    ];
    trusted-public-keys = [
      "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
      "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
      "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
    ];
    extra-trusted-public-keys = [
      #       "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
      #       "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
      #       "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      #       "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      #       "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      #       "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
      #       "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      #       "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
    ];
    http-connections = 128;
    max-substitution-jobs = 128;
    keep-outputs = true; # Nice for developers
    keep-derivations = true; # Idem
    accept-flake-config = true;
    #     allow-dirty = false;
    allow-dirty = true;
    #     builders-use-substitutes = true;
    fallback = true;
    log-lines = 128;
    #     pure-eval = true;
    # run-diff-hook = true;
    # secret-key-files
    show-trace = true;
    # tarball-ttl = 0;
    # trace-function-calls = true;
    trace-verbose = true;
    # use-xdg-base-directories = true;
    #     allow-dirty = false;

    /*
      buildMachines = [ ];
      distributedBuilds = true;
      # optional, useful when the builder has a faster internet connection than yours
      extraOptions = ''
        builders-use-substitutes = true
      '';
    */
    auto-optimise-store = true;
    #pure-eval = true;
    pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
    restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
    use-registries = true;
    use-cgroups = true;
    #     };
    #     package = pkgs.nixVersions.nix_2_23;
    #     optimise.automatic = true;
    # auto-optimise-store = true;
    #     gc = {
    #       automatic = true;
    #       dates = "weekly";
    #       options = "--delete-older-than 180d";
    #     };
  };
}
