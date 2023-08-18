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
    hyprland = {
      url = "github:hyprwm/Hyprland";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, zig-overlay, neovim-nightly-overlay, flake-utils, home-manager, hyprland, ... }:
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
    };
  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        ./configuration.nix
        hyprland.nixosModules.default
        {
          programs.hyprland = {
            enable = true;
            xwayland.enable = true;
#             nvidiaPatches = true; # ONLY use this line if you have an nvidia card
          };
        }
        {
          nixpkgs.overlays = [
            (final: prev: { zigpkgs = zig-overlay.packages.${prev.system}; })
            overlay
          ];
          system.stateVersion = stateVersion;
        }
        homeManagerConfiguration
      ];
    };
  };
}
