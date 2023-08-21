{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    zig-overlay = {
      url = "github:developing-today-forks/zig-overlay/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home = {
      url = "path:./home";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    alejandra = {
      url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    zig-overlay,
    neovim-nightly-overlay,
    flake-utils,
    home,
    alejandra,
    ...
  }: let
    stateVersion = "23.11";
    overlays = [
      zig-overlay.overlays.default
      neovim-nightly-overlay.overlay
      alejandra.overlay
    ];
    systemNixOsModules = [
      {
        nixpkgs = {
          overlays = overlays;
          config.allowUnfree = true;
        };
        system.stateVersion = stateVersion;
      }
      ./configuration.nix
    ];
    hyprlandNixOsModules = [
      {
        programs = {
          hyprland = {
            enable = true;
          };
        };
      }
    ];
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      system = system;
      overlays = overlays;
    };

    homeManagerNixOsModules = home.homeManagerNixOsModules stateVersion;

  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules =
        systemNixOsModules
        ++ hyprlandNixOsModules
        ++ homeManagerNixOsModules;
    };
  };
}
