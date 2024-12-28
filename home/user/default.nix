{
  lib,
  inputs,
  stateVersion,
  pkgs,
  system,
  ...
}:
{
  imports = [ (lib.from-root "nixos/home") ];
  home-manager.users.user = {
    wayland.windowManager.hyprland = {
      enable = true;
      plugins = [ inputs.hypr-dynamic-cursors.packages.${pkgs.system}.hypr-dynamic-cursors ];
      extraConfig = builtins.readFile (lib.from-root "config/hypr/hyprland.conf");
    };
    # TODO: ensure home manager standalone can still work
    # TODO: factor out modules into shared files
    nixpkgs.config = {
      allowBroken = true;
      allowUnfree = true;
      allowUnfreePredicate = _: true;
      permittedInsecurePackages = [
        "olm-3.2.16"
        "electron" # le sigh
        "qtwebkit-5.212.0-alpha4" # ???
      ];
    };
    gtk = {
      enable = true;
      gtk3.extraConfig.gtk-decoration-layout = "menu:";
      cursorTheme.name = "Qogir";
      iconTheme.name = "Qogir";
      theme.name = "Jasper-Grey-Dark-Compact";
    };
    xdg = {
      enable = true;
      userDirs.enable = true;

      configFile."hypr" = {
        source = lib.from-root "config/hypr";
        recursive = true;
      };
      mimeApps.defaultApplications = {
        "application/x-extension-htm" = "firefox.desktop";
        "application/x-extension-html" = "firefox.desktop";
        "application/x-extension-shtml" = "firefox.desktop";
        "application/x-extension-xht" = "firefox.desktop";
        "application/x-extension-xhtml" = "firefox.desktop";
        "application/xhtml+xml" = "firefox.desktop";
        "text/html" = "firefox.desktop";
        "x-scheme-handler/chrome" = "firefox.desktop";
        "x-scheme-handler/http" = "firefox.desktop";
        "x-scheme-handler/https" = "firefox.desktop";
      };
    };
    services = {
      udiskie = {
        enable = true;
      };
      mako = {
        enable = true;
        anchor = "top-right";
        borderRadius = 0;
        borderSize = 0;
        padding = "0"; # within
        margin = "0"; # "36,0,0,0"; # outside # 36? 40?
        # margin = "36,0,0,0"; # outside # 36? 40?
        # .tabbrowser-tab[selected] {
        #   max-height: 24px !important;
        #   min-height: 24px !important;
        # }
        # tab:not([selected="true"]) {
        #   max-height: 24px !important;
        #   min-height: 24px !important;
        # }
        # maxIconSize = 256;
        maxIconSize = 512;
        ignoreTimeout = true;
        defaultTimeout = 15000;
        layer = "top";
        height = 240;
        width = 420;
        format = "<b>%s</b>\\n%b";
        backgroundColor = "#303030FF";
        borderColor = "#333333FF";
        # on-button-right=exec makoctl menu -n "$id" rofi -dmenu -p 'Select action: '
        # on-button-right=exec hyprctl setprop pid:$idhyprctl dispatch focuswindow
        # on-button-left=exec bash -c 'hyprctl dispatch focuswindow "pid:$1"' _ $id
        # on-button-right=exec bash -c 'hyprctl dispatch focuswindow "pid:$1"' _ $id
        # outside # 36? 40?12
        extraConfig = ''
          outer-margin=36,0,0,0

          [app-name="Element"]
          on-button-left=exec bash -c 'hyprctl dispatch workspace $(hyprctl -j clients | jq -r ".[] | select (.class == \"Element\") | .workspace.id")' _

          [urgency=low]
          default-timeout=10000

          [urgency=high]
          default-timeout=30000

          [mode=dnd]
          invisible=1
        '';
      };
      # dunst = {
      #   enable = true;
      #   package = pkgs.dunst;
      #   settings = {
      #     global = {
      #       monitor = 0;
      #       follow = "mouse";
      #       # border = 0;
      #       # height = 300;
      #       height = 360;
      #       # height = 400;
      #       # width = 320;
      #       # width = 420;
      #       # width = 480;
      #       # width = 240;
      #       # width = 320;
      #       offset = "0x0";
      #       # offset = "33x65";
      #       indicate_hidden = "yes";
      #       shrink = "yes";
      #       # shrink = "no";
      #       separator_height = 0;
      #       padding = 0;
      #       # padding = 32;
      #       # horizontal_padding = 32;
      #       horizontal_padding = 0;
      #       frame_width = 0;
      #       sort = "no";
      #       idle_threshold = 120;
      #       font = "Noto Sans";
      #       line_height = 4;
      #       markup = "full";
      #       format = "<b>%s</b>\\n%b";
      #       alignment = "left";
      #       # transparency = 10;
      #       transparency = 100;
      #       show_age_threshold = 60;
      #       word_wrap = "yes";
      #       ignore_newline = "no";
      #       stack_duplicates = false;
      #       hide_duplicate_count = "yes";
      #       show_indicators = "no";
      #       # icon_position = "off";
      #       icon_position = "left";
      #       icon_theme = "Adwaita-dark";
      #       sticky_history = "yes";
      #       history_length = 20;
      #       # browser = "google-chrome-stable";
      #       # browser = "firefox";
      #       browser = "${config.programs.firefox.package}/bin/firefox -new-tab";
      #       dmenu = "${pkgs.rofi-wayland}/bin/rofi -dmenu"; # wofi? etc.
      #       always_run_script = true;
      #       title = "Dunst";
      #       class = "Dunst";
      #       # max_icon_size = 64;
      #       max_icon_size = 128;
      #       # max_icon_size = 32;
      #       history = "ctrl+grave";
      #       context = "grave+space";
      #       close = "mod4+shift+space";
      #     };
      #   };
      # };
      activitywatch = {
        enable = true;
        package = inputs.nixpkgs-stable.legacyPackages.${pkgs.system}.activitywatch;
      };
    };
    manual.manpages.enable = true;
    programs = {
      ghostty = {
        enable = true;
        package = inputs.nixpkgs-master.legacyPackages.${system}.ghostty;
        settings = {
          # ghostty +list-themes
          theme = "synthwave";
          window-decoration = false;
        };
      };
      bash.enable = true;
      waybar = import (lib.from-root "home/common/programs/waybar.nix") { inherit pkgs; };
      alacritty = import (lib.from-root "home/common/programs/alacritty.nix");
      kitty = import (lib.from-root "home/common/programs/kitty.nix");
      yazi = import (lib.from-root "home/common/programs/yazi.nix") { inherit pkgs; };
      abook.enable = true;
      autojump.enable = true;

      autorandr.enable = true;
      # bash.enable = true; # bashrc overrides my bashrc hmmm
      bashmount.enable = true;
      # chromium.enable = true; # long build times
      dircolors.enable = true;
      direnv = {
        enable = true;
        enableZshIntegration = true;
      };
      emacs.enable = true;
      # eww.enable = true; # config
      #eza.enable = true;
      firefox.enable = true;
      fuzzel = {
        enable = true;
        settings = {
          main = {
            font = "Sarasa Mono SC";
            terminal = "foot";
            prompt = "->";
          };

          border = {
            width = 0;
            radius = 6;
          };

          dmenu = {
            mode = "text";
          };
          # colors = {
          #   background = "${config.color.base00}f2";
          #   text = "${config.color.base05}ff";
          #   match = "${config.color.base0A}ff";
          #   selection = "${config.color.base03}ff";
          #   selection-text = "${config.color.base05}ff";
          #   selection-match = "${config.color.base0A}ff";
          #   border = "${config.color.base0D}ff";
          # };
        };
      };
      fzf.enable = true;
      gh.enable = true;
      # git-credential-oauth.enable = true; # can't get browser to return back
      git = {
        # TODO: global config
        enable = true;
        lfs.enable = true;

        userName = "Drewry Pope";
        userEmail = "drewrypope@gmail.com";
        aliases = {
          ci = "commit";
          co = "checkout";
          s = "status";
        };
        # signing.signByDefault = true;
        # gitCliff
        # difftastic
        # diff-so-fancy
        # diff-highlight
        # delta
        # gitui
        #
        # attributes = [
        #   "*.pdf diff=pdf"
        # ];

        maintenance = {
          repositories = [ "/home/user/code" ];
          timers = {
            daily = "Tue..Sun *-*-* 0:53:00";
            hourly = "*-*-* 1..23:53:00";
            weekly = "Mon 0:53:00";
          };
        };
        extraConfig = {
          push = {
            autoSetupRemote = true;
          };
          pull = {
            rebase = true;
            # rebase = false;
            # ff = "only";
          };
          safe = {
            directory = "*";
          };
          help.autocorrect = "immediate";
          init.defaultBranch = "main";
          #   credential.helper = "${
          #       pkgs.git.override { withLibsecret = true; }
          #     }/bin/git-credential-libsecret";
        };
      };
      gitui.enable = true;
      # gnome-terminal.enable = true; # strange error, probably because i'm not using gnome. interesting.
      go.enable = true;
      gpg.enable = true;
      havoc.enable = true;
      #     helix.enable = true; # try again vs binary? didn't like editor override.
      hexchat.enable = true;
      # htop.enable = true;
      i3status-rust.enable = true;
      i3status.enable = true;
      info.enable = true;
      irssi.enable = true;
      java.enable = true;
      jq.enable = true;
      jujutsu.enable = true;
      # just.enable = true;
      kakoune.enable = true;
      #kitty.enable = true;
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
      # password-store.enable = true;
      powerline-go.enable = true;
      #pyenv.enable = true;
      pylint.enable = true;
      pywal.enable = true;
      rbenv.enable = true;
      readline.enable = true;
      #ripgrep.enable = true;
      rtorrent.enable = true;
      # sagemath.enable = true; # oh my god 1 hour + build times and then it usually fails. if it's cached you're fine but on unstable it is just not always cached. even worse against master branch
      ssh.enable = true;
      starship.enable = true;
      swaylock.enable = true;
      taskwarrior = {
        enable = true;
        package = pkgs.taskwarrior3;
      };
      tealdeer.enable = true;
      terminator.enable = true;
      termite.enable = true;
      #texlive.enable = true; # failed on wsl
      # thunderbird.enable = true;
      tiny.enable = true;
      tmate.enable = true;
      # tmux.enable = true;
      # vim-vint.enable = true;
      # vim.enable = true;
      # vscode.enable = true;
      wlogout.enable = true;
      zathura.enable = true;
      zellij.enable = true;
      zoxide.enable = true;
      # zplug.enable = true;
      nushell = {
        enable = true;
        environmentVariables = {
          NIXOS_OZONE_WL = "1";
          ELECTRON_OZONE_PLATFORM_HINT = "auto";
          EDITOR = "nvim";
          VISUAL = "nvim";
          TERM = "kitty"; # alacritty";
        };
        shellAliases = {
          #switch = "sudo nixos-rebuild switch";
        };
        extraConfig = ''
          $env.config = {
            show_banner: false,
          }
        '';
      };
      #oils-for-unix.enable = true;
      obs-studio.enable = true;
      oh-my-posh.enable = true;
      fish.enable = true;
      bat.enable = true;
      zsh = {
        enable = true;
        oh-my-zsh = {
          enable = true;
          plugins = [
            "git"
            "python"
            "docker"
            "fzf"
          ];
          theme = "dpoggi";
        };
      };
      htop = {
        enable = true;
        settings = {
          delay = 10;
          show_program_path = false;
          show_cpu_frequency = true;
          show_cpu_temperature = true;
          hide_kernel_threads = true;
          leftMeters = [
            "AllCPUs2"
            "Memory"
            "Swap"
          ];
          rightMeters = [
            "Hostname"
            "Tasks"
            "LoadAverage"
            "Uptime"
            "Systemd"
          ];
        };
      };
      tmux = {
        enable = true;
        # setw -g mouse on
      };
      password-store = {
        enable = true;
        settings = {
          PASSWORD_STORE_DIR = "$XDG_DATA_HOME/password-store";
        };
      };
    };
    home = {
      inherit stateVersion;
      shellAliases = {
        l = "exa";
        ls = "exa";
        cat = "bat";
      };
      sessionVariables = {
        EDITOR = "nvim";
        TERM = "kitty"; # "alacritty" "xterm-256color"
        # PATH = "$HOME/bin:$PATH";
        NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
        NIXPKGS_ALLOW_UNFREE = "1";
        # XDG_CACHE_HOME = "$HOME/.cache";
        # XDG_CONFIG_DIRS = "/etc/xdg";
        # XDG_CONFIG_HOME = "$HOME/.config";
        # XDG_DATA_DIRS = "/usr/local/share/:/usr/share/";
        # XDG_DATA_HOME = "$HOME/.local/share";
        # XDG_STATE_HOME = "$HOME/.local/state";
      };
      # sessionPath = [
      #   "$HOME/bin"
      #   "$HOME/.local/bin"
      # ];
      pointerCursor = {
        package = pkgs.vanilla-dmz;
        name = "Vanilla-DMZ";
        gtk.enable = true;
        size = 24;
        x11.enable = true;
      };
      file.".config/nixpkgs/config.nix".text = ''
        {
          allowUnfree = true;
        }
      '';
      packages =
        with pkgs;
        [
          libnotify
          #
          #         dog
          #         felix
          #         figlet/*
          #         gcc
          #         helix
          #         hex
          #         lolcat
          #         lolcat*/*/
          #         nodePackages.prettier
          #         oh-my-zsh
          #         polybar
          #         python-debug
          #         rofi
          #         tldr
          #         waybar-hyprland-git
          #       swayidledd
          #     configure-gtk
          #     dbus-sway-environment
          #     hyprland
          #     inputs.hyprwm-contrib.packages.${system}.grimblast
          #  1history
          #  astro
          #  cakawka
          #  calculator
          #  cicada
          #  counts
          #  cpc
          #  delicate
          #  dtrace
          #  dua-cli
          #  dust du-dust above
          #  floki
          #  frum
          #  hashguard
          #  kani-verifier
          #  legdur
          #  lemmy
          #  medic
          #  mrml
          #  nat
          #  notty
          #  opentelemetry
          #  oreboot
          #  pepper
          #  pleco
          #  printfn
          #  qsv
          #  rip
          #  rustodon
          #  stringsext
          #  teehee
          #  tv-renamer
          #  voila
          #  weld
          #  xi
          #  zh
          # Bash
          # Command Shells
          # Core Packages
          # Dart
          # Development
          # Elixir
          # Erlang
          # Files
          # Haskell
          # Joke/*s
          # Language Servers
          # Lua
          # Media
          # My Packages
          # My Proprietary Packages
          # Nix
          # Overview
          # Programming Languages
          # Python
          # QT
          # Rust CLI Tools! I love rust.
          # Standard Packages
          # Telescope tools
          # These are so intellij file watchers has something to use
          # Typescript
          # Web (ESLint, HTML, CSS, JSON)
          # Xorg Stuff :-(
          # bandwhich # isn't working right?
          # calibre
          # cliphist
          # egui_graphs
          # fenix
          # frolic
          # hot-lib-reloader
          # https://github.com/Inlyne-Project/inlyne/issues/356
          # https://github.com/NixOS/nixpkgs/issues/332957
          # hyprland-share-picker
          # inlyne # rust 1.80
          # intelli-shell
          # libsForQt6.qt6.qtwayland
          # lua
          # mlocate # shadowed by plocate
          # neovim
          # oil # try again later
          # plotlib
          # plotly
          # python.pkgs.pip
          # qt5-wayland
          # qt6-wayland
          # ripgrep-all # regression cannot find hello 26 times 2023-08-19
          # ripsecrets
          # rmesg # unknown
          # rpn
          # rustfix
          # soup
          # sqlitecpp
          # tldr # shadowed by tealdeer
          # todo figure out how to use sway
          # trustfall
          # vim-racer
          # xd # i don't know what this is
          ## Desktop Environments
          ## Go
          ## Libraries
          ## Programs
          ## Rust
          ## Window Managers
          ## block ick
          ## endblock ick
          #awesome
          #cargo-graph
          #cinnamon.cinnamon-desktop
          #duckdb # long compile todo
          #dust # abandoned
          #element-desktop # build time long, electron bad
          #eww-wayland
          #exa
          #eza # exa # ls
          #fh # ffi parse failure
          #fprint
          #gnupg
          #monero-gui
          #neofetch
          #neovim
          #nixfmt
          #nodePackages.pyright
          #nushell
          #oil # oil is python oils-for-unix is cpp
          #pinentry
          #pinentry-qt
          #plasma5Packages.kdenlive # build failures? maybe need plasma6?
          #pyright
          #qtcreator
          #rnix-lsp
          #signal-desktop
          #skypeforlinux
          #slack
          #tabnine
          #tdesktop
          #terraform
          #tor-browser-bundle-bin
          #tp-note # unknown
          #trash-cli
          #vim
          #vimPlugins.cmp-tabnine
          #vimPlugins.coc-tabnine
          #vimPlugins.copilot-cmp
          #vimPlugins.nvim-cmp
          #vimPlugins.nvim-treesitter-parsers.toml
          #vimPlugins.nvim-treesitter-parsers.typescript
          #vimPlugins.tabnine-vim
          #vimPlugins.telescope-zoxide
          #vimPlugins.vim-prettier
          #vimPlugins.vim-toml
          #vimPlugins.zoxide-vim
          #vscode
          #vscode-insiders
          #waybar-hyprland
          #ytop # abandoned
          #zoom-us
          acpi
          acpitool
          adwaita-icon-theme # default gnome cursors
          alacritty
          alacritty # gpu accelerated terminal
          alsa-lib
          amp
          any-nix-shell
          arandr
          atuin
          audacity
          autojump
          autorandr
          awscli
          bacon
          bat
          bat # cat
          beam.packages.erlang.elixir-ls
          beam.packages.erlang.erlang-ls
          beep
          bemenu # wayland clone of dmenu
          bingrep
          bitwarden
          bitwarden-cli
          bitwarden-desktop
          bitwarden-menu
          black
          blink1-tool
          blueman
          bluez
          bluez-tools
          bottom
          brave
          brig
          brightnessctl
          brillo
          broot
          bspwm
          btop
          cachix
          cargo
          cargo-audit
          cargo-binstall
          cargo-crev
          cargo-geiger
          cargo-wipe
          cava
          ccls
          celluloid
          charm
          charm-freeze
          choose
          cmake
          cmatrix
          conform
          consul
          coreutils
          cpufetch
          curl
          dart
          dash
          delta # better diff
          deno
          difftastic
          direnv
          discord
          diskonaut
          dmenu
          dnsutils
          docker-compose
          dogdns # dns for dogs
          dolphin
          dprint
          dracula-theme # gtk theme
          drill
          du-dust
          dua
          dunst
          dwm
          elinks
          elmPackages.elm-format
          endlessh
          espeak
          eva
          eww
          eza
          fastmod
          fblog
          fclones
          fd
          fd # replace find
          feh
          fend
          ffsend
          flameshot
          flatpak
          fnm
          fontconfig
          fontfinder
          freetype
          fselect
          furtherance
          fw
          fzf
          gh
          gimp
          git
          git-absorb
          git-cliff
          git-crypt
          gitAndTools.diff-so-fancy
          github-desktop
          gitui
          glib
          glib # gsettings
          glibc
          gnugrep
          gnumake
          gnupg
          gnused
          go
          gparted
          gptman
          grex # ya grep
          grim
          grim # screenshot functionality
          gthumb
          gtklock
          haskellPackages.haskell-language-server
          hck
          helix # neovim 2
          hexyl
          himalaya
          html-tidy
          htmlq
          htop
          htop # top for humans
          huniq
          hyperfine
          hyprdim
          hyprland-autoname-workspaces
          hyprland-per-window-layout
          hyprland-protocols
          hyprpaper
          imagemagick
          intel-gpu-tools
          ion
          ipfs
          jack2
          jetbrains-mono
          jless
          jq
          jql
          just
          k9s
          kalker
          kdash
          kibi
          kickoff
          killall
          kitty
          kondo
          krabby
          lagrange
          lapce
          lazygit
          lazygit # command line git ui
          lefthook
          lemmeknow
          less
          lf
          lfs
          libnotify
          libsForQt5.polkit-kde-agent
          libsForQt5.qt5.qtwayland
          libsForQt5.yakuake
          libtool
          libva-utils
          libverto
          licensor
          light
          lld
          lmms
          loc
          lsd
          lua-language-server
          lxde.lxsession
          macchina
          # mako # notification system developed by swaywm maintainer
          mangohud
          mask
          mcfly
          mdbook
          mdcat
          miniserve
          mkcert
          monolith
          mosh
          mpv
          nano
          navi
          ncspot
          neofetch # sysinfo
          neovim
          networkmanager
          networkmanagerapplet
          nfs-utils
          nickel
          nil
          ninja
          nitrogen
          nix-init
          nix-melt
          nixfmt-rfc-style
          nixpkgs-fmt # ??
          nodePackages.bash-language-server
          nodePackages.eslint
          nodePackages.prettier
          nodePackages.prettier-plugin-toml
          nodePackages.typescript
          nodePackages.typescript-language-server
          nodePackages.vercel
          nodePackages.vscode-langservers-extracted
          nodePackages.wrangler
          nodejs
          nodejs-18_x
          nomacs
          nomino
          nsh
          nurl
          nwg-displays
          nwg-dock-hyprland
          oh-my-fish
          openconnect
          openssl
          ormolu
          ouch
          packer
          pastel
          pavucontrol
          pciutils
          pgfplots
          picom
          pinentry-all # TODO: consider pinentry though gnupg services service, also consider whether this should be global and not for 'user'
          pinentry-rofi
          pipewire
          pipr
          pkg-config
          please
          plocate
          pls
          polkit_gnome
          polybarFull
          powershell
          procps
          procs
          procs # replace proc
          pstree
          pueue
          pulseaudio
          pv
          pwgen
          python3Full
          qt5.qmake
          qt5.qtwayland
          libsForQt5.qt5ct
          qt6.qmake
          qt6.qtwayland
          qt6ct
          qtractor
          racer
          ranger # midnight commander / file manager
          rargs
          rbw
          restic
          ripgrep
          rmtrash # ctrl + z for rm
          rnr
          rofi-rbw
          rofi-wayland
          rufo
          runiq
          rust-analyzer
          rustc
          rustdesk
          rustfmt
          sad
          sd
          shellcheck
          shellharden
          shfmt
          silver-searcher
          skim
          slack
          slurp
          slurp # screenshot functionality
          sops
          sox
          spotify
          sqlite
          st
          starship
          starship # replacement prompt (not shell)
          statix
          stdenv
          steam
          sway
          swaycons
          swayidle
          swaylock
          swaynotificationcenter
          swww
          sxhkd
          synergy
          systeroid
          tealdeer # ya tldr
          #terminus-nerdfont
          thefuck
          tidy-viewer
          tidyp
          tig # command line git
          tiny
          tmux
          tmuxPlugins.continuum
          tmuxPlugins.resurrect
          toilet
          tokei
          tokei # this gives language stats about a repo
          topgrade
          transmission_4-gtk
          trash-cli
          tree
          tree-sitter
          treefmt
          trippy
          udev
          universal-ctags # for nvim nvchad custom
          unzip
          vanilla-dmz
          variety
          vault
          vaultwarden
          vdpauinfo
          vlc # build time long, vlc good, try again later
          volta
          w3m
          warp
          watchexec
          wdisplays
          wdisplays # tool to configure displays
          wezterm
          wget
          whois
          wireplumber
          wl-clipboard
          wl-clipboard # wl-copy and wl-paste for copy/paste from stdin / stdout
          wofi
          wtf
          wttrbar
          xclip
          xcp
          xdg-desktop-portal
          xdg-desktop-portal-hyprland
          xdg-utils
          xdg-utils # for opening default programs when clicking links
          xfce.thunar
          xh
          xorg.libX11
          xorg.libXcursor
          xournalpp # xournal
          xsv
          yad
          yarn
          ydotool
          yt-dlp # youtube-dl
          zee
          zellij
          zellij # tmux
          zip
          zlib
          zoom-us
          zoxide
          zstd
          zsv
          zulip
          zulip-term
        ]
        ++ [
          ## R
          (pkgs.rWrapper.override {
            packages = with pkgs.rPackages; [
              dplyr
              xts
              ggplot2
              reshape2
            ];
          })
          #(pkgs.rstudioWrapper.override {
          #  packages = with pkgs.rPackages; [
          #    dplyr
          #    xts
          #    ggplot2
          #    reshape2
          #    rstudioapi
          #  ];
          #})
        ];
    };
  };
}
