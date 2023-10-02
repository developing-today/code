{
  inputs = {
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    nixvim-upstream = {
      # url = "github:nix-community/nixvim";
      url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.beautysh.follows = "beautysh";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };

    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; #*/ # inputs.systems
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    neovim-flake = {
      url = "github:neovim/neovim?dir=contrib";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    hercules-ci-agent = {
      url = "github:hercules-ci/hercules-ci-agent";
      inputs.flake-parts.follows = "flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-agent.follows = "hercules-ci-agent";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
      inputs.neovim-flake.follows = "neovim-flake";
    }; # need to actually use this

    beautysh = {
      url = "github:lovesegfault/beautysh";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
    };

    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
    };
  };
  outputs = {
    nixpkgs,
    nixvim-upstream,
    flake-utils,
    neovim-nightly-overlay,
    ...
  } @ inputs: let
    config = import ./config;
  in
    flake-utils.lib.eachDefaultSystem (system: let
      overlay = inputs.neovim-nightly-overlay.overlay;
      pkgs = import nixpkgs {
        inherit system;
        overlays = [overlay];
      };
      nixvimLib = nixvim-upstream.lib.${system};
      nixvim = nixvim-upstream.legacyPackages.${system};
      nvim = nixvim.makeNixvimWithModule {
        inherit pkgs;
        module = config;
      };
      nixosModules = nixvim-upstream.nixosModules.nixvim;
      homeManagerModules = nixvim-upstream.homeManagerModules.nixvim;
    in {
      packages = {
        default = nvim;
      };
      nixosModules = nixosModules;
      homeManagerModules = homeManagerModules;
    });
}
