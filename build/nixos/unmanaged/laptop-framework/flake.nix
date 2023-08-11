{
  inputs.nixpkgs.url = "nixpkgs/nixos-unstable";
  inputs.zig-overlay.url = "github:mitchellh/zig-overlay";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.home-manager.url = "github:nix-community/home-manager";
  inputs.home-manager.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, zig-overlay, flake-utils, home-manager, ... }:
    let
      homeManagerConfiguration = { pkgs, ... }: {
        imports = [
          home-manager.nixosModules.home-manager
        ];

        home-manager.users.user = {
          home.stateVersion = "23.11"
          programs.neovim = {
            enable = true;
            defaultEditor = true;
            viAlias = true;
            vimAlias = true;

            plugins = [
              pkgs.vimPlugins.nvim-tree-lua
              {
                plugin = pkgs.sqlite-lua;
                config = "let g:sqlite_clib_path = '${pkgs.sqlite.out}/lib/libsqlite3.so'";
              }
              {
                plugin = pkgs.vimPlugins.vim-startify;
                config = "let g:startify_change_to_vcs_root = 0";
              }
              pkgs.vimPlugins.vim-nix
            ];
          };
        };
      };
    in {
      nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          ./configuration.nix
          ({ pkgs, ... }: {
            nixpkgs.overlays = [
              (final: prev: { zigpkgs = zig-overlay.packages.${prev.system}; })
            ];
          })
          homeManagerConfiguration
        ];
      };
    };
  }

