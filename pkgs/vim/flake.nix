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

    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
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
          inputs.flake-utils.follows = "flake-utils";
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
}
