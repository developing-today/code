{
  description = "Home Manager Flake";

  inputs = {
    # todo: figure out that hyprland flake
    # master then if it breaks unstable then if it breaks 23.11 or something.
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";

    home-manager = {
      url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
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
    };
    nixvim = {
      url = "github:nix-community/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    home-manager,
    nixvim,
    ...
  }: {
    homeManagerNixOsModules = stateVersion: [
      ({pkgs, ...}: {
        imports = [
          home-manager.nixosModules.home-manager
        ];
        home-manager.users.user = import ./users/user.nix {inherit stateVersion pkgs;};
      })
    ];
  };
}
