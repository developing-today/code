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
          extraConfig = ''
            set runtimepath+=/home/user/forks/NvChad
            set packpath+=/home/user/forks/NvChad
            luafile /home/user/forks/NvChad/_init.lua
          '';
          plugins = [
            pkgs.vimPlugins.nvim-tree-lua
            {
              plugin = pkgs.vimPlugins.sqlite-lua;
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
      inherit system;
      modules = [
        ./configuration.nix
        {
          nixpkgs.overlays = [
            (final: prev: { zigpkgs = zig-overlay.packages.${prev.system}; })
          ];
          system.stateVersion = stateVersion;
        }
        homeManagerConfiguration
      ];
    };
  };
}

