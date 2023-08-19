{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";

    #  hardware.url = "github:nixos/nixos-hardware";
    flake-utils.url = "github:numtide/flake-utils";
    # must type this all out every time
    # because flake inputs are basically static
    # can't make a let var function closure thing around it or whatever
    zig-overlay = {
      #      url = "github:mitchellh/zig-overlay"; # https://github.com/mitchellh/zig-overlay/pull/27
      url = "github:developing-today-forks/zig-overlay/quote-urls";
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
    # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake
    alejandra = {
      # url = "github:kamadorueda/alejandra"; # https://github.com/kamadorueda/alejandra/pull/398
      url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # nix-rice = https://github.com/bertof/nix-rice
  };
  outputs = {
    self,
    nixpkgs,
    zig-overlay,
    neovim-nightly-overlay,
    flake-utils,
    home-manager,
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
      ./configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
    ];
    # overlayNixOsModules = ?
    hyprlandNixOsModules = [
      {
        programs = {
          hyprland = {
            enable = true;
            # nvidiaPatches = true; # ONLY use this line if you have an nvidia card
          };
        };
      }
    ];
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      system = system;
      overlays = overlays; # are overlays needed in home manager? document which/why?
    };
    homeManagerNixOsModules = [
      ({pkgs, ...}: {
        imports = [
          home-manager.nixosModules.home-manager
        ];
        home-manager.users.user = {
          nixpkgs.config.allowUnfree = true;
          home = {
            stateVersion = stateVersion;
            shellAliases = {
              l = "exa";
              ls = "exa";
              cat = "bat";
            };
            sessionVariables = {
              EDITOR = "nvim";
            };
          };
          programs = {
            waybar = {
              enable = true;
              package = pkgs.waybar-hyprland.overrideAttrs (oldAttrs: {
                mesonFlags = oldAttrs.mesonFlags ++ ["-Dexperimental=true"];
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
            abook.enable = true;
            autojump.enable = true;
            autorandr.enable = true;
            # bash.enable = true; # bashrc overrides my bashrc hmmm
            bashmount.enable = true;
            chromium.enable = true;
            dircolors.enable = true;
            direnv.enable = true;
            emacs.enable = true;
            # eww.enable = true; # config
            exa.enable = true;
            firefox.enable = true;
            fzf.enable = true;
            gh.enable = true;
            # git-credential-oauth.enable = true; # can't get browser to return back
            git.enable = true;
            gitui.enable = true;
            # gnome-terminal.enable = true; # strange error, probably because i'm not using gnome. interesting.
            go.enable = true;
            gpg.enable = true;
            havoc.enable = true;
            helix.enable = true;
            hexchat.enable = true;
            htop.enable = true;
            i3status-rust.enable = true;
            i3status.enable = true;
            info.enable = true;
            irssi.enable = true;
            java.enable = true;
            jq.enable = true;
            jujutsu.enable = true;
            #          just.enable = true;
            kakoune.enable = true;
            kitty.enable = true;
            lazygit.enable = true;
            ledger.enable = true;
            less.enable = true;
            lesspipe.enable = true;
            lf.enable = true;
            man.enable = true;
            matplotlib.enable = true;
            mcfly.enable = true;
            # mercurial.enable = true; # config
            pandoc.enable = true;
            password-store.enable = true;
            powerline-go.enable = true;
            pyenv.enable = true;
            pylint.enable = true;
            pywal.enable = true;
            rbenv.enable = true;
            readline.enable = true;
            ripgrep.enable = true;
            rtorrent.enable = true;
            sagemath.enable = true;
            ssh.enable = true;
            starship.enable = true;
            swaylock.enable = true;
            taskwarrior.enable = true;
            tealdeer.enable = true;
            terminator.enable = true;
            termite.enable = true;
            texlive.enable = true;
            #          thunderbird.enable = true;
            tiny.enable = true;
            tmate.enable = true;
            tmux.enable = true;
            vim-vint.enable = true;
            vim.enable = true;
            #          vscode.enable = true;
            wlogout.enable = true;
            zathura.enable = true;
            zellij.enable = true;
            zoxide.enable = true;
            #          zplug.enable = true;
            fish.enable = true;
            nushell.enable = true;
            obs-studio.enable = true;
            oh-my-posh.enable = true;
            alacritty.enable = true;
            bat.enable = true;

            zsh = {
              enable = true;
              oh-my-zsh = {
                enable = true;
                plugins = ["git" "python" "docker" "fzf"];
                theme = "dpoggi";
              };
            };
          };
        };
      })
    ];
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
