{
  inputs = {
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    nixvim = {
      url = "github:nix-community/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = {
    nixpkgs,
    nixvim,
    flake-utils,
    ...
  } @ inputs: let
    config = import ./config; # import the module directly
  in
    flake-utils.lib.eachDefaultSystem (system: let
      nixvimLib = nixvim.lib.${system};
      pkgs = import nixpkgs {inherit system;};
      nixvim' = nixvim.legacyPackages.${system};
      nvim = nixvim'.makeNixvimWithModule {
        inherit pkgs;
        module = config;
      };
    in {
      checks = {
        # Run `nix flake check .` to verify that your config is not broken
        default = nixvimLib.check.mkTestDerivationFromNvim {
          inherit nvim;
          name = "A nixvim configuration";
        };
      };

      packages = {
        # Lets you run `nix run .` to start nixvim
        default = nvim;
      };
      /*
      nixosModules.nixvim = { config, lib, pkgs, ... }: {
        options.programs.nixvim.enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
        };

        config = lib.mkIf config.programs.nixvim.enable {
          environment.systemPackages = [ nvim ];
        };
      };
      homeManagerModules.nixvim = { config, lib, pkgs, ... }: {
        options.programs.nixvim.enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Enable or disable NixVim.";
        };

        config = lib.mkIf config.programs.nixvim.enable {
          environment.systemPackages = [ nvim ];
        };
      };
      */
    });
}
