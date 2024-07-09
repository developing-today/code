{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/master";
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";

    # todo: figure out that hyprland flake
    home-manager = {
      url = "github:nix-community/home-manager"; # */
      #url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
    flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz"; # flake = false?
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

    nixvim = {
      #url = "github:developing-today-forks/nix-community_nixvim";
      url = "github:nix-community/nixvim";
      #url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./programs/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixvim.follows = "nixvim";
      inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
    };
  };
  outputs =
    {
      self,
      nixpkgs,
      home-manager,
      vim,
      ...
    }@inputs:
    let
      system = "x86_64-linux"; # something something flake-utils
      vimOverlay = vim.overlay.${system};
      overlays = [ vimOverlay ];
    in
    {
      homeManagerNixosModules = stateVersion: [
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
            home-manager.users.user = import ./users/user.nix {
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
      vim-overlay = vimOverlay;
    };
}
