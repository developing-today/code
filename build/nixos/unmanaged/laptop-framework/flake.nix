{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    zig-overlay.url = "github:mitchellh/zig-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    neovim-nightly-overlay.url = "github:nix-community/neovim-nightly-overlay";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, zig-overlay, flake-utils, home-manager, neovim-nightly-overlay, ... }:
  let
    system = "x86_64-linux";
    overlay = neovim-nightly-overlay.overlay;
    pkgs = import nixpkgs { system = system; overlays = [ overlay ]; };
    stateVersion = "23.11";
    homeManagerConfiguration = { pkgs, ... }: {
      imports = [
        home-manager.nixosModules.home-manager
      ];
      home-manager.users.user = {
        home.stateVersion = stateVersion;
        programs.neovim = {
          enable = true;
          defaultEditor = true;
          viAlias = true;
          vimAlias = true;
          vimdiffAlias = true;
          package = pkgs.neovim-nightly; # Use the nightly package
          # Rest of the neovim configuration
        };
      };
    };
  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        ./configuration.nix
        {
          nixpkgs.overlays = [
            (final: prev: { zigpkgs = zig-overlay.packages.${prev.system}; })
            overlay # Include the Neovim nightly overlay
          ];
          system.stateVersion = stateVersion;
        }
        homeManagerConfiguration
      ];
    };
  };
}

