{
  stateVersion,
  pkgs,
  ...
}: {
  nixpkgs.config.allowUnfree = true;

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
      source = ../config/hypr;
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
    packages = with pkgs;
      [
        neovim
      ]
      ++ [
        lf
        #
        git
        kitty
        direnv
        #vscode-insiders
        openssl
        libsForQt5.yakuake
        tmux
        alacritty
        ripgrep
        # ripgrep-all # regression cannot find hello 26 times 2023-08-19
        zoxide
        #vimPlugins.zoxide-vim
        #vimPlugins.telescope-zoxide
        starship # replacement prompt (not shell)
        hyperfine
        # todo figure out how to use sway
        zellij # tmux
        #eza # exa # ls
        eza
        rmtrash # ctrl + z for rm
        mcfly
        dogdns # dns for dogs
        htop # top for humans
        lazygit # command line git ui
        tig # command line git
        helix # neovim 2
        ranger # midnight commander / file manager
        bat # cat
        # xd # i don't know what this is
        procs # replace proc
        sd
        yarn
        fd # replace find
        #dust # abandoned
        tokei # this gives language stats about a repo
        #ytop # abandoned
        tealdeer # ya tldr
        # tldr # shadowed by tealdeer
        # bandwhich # isn't working right?
        grex # ya grep
        # rmesg # unknown
        delta # better diff
        #tp-note # unknown
        # oil # try again later
        plocate
        # mlocate # shadowed by plocate
        universal-ctags # for nvim nvchad custom
        neofetch # sysinfo
        awscli
        jq
        fd
        htop
        tree
        curl
        wget
        #         helix
        pstree
        mkcert
        gitAndTools.diff-so-fancy
        ripgrep
        lazygit
        thefuck
        tree-sitter
        gnupg
        gnused
        silver-searcher
        fzf
        less
        git
        git-absorb
        tmux
        tmuxPlugins.continuum
        tmuxPlugins.resurrect
        openconnect
        jetbrains-mono
        go
        sops
        k9s
        unzip
        gnumake
        cmake
        deno
        #tabnine
        #vimPlugins.cmp-tabnine
        #vimPlugins.tabnine-vim
        #vimPlugins.nvim-cmp
        #vimPlugins.copilot-cmp
        #vimPlugins.coc-tabnine
        sqlite
        # sqlitecpp
        lua-language-server
        # lua
        #         gcc
        ## block ick
        # These are so intellij file watchers has something to use
        nodejs-18_x
        #         nodePackages.prettier
        nodePackages.wrangler
        nodePackages.vercel
        #vimPlugins.nvim-treesitter-parsers.typescript
        nodePackages.typescript
        nodePackages.typescript-language-server
        ## endblock ick

        # Rust CLI Tools! I love rust.
        #exa
        bat
        tokei
        xsv
        fd

        # Development
        # neovim
        tmux
        jq
        git-crypt
        dnsutils
        whois

        # Files
        zstd
        restic
        brig
        ipfs

        # Media
        youtube-dl
        imagemagick

        # Overview
        htop
        wtf
        lazygit
        #neofetch

        # Joke/*s
        #         figlet/*
        #         lolcat*/*/

        tree-sitter
        nodejs
        # Language Servers
        # Bash
        nodePackages.bash-language-server
        # Dart
        dart
        # Elixir
        beam.packages.erlang.elixir-ls
        # Erlang
        beam.packages.erlang.erlang-ls
        # Haskell
        haskellPackages.haskell-language-server
        # Lua
        lua-language-server
        # Nix
        nil
        nixpkgs-fmt
        statix
        # Python
        #pyright
        #         python-debug
        black
        # Typescript
        nodePackages.typescript-language-server
        # Web (ESLint, HTML, CSS, JSON)
        nodePackages.vscode-langservers-extracted
        # Telescope tools
        ripgrep
        fd

        nickel

        plasma5Packages.kdenlive
        git
        git-crypt
        gnupg
        audacity
        gimp
        nano
        qtractor
        jack2
        lmms

        #rnix-lsp
        #         gcc
        ripgrep
        fd
        #nodePackages.pyright
        nodePackages.eslint
        ccls

        zlib
        dmenu
        arandr
        picom
        espeak
        blink1-tool
        flameshot
      ]
      ++ [
        alacritty
        autorandr
        brightnessctl
        feh
        wget
        pciutils
        intel-gpu-tools
        killall
        libva-utils
        spotify
        #skypeforlinux
        zoom-us
        slack
        docker-compose
        direnv
        pavucontrol
        pulseaudio
        dunst
        cmake
        #         gcc
        gnumake
        libtool
        vdpauinfo
      ]
      ++ [
        glib
        grim
        slurp
        sway
        #       swayidledd
        swaylock
        wdisplays
        wl-clipboard
      ]
      ++ [
        #vim
        wget
        w3m
        dmenu
        #neofetch
        #neovim
        autojump
        starship
        brave
        bspwm
        celluloid
        dwm
        dunst
        elinks
        feh
        flameshot
        flatpak
        fontconfig
        freetype
        #         gcc
        gh
        gimp
        git
        github-desktop
        gnugrep
        gnumake
        gparted
        kitty
        libverto
        mangohud
        #neovim
        nfs-utils
        ninja
        nodejs
        nomacs
        openssl
        pavucontrol
        picom
        polkit_gnome
        powershell
        python3Full
        # python.pkgs.pip
        ripgrep
        sxhkd
        st
        stdenv
        synergy
        swaycons
        terminus-nerdfont
        #         tldr
        trash-cli
        unzip
        variety
        #vscode
        xclip
      ]
      ++ [
        alacritty # gpu accelerated terminal
        #     dbus-sway-environment
        #     configure-gtk
        xdg-utils # for opening default programs when clicking links
        glib # gsettings
        dracula-theme # gtk theme
        gnome3.adwaita-icon-theme # default gnome cursors
        swaylock
        swayidle
        grim # screenshot functionality
        slurp # screenshot functionality
        wl-clipboard # wl-copy and wl-paste for copy/paste from stdin / stdout
        bemenu # wayland clone of dmenu
        mako # notification system developed by swaywm maintainer
        wdisplays # tool to configure displays
      ]
      ++ [
        # Core Packages
        lld
        #         gcc
        glibc

        udev
        wget
        procps
        killall
        zip
        unzip
        bluez
        bluez-tools
        libnotify
        brightnessctl
        light
        xdg-desktop-portal
        xdg-utils
        pipewire
        wireplumber
        alsaLib
        pkg-config

        # Standard Packages
        networkmanager
        networkmanagerapplet
        git
        fzf
        #vim
        #         tldr
        sox
        yad
        flatpak

        # QT
        #qtcreator
        qt5.qtwayland
        # qt5-wayland
        qt5.qmake
        libsForQt5.qt5.qtwayland
        qt5ct

        qt6.qtwayland
        # qt6-wayland
        qt6.qmake
        # libsForQt6.qt6.qtwayland
        qt6ct

        # My Packages
        #         helix
        brave
        xfce.thunar
        kitty
        bat
        #exa
        pavucontrol
        blueman
        #trash-cli
        ydotool
        cava
        #neofetch
        cpufetch
        starship
        #         lolcat
        gimp
        transmission-gtk
        slurp
        gparted
        vlc
        mpv
        krabby
        zellij
        shellcheck
        thefuck
        gthumb
        cmatrix
        lagrange

        # My Proprietary Packages
        discord
        steam
        spotify

        # Xorg Stuff :-(
        ## Libraries
        xorg.libX11
        xorg.libXcursor
        ## Window Managers
        #awesome
        ## Desktop Environments
        cinnamon.cinnamon-desktop
        ## Programs
        nitrogen
        picom
        dunst
        flameshot

        # Programming Languages
        ## Rust
        cargo
        rustc
        rust-analyzer
        ## Go
        go
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

        # Command Shells
        nushell

        #     hyprland
        cliphist
        alacritty
        #         rofi
        rofi-wayland
        swww
        swaynotificationcenter
        lxde.lxsession
        #     inputs.hyprwm-contrib.packages.${system}.grimblast
        gtklock
        eww
        #eww-wayland
        xdg-desktop-portal-hyprland
        hyprland-protocols
        # hyprland-share-picker
        hyprland-autoname-workspaces
        hyprland-per-window-layout
        nwg-displays
        nwg-dock-hyprland
        hyprdim
        #waybar-hyprland
        wttrbar
        #         polybar
        polybarFull
        endlessh
      ]
      ++ [
        oil
        toilet
        nsh
        lsd
        xcp
        du-dust
        zoxide
        fd
        sd
        procs
        bottom
        topgrade
        broot
        kdash
        xh
        monolith
        # ripsecrets
        eva
        atuin
        bat
        #  dust du-dust above
        fd
        fend
        hyperfine
        miniserve
        ripgrep
        just
        cargo-audit
        cargo-wipe
        watchexec
        #  calculator
        fselect
        #  teehee
        # rpn
        kalker
        kondo
        rnr
        pipr
        #  stringsext
        himalaya
        topgrade
        ncspot
        loc
        difftastic
        html-tidy
        tidyp
        tidy-viewer
        #  medic
        amp
        kibi
        lapce
        #  pepper
        #  xi
        zee
        wezterm
        cargo-geiger
        cargo-crev
        bacon
        cargo-binstall
        #  kani-verifier
        #  printfn
        fend
        zsv
        #  zh
        skim
        sd
        #  qsv
        rargs
        #  rip
        #  qsv
        procs
        pipr
        pastel
        ouch
        miniserve
        mdcat
        mdbook
        macchina
        lfs
        lemmeknow
        #  legdur
        just
        jql
        jless
        inlyne
        htmlq
        gptman
        git-cliff
        #  frum
        ffsend
        #         felix
        fclones
        dua
        #         dog
        choose
        #  counts
        #  pleco
        drill
        ion
        #  cpc
        #  oreboot
        xcp
        hck
        skim
        coreutils
        hexyl
        #  nat
        fnm
        volta
        sad
        systeroid
        please
        #  stringsext
        navi
        huniq
        fastmod
        furtherance
        gitui
        #  lemmy
        deno
        #  astro
        #  mrml
        shellharden
        dprint
        #  dtrace
        #  floki
        #  hashguard
        dua
        #  dua-cli
        tiny
        #  notty
        #  weld
        #  opentelemetry
        #  rustodon
        #  voila
        fblog
        diskonaut
        kondo
        kickoff
        #  cicada
        bingrep
        fontfinder
        #  tv-renamer
        trippy
        #         hex
        ion
        #  cakawka
        pueue
        #  delicate
        runiq
        #  1history
        nix-init
        nix-melt
        nurl
        nomino
        licensor
        rustdesk
        warp
        # intelli-shell
        git-cliff
        fw
        # frolic
        mask
        # hot-lib-reloader
        racer
        # rustfix
        #cargo-graph
        # fenix
        # vim-racer
        # egui_graphs
        pgfplots
        # plotly
        # plotlib
        #duckdb # long compile todo
        # trustfall
        # soup
        libsForQt5.polkit-kde-agent
        wireplumber
        dunst
        mako
        ion
        dash
        pls
        #         oh-my-zsh
        oh-my-fish
      ]
      ++ [
        hyprpaper
        acpi
        acpitool
        dolphin
        wofi

        wl-clipboard
        wdisplays

        spotify
        vlc

        #signal-desktop
        #tdesktop
        #element-desktop # build time long, electron bad
        tor-browser-bundle-bin
        monero-gui
        discord
        #zoom-us
        # calibre
        #slack
        xournal
        xdg-utils

        bat
        pv
        #exa
        ripgrep
        pwgen
        docker-compose
        tmux
        btop
        mosh
        any-nix-shell
        #         waybar-hyprland-git
        treefmt
        lefthook
        conform
        #vimPlugins.vim-toml
        #vimPlugins.vim-prettier
        #vimPlugins.nvim-treesitter-parsers.toml
        nodePackages.prettier
        nodePackages.prettier-plugin-toml
        shfmt
        rustfmt
        rufo
        black
        #terraform
        packer
        consul
        vault
        elmPackages.elm-format
        ormolu
        cachix
        brillo
        beep
        fh
      ];
  };
  manual.manpages.enable = true;
  programs = {
    waybar = import ../programs/waybar.nix {inherit pkgs;};
    alacritty = import ../programs/alacritty.nix;
    # neovim = import ../programs/nvim.nix {inherit pkgs;};
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
    git.enable = true;
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
    taskwarrior.enable = true;
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
    fish.enable = true;
    nushell.enable = true;
    obs-studio.enable = true;
    oh-my-posh.enable = true;
    bat.enable = true;
    zsh = {
      enable = true;
      oh-my-zsh = {
        enable = true;
        plugins = ["git" "python" "docker" "fzf"];
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
}
