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

        home-manager.activation.createSymlink = hm.dag.entryAfter [ "writeBoundary" ] ''
          checkAndDeleteIfEmpty() {
            if [ -d "$1" ] && [ -z "$(find "$1" -maxdepth 0 -empty)" ]; then
              rm -r "$1"
            fi
          }

          checkAndDeleteIfEmpty "${config.home.homeDirectory}/.config/nvim"
          checkAndDeleteIfEmpty "${config.home.homeDirectory}/NvChad"
          checkAndDeleteIfEmpty "${config.home.homeDirectory}/forks/NvChad"
          checkAndDeleteIfEmpty "${config.home.homeDirectory}/NvChad/lua/custom"

          if [ ! -d "${config.home.homeDirectory}/.config/nvim" ]; then
            if [ -d "${config.home.homeDirectory}/NvChad" ] || [ -d "${config.home.homeDirectory}/forks/NvChad" ]; then
              ln -sf "${config.home.homeDirectory}/NvChad" "${config.home.homeDirectory}/.config/nvim" || ln -sf "${config.home.homeDirectory}/forks/NvChad" "${config.home.homeDirectory}/.config/nvim"
            else
              git clone --depth 1 "https://github.com/developing-today-forks/NvChad" "${config.home.homeDirectory}/NvChad"
            fi

            if [ ! -d "${config.home.homeDirectory}/NvChad/lua/custom" ]; then
              git clone --depth 1 "https://github.com/developing-today-forks/NvChad-custom" "${config.home.homeDirectory}/NvChad-custom"
              ln -sf "${config.home.homeDirectory}/NvChad-custom" "${config.home.homeDirectory}/NvChad/lua/custom"
            fi
          fi
        '';
        home-manager.users.user = {
          home.stateVersion = stateVersion;

          programs.neovim = {
            enable = true;
            defaultEditor = true;
            viAlias = true;
            vimAlias = true;

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

    in
    {
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
