{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    zig-overlay = {
      url = "github:mitchellh/zig-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, zig-overlay, neovim-nightly-overlay, flake-utils, home-manager, ... }:
  let
    stateVersion = "23.11";
    overlays = [
      zig-overlay.overlays.default
      neovim-nightly-overlay.overlay
    ];
    systemNixOsModules = [ {
      nixpkgs.overlays = overlays;
      system.stateVersion = stateVersion;
    } ./configuration.nix ];
    hyprlandNixOsModules = [ {
      programs = {
        hyprland = {
          enable = true;
          # nvidiaPatches = true; # ONLY use this line if you have an nvidia card
        };
      };
    } ];
    system = "x86_64-linux";
    pkgs = import nixpkgs { system = system; overlays = overlays; };
    homeManagerNixOsModules = [ ({ pkgs, ... }: {
      imports = [
        home-manager.nixosModules.home-manager
      ];
      home-manager.users.user = {
        home = {
          stateVersion = stateVersion;
          shellAliases = {
            l = "exa";
            ls = "exa";
            cat = "bat";
          };
          sessionVariables = {
            EDITOR="nvim";
          };
        };
        programs = {
          waybar = {
            enable = true;
            package = pkgs.waybar-hyprland.overrideAttrs (oldAttrs: {
              mesonFlags = oldAttrs.mesonFlags ++ [ "-Dexperimental=true" ];
            });
          };
          neovim = {
            enable = true;
            defaultEditor = true;
            viAlias = true;
            vimAlias = true;
            vimdiffAlias = true;
            package = pkgs.neovim-nightly;
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
          zsh = {
            enable = true;
              oh-my-zsh= {
              enable = true;
              plugins = ["git" "python" "docker" "fzf"];
              theme = "dpoggi";
            };
          };
        };
      };
    }) ];
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
