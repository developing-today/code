  {
    description = "Home Manager Flake";

    inputs.nixpkgs.url = "github:NixOS/nixpkgs";
    inputs.home-manager.url = "github:nix-community/home-manager";

    outputs = { self, nixpkgs, home-manager, ... }: {
      homeManagerNixOsModules = stateVersion: [
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
    };
  }
