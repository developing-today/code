{
  inputs = {
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";

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
    };
  };

  outputs = {
    self,
    nixpkgs,
    neovim-nightly-overlay,
    ...
  }: {
    # Directly define outputs for each system
    legacyPackages.x86_64-linux = let
      overlay = final: prev: {
        inherit (neovim-nightly-overlay.packages.x86_64-linux) neovim;
      };
      pkgs = import nixpkgs {
        system = "x86_64-linux";
        overlays = [overlay];
      };
    in
      pkgs;

    legacyPackages.aarch64-linux = let
      overlay = final: prev: {
        inherit (neovim-nightly-overlay.packages.aarch64-linux) neovim;
      };
      pkgs = import nixpkgs {
        system = "aarch64-linux";
        overlays = [overlay];
      };
    in
      pkgs;

    # Add more systems if needed

    # Expose library
    lib = nixpkgs.lib;

    # Expose other top-level attributes as needed
    checks = {}; # don't skip?
    htmlDocs = nixpkgs.htmlDocs;
  };
}
