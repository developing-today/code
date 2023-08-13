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
          " Determine the directory of the current Vim script
          let s:script_dir = expand('<sfile>:p:h')

          " Check for _init.lua, lua/core, and lua/plugins
          if filereadable(s:script_dir . '/_init.lua') && isdirectory(s:script_dir . '/lua/core') && isdirectory(s:script_dir . '/lua/plugins')
            " All exist, do nothing
          elseif (filereadable(s:script_dir . '/_init.lua') || isdirectory(s:script_dir . '/lua/core') || isdirectory(s:script_dir . '/lua/plugins')) && !(filereadable(s:script_dir . '/_init.lua') && isdirectory(s:script_dir . '/lua/core') && isdirectory(s:script_dir . '/lua/plugins'))
            " Partial existence, throw an error
            throw "Partial files and directories found. Please check the setup."
          else
            " All don't exist, clone NvChad into the current directory
            silent !sh -c '
              \ git clone https://github.com/developing-today-forks/NvChad ' . s:script_dir . '/NvChad &&
              \ mv ' . s:script_dir . '/NvChad/* ' . s:script_dir . '/ &&
              \ mv ' . s:script_dir . '/NvChad/.* ' . s:script_dir . '/ &&
              \ rm -rf ' . s:script_dir . '/NvChad
              \ '
          endif

          " Check for lua/custom and clone NvChad-custom directly if it doesn't exist
          if !isdirectory(s:script_dir . '/lua/custom')
            silent !sh -c '
              \ git clone https://github.com/developing-today-forks/NvChad-custom ' . s:script_dir . '/lua/custom
              \ '
          endif
          luafile ./_init.lua
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

