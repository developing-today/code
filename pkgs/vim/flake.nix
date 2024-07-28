{
  inputs = {
    #nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "github:NixOS/nixpkgs/master";
    nixpkgs.url = "github:dezren39/nixpkgs/master";

    nixvim = {
      url = "github:nix-community/nixvim";
      #url = "github:developing-today-forks/nix-community_nixvim";
      #url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
      inputs.devshell.follows = "devshell";
      # inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.git-hooks.follows = "git-hooks";
      inputs.treefmt-nix.follows = "treefmt-nix";
      inputs.nix-darwin.follows = "nix-darwin";
    };
    nix-darwin = {
      url = "github:lnl7/nix-darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.gitignore.follows = "gitignore";
      inputs.flake-compat.follows = "flake-compat";
    };
    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    systems = {
      # url = "github:nix-systems/default-linux";
      url = "github:nix-systems/default";
      # inputs.nixpkgs.follows = "nixpkgs";
    };

    flake-utils = {
      url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
      inputs.systems.follows = "systems";
    };

    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
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
      inputs.git-hooks.follows = "git-hooks";
      inputs.neovim-src.follows = "neovim-src";
    }; # need to actually use this
    neovim-src = {
          url = "github:neovim/neovim";
          flake = false;
        };
    devshell = {
          url = "github:numtide/devshell";
          inputs.nixpkgs.follows = "nixpkgs";
        };
  };
  outputs =
    {
      nixpkgs,
      nixvim,
      flake-utils,
      neovim-nightly-overlay,
      ...
    }@inputs:
    let
      enablePkgs = { ... }@args: builtins.mapAttrs (n: v: v // { enable = true; }) args;
      enablePlugins =
        attrSet:
        if attrSet ? plugins then attrSet // { plugins = enablePkgs attrSet.plugins; } else attrSet;
      enableLspServers =
        attrSet:
        if attrSet ? lsp && attrSet.lsp ? servers then
          attrSet
          // {
            lsp = attrSet.lsp // {
              servers = enablePkgs attrSet.lsp.servers;
            };
          }
        else
          attrSet;
      enableColorschemes =
        attrSet:
        if attrSet ? colorschemes then
          attrSet // { colorschemes = enablePkgs attrSet.colorschemes; }
        else
          attrSet;
      enableModules = attrSet: enableColorschemes (enableLspServers (enablePlugins attrSet));
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            permittedInsecurePackages = [ "electron" ];
          };
          overlays = [ inputs.neovim-nightly-overlay.overlays.default ];
        };
        module = import ./config { inherit enableModules pkgs; };
        neovim = nixvim.legacyPackages.${system}.makeNixvimWithModule {
          inherit pkgs;
          module = module;
        };
        nixosModules = nixvim.nixosModules.nixvim;
        homeManagerModules = nixvim.homeManagerModules.nixvim;
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
  nixConfig = {
    # should match nix.settings
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
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    #     trusted-users = [ "root" "@wheel" ];
    trusted-users = [ "root" ];
    substituters = [
      # TODO: priority order
      "https://cache.nixos.org" #priority
      "https://yazi.cachix.org"
      #         "https://nix-community.cachix.org"
      #         "https://nix-gaming.cachix.org"
      #         "https://cache.m7.rs"
      #         "https://nrdxp.cachix.org"
      #         "https://numtide.cachix.org"
      #         "https://colmena.cachix.org"
      #         "https://sylvorg.cachix.org"
    ];
    trusted-substituters = [
      "https://cache.nixos.org" #priority
      "https://yazi.cachix.org"
      #         "https://nix-community.cachix.org"
      #         "https://nix-gaming.cachix.org"
      #         "https://cache.m7.rs"
      #         "https://nrdxp.cachix.org"
      #         "https://numtide.cachix.org"
      #         "https://colmena.cachix.org"
      #         "https://sylvorg.cachix.org"
    ];
    /*
    extra-substituters = [ "https://yazi.cachix.org" ];
    extra-trusted-public-keys = [ "yazi.cachix.org-1:Dcdz63NZKfvUCbDGngQDAZq6kOroIrFoyO064uvLh8k=" ];
    */
    trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "yazi.cachix.org-1:Dcdz63NZKfvUCbDGngQDAZq6kOroIrFoyO064uvLh8k="
      #         "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      #         "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
      #         "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
      #         "nrdxp.cachix.org-1:Fc5PSqY2Jm1TrWfm88l6cvGWwz3s93c6IOifQWnhNW4="
      #         "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      #         "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
      #         "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
    ];
    extra-substituters = [ ];
    extra-trusted-substituters = [ ];
    extra-trusted-public-keys = [ ];
    http-connections = 100; # 128 default:25
    max-substitution-jobs = 64; # 128 default:16
    # Store:querySubstitutablePaths Store::queryMissing binary-caches-parallel-connections fileTransferSettings.httpConnections
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
    #       buildMachines = [ ];
    #       distributedBuilds = true;
    #       # optional, useful when the builder has a faster internet connection than yours
    #       extraOptions = ''
    #         builders-use-substitutes = true
    #       '';
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
