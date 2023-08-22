{
  config,
  pkgs,
  ...
}: let
  vscode-insiders = (pkgs.vscode.override {isInsiders = true;}).overrideAttrs (oldAttrs: rec {
    src = builtins.fetchTarball {
      url = "https://code.visualstudio.com/sha/download?build=insider&os=linux-x64";
      sha256 = "16fzxqs6ql4p2apq9aw7l10h4ag1r7jwlfvknk5rd2zmkscwhn6z";
    };
    version = "latest";
    buildInputs = oldAttrs.buildInputs ++ [pkgs.krb5];
  });
in {
  imports = [./hardware-configuration.nix ./cachix.nix];

  boot.loader = {
    systemd-boot.enable = true;
    efi.canTouchEfiVariables = true;
  };

  networking = {
    hostName = "nixos";
    #     hostId = "deadbeef";
    #     useDHCP = true;
    #     wireless = {
    #       enable = true;
    #       wifi.backend = "iwd";
    #       interfaces = [ ... ];
    #       networks = {
    #         ...
    #       };
    #     };
    networkmanager = {
      enable = true;
      #       unmanaged = [
      #         "*" "except:type:wwan" "except:type:gsm"
      #       ];
    };
  };

  i18n = {
    defaultLocale = "en_US.UTF-8";
    extraLocaleSettings = {
      LC_ADDRESS = "en_US.UTF-8";
      LC_IDENTIFICATION = "en_US.UTF-8";
      LC_MEASUREMENT = "en_US.UTF-8";
      LC_MONETARY = "en_US.UTF-8";
      LC_NAME = "en_US.UTF-8";
      LC_NUMERIC = "en_US.UTF-8";
      LC_PAPER = "en_US.UTF-8";
      LC_TELEPHONE = "en_US.UTF-8";
      LC_TIME = "en_US.UTF-8";
    };
  };

  time.timeZone = "America/Chicago";
  nix = {
    settings = {
      experimental-features = ["nix-command" "flakes" "auto-allocate-uids" "ca-derivations" "cgroups" "no-url-literals" "repl-flake"];
      trusted-users = ["user"];
      use-xdg-base-directories = true;
      builders-use-substitutes = true;
      trusted-public-keys = [
        "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
        # lol pretty sure need to delete this.
        "serokell-1:aIojg2Vxgv7MkzPJoftOO/I8HKX622sT+c0fjnZBLj0="
      ];
      substituters = ["https://cache.nixos.org"];
      trusted-substituters = [
        # lol pretty sure need to delete this.
        "s3://serokell-private-cache?endpoint=s3.eu-central-1.wasabisys.com&profile=serokell-private-cache-wasabi"
      ];
      auto-optimise-store = true;
      pure-eval = true;
      restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
      use-registries = true;
      use-cgroups = true;
    };
    package = pkgs.nixUnstable;
    optimise.automatic = true;
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 30d";
    };
  };
  nixpkgs.config.allowUnfree = true;
  sound.enable = true;
  hardware = {
    #     bluetooth.enable = true;
    pulseaudio.enable = false;
    #     nvidia = {
    #       # Enable modesetting for Wayland compositors (hyprland)
    #       modesetting.enable = true;
    #       # Use the open source version of the kernel module (for driver 515.43.04+)
    #       open = true;
    #       # Enable the Nvidia settings menu
    #       nvidiaSettings = true;
    #       # Select the appropriate driver version for your specific GPU
    #       package = config.boot.kernelPackages.nvidiaPackages.stable;
    #     };
    #     opengl = { # for nvidia
    #       enable = true;
    #       driSupport = true;
    #       driSupport32Bit = true;
    #     };
  };

  security.rtkit.enable = true;

  virtualisation = {
    libvirtd.enable = true;
    docker.enable = true;
  };

  users = {
    defaultUserShell = pkgs.nushell;
    users.user = {
      isNormalUser = true;
      description = "user";
      extraGroups = ["trusted-users" "networkmanager" "wheel" "docker" "video" "kvm"];
      packages = with pkgs; [firefox kate];
    };
  };

  fonts = {
    packages = with pkgs; [
      noto-fonts
      noto-fonts-cjk
      noto-fonts-emoji
      font-awesome
      source-han-sans
      source-han-sans-japanese
      source-han-serif-japanese
      (nerdfonts.override {fonts = ["Meslo"];})
    ];
    fontconfig = {
      enable = true;
      defaultFonts = {
        monospace = ["Meslo LG M Regular Nerd Font Complete Mono"];
        serif = ["Noto Serif" "Source Han Serif"];
        sansSerif = ["Noto Sans" "Source Han Sans"];
      };
    };
  };

  services = {
    printing.enable = true;
    pipewire = {
      enable = true;
      audio.enable = true;
      pulse.enable = true;
      wireplumber.enable = true;
      alsa = {
        enable = true;
        support32Bit = true;
      };
      jack.enable = true;
    };
    #     devmon.enable = true;
    #     udisks2.enable = true;
    #     gvfs.enable = true;
    #     blueman.enable = true;
    flatpak.enable = true;
    dbus.enable = true;
    openssh.enable = true;

    locate = {
      enable = true;
      locate = pkgs.plocate;
      interval = "hourly";
      localuser = null;
    };
    xserver = {
      enable = true;
      displayManager = {
        #autoLogin = { enable = true; user = "user"; };
        #defaultSession = "hyprland";
        sddm.enable = true;
      };
      #            libinput.enable = true;
      #       desktopManager.plasma5.enable = true;
      layout = "us";
      xkbVariant = "";
      # #       videoDrivers = [ "nvidia" ]; # If you are using a hybrid laptop add its iGPU manufacturer nvidia amd intel
    };
  };
  programs = {
    steam = {
      enable = true;
      remotePlay.openFirewall = true; # Open ports in the firewall for Steam Remote Play
      dedicatedServer.openFirewall = true; # Open ports in the firewall for Source Dedicated Server
    };
  };
  environment = {
    sessionVariables.NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
    variables.EDITOR = "nvim";
    systemPackages = with pkgs;
      [
        zigpkgs.master
        #
        git
        kitty
        xwayland
        direnv
        vscode-insiders
        openssl
        libsForQt5.yakuake
        tmux
        alacritty
        ripgrep
        # ripgrep-all # regression cannot find hello 26 times 2023-08-19
        zoxide
        vimPlugins.zoxide-vim
        vimPlugins.telescope-zoxide
        starship # replacement prompt (not shell)
        hyperfine
        # todo figure out how to use sway
        zellij # tmux
        exa # ls
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
        helix
        pstree
        mkcert
        gitAndTools.diff-so-fancy
        ripgrep
        fortune
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
        gcc
        ## block ick
        # These are so intellij file watchers has something to use
        nodejs-18_x
        #         nodePackages.prettier
        nodePackages.wrangler
        nodePackages.vercel
        vimPlugins.nvim-treesitter-parsers.typescript
        nodePackages.typescript
        nodePackages.typescript-language-server
        ## endblock ick

        # Rust CLI Tools! I love rust.
        exa
        bat
        tokei
        xsv
        fd

        # Development
        neovim
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
        neofetch

        # Jokes
        fortune
        figlet
        lolcat

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
        pyright
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

        rnix-lsp
        gcc
        ripgrep
        fd
        nodePackages.pyright
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
        binutils
        brightnessctl
        feh
        wget
        pciutils
        intel-gpu-tools
        killall
        libva-utils
        spotify
        skypeforlinux
        zoom-us
        slack
        docker-compose
        direnv
        pavucontrol
        polybar
        pulseaudio
        dunst
        cmake
        gcc
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
        waybar
        waybar-hyprland
        wayland
        wdisplays
        wl-clipboard
      ]
      ++ [
        vim
        wget
        w3m
        dmenu
        neofetch
        neovim
        autojump
        starship
        brave
        bspwm
        celluloid
        clang-tools_9
        dwm
        dunst
        elinks
        eww
        feh
        flameshot
        flatpak
        fontconfig
        freetype
        gcc
        gh
        gimp
        git
        github-desktop
        gnugrep
        gnumake
        gparted
        kitty
        libverto
        lightdm # added by ilh
        mangohud
        neovim
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
        rofi
        sxhkd
        st
        stdenv
        synergy
        swaycons
        terminus-nerdfont
        tldr
        trash-cli
        unzip
        variety
        vscode
        xclip
      ]
      ++ [
        alacritty # gpu accelerated terminal
        #     dbus-sway-environment
        #     configure-gtk
        wayland
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
        gcc
        glibc
        clang
        udev
        llvmPackages.bintools
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
        pkgconfig

        # Standard Packages
        networkmanager
        networkmanagerapplet
        git
        fzf
        vim
        tldr
        sox
        yad
        flatpak

        # GTK
        gtk2
        gtk3
        gtk4

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
        helix
        brave
        xfce.thunar
        kitty
        bat
        exa
        pavucontrol
        blueman
        #trash-cli
        ydotool
        cava
        neofetch
        cpufetch
        starship
        lolcat
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
        (pkgs.rstudioWrapper.override {
          packages = with pkgs.rPackages; [
            dplyr
            xts
            ggplot2
            reshape2
            rstudioapi
          ];
        })

        # Command Shells
        nushell

        # Display Managers
        lightdm
        sddm
        gnome.gdm

        # Hyprland Rice
        #     hyprland
        xwayland
        cliphist
        alacritty
        rofi-wayland
        swww
        swaynotificationcenter
        lxde.lxsession
        #     inputs.hyprwm-contrib.packages.${system}.grimblast
        gtklock
        eww-wayland
        xdg-desktop-portal-hyprland
        hyprland-protocols
        hyprland-share-picker
        hyprland-autoname-workspaces
        hyprland-per-window-layout
        nwg-displays
        nwg-dock-hyprland
        gtk3
        gtk4
        hyprdim
        waybar-hyprland
        wttrbar
        eww
        eww-wayland
        polybarFull
        awesome
        cowsay
        banner
        bsdgames
        endlessh
      ]
      ++ [
        alejandra # from overlay
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
        felix
        fclones
        dua
        dog
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
        hex
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
        cargo-graph
        # fenix
        # vim-racer
        # egui_graphs
        pgfplots
        # plotly
        # plotlib
        duckdb
        # trustfall
        # soup
        libsForQt5.polkit-kde-agent
        wireplumber
        dunst
        mako
        bsdgames
        haskellPackages.misfortune
        taoup
        rPackages.fortunes
        emacsPackages.fortune-cookie
        fortune
        ion
        dash
        pls
        oh-my-zsh
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

        signal-desktop
        tdesktop
        element-desktop
        tor-browser-bundle-bin
        monero-gui
        discord
        zoom-us
        # calibre
        slack
        xournal
        xdg-utils

        bat
        pv
        exa
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
        #         vimPlugins.vim-toml
        #         vimPlugins.vim-prettier
        #         vimPlugins.nvim-treesitter-parsers.toml
        #         prettierToml #overlay
        nodePackages.prettier
        nodePackages.prettier-plugin-toml
        shfmt
        rustfmt
      ];

    ######## STUPID PACKAGES BULLSHIT ABOVE THIS LINE
  };
}
