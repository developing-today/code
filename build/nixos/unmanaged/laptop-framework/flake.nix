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

            extraConfig = ''
              let g:sqlite_clib_path = '${pkgs.sqlite.out}/lib/libsqlite3.so'
            '';

            plugins = with pkgs.vimPlugins; [
              nvim-tree-lua
              sqlite-lua
              vim-startify
              vim-nix
            ];
          };

          home.activation = {
            createSymlink = dag.entryAfter [ "writeBoundary" ] ''
              checkAndDeleteIfEmpty() {
                if [ -d "$1" ] && [ -z "$(find "$1" -maxdepth 0 -empty)" ]; then
                  rm -r "$1"
                fi
              }

              checkAndDeleteIfEmpty "~/.config/nvim"
              checkAndDeleteIfEmpty "~/NvChad"
              checkAndDeleteIfEmpty "~/forks/NvChad"
              checkAndDeleteIfEmpty "~/NvChad/lua/custom"

              if [ ! -d "~/.config/nvim" ]; then
                if [ -d "~/NvChad" ] || [ -d "~/forks/NvChad" ]; then
                  ln -sf "~/NvChad" "~/.config/nvim" || ln -sf "~/forks/NvChad" "~/.config/nvim"
                else
                  git clone --depth 1 "https://github.com/developing-today-forks/NvChad" "~/NvChad"
                fi

                if [ ! -d "~/NvChad/lua/custom" ]; then
                  git clone --depth 1 "https://github.com/developing-today-forks/NvChad-custom" "~/NvChad-custom"
                  ln -sf "~/NvChad-custom" "~/NvChad/lua/custom"
                fi
              fi
            '';
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

