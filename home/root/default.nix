{
  lib,
  inputs,
  stateVersion,
  pkgs,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/home")
  ];
  home-manager.users.root = { # TODO: ensure home manager standalone can still work
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
        source = ../../config/hypr;
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
    manual.manpages.enable = true;
    programs = {
      waybar = import ../../home/common/programs/waybar.nix { inherit pkgs; };
      alacritty = import ../../home/common/programs/alacritty.nix;
      kitty = import ../../home/common/programs/kitty.nix;
      yazi = import ../../home/common/programs/yazi.nix { inherit pkgs; };
      # neovim = import programs/nvim.nix {inherit pkgs;};
      # nixvim.enable = true;
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
      fzf.enable = true;
      gh.enable = true;
      # git-credential-oauth.enable = true; # can't get browser to return back
      git = {
        enable = true;
        lfs.enable = true;

        userName  = "Drewry Pope";
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
          repositories = [
            "/home/user/code"
          ];
          timers = {
            daily = "Tue..Sun *-*-* 0:53:00";
            hourly = "*-*-* 1..23:53:00";
            weekly = "Mon 0:53:00";
          };
        };
        extraConfig = {
              push = { autoSetupRemote = true; };
              safe = {
                directory = "/home/code/user";
              };
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
      };
      pointerCursor = {
        package = pkgs.vanilla-dmz;
        name = "Vanilla-DMZ";
        gtk.enable = true;
        size = 24;
        x11.enable = true;
      };
      packages =
        with pkgs;
        [
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
          alsaLib
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
          mako
          mako # notification system developed by swaywm maintainer
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
          qt5ct
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
          terminus-nerdfont
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
          xournal
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
        ] ++ [
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
