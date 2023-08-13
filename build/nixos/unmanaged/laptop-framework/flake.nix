{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    zig-overlay.url = "github:mitchellh/zig-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, zig-overlay, flake-utils, home-manager, ... }:
  let
    system = "x86_64-linux";
    pkgs = nixpkgs.legacyPackages.${system};
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
        extraConfig = ''
          set runtimepath+=/home/user/forks/NvChad
          luafile _init.lua
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

