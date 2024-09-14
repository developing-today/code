{
  inputs,
  config,
  lib,
  pkgs,
  ...
}:
{
  imports = [ ../../../hosts/common/modules/sops.nix ];
  boot = {
    tmp = {
      cleanOnBoot = true;
    };
    loader = {
      systemd-boot = {
        enable = true;
        configurationLimit = 512;
      };
      efi.canTouchEfiVariables = true;
    };
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
    firewall = {
      allowedUDPPorts = [ config.services.tailscale.port ];
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
    # This will add each flake input as a registry
    # To make nix3 commands consistent with your flake
    registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs);
    # This will additionally add your inputs to the system's legacy channels
    # Making legacy nix commands consistent as well, awesome!
    nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry;
    settings = (import ./nixconfig.nix);
    package = pkgs.nixVersions.nix_2_23;
    optimise.automatic = true;
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 180d";
    };
  };
  nixpkgs.config = {
    allowUnfree = true;
    permittedInsecurePackages = [
    "olm-3.2.16"
      "electron" # le sigh
      "qtwebkit-5.212.0-alpha4" # ???
    ];
  };
  #sound.enable = true;
  hardware = {
    brillo.enable = false;
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
    defaultUserShell = pkgs.oils-for-unix; # pkgs.nushell; # oils-for-unix; #nushell;
    users = {
      # TODO: maybe don't use a partial here
      # TODO: instead pass a full path as an import
      user = import ../../../hosts/common/users/user { inherit pkgs; };
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
      (nerdfonts.override { fonts = [ "Meslo" ]; })
    ];
    fontconfig = {
      enable = true;
      defaultFonts = {
        monospace = [ "Meslo LG M Regular Nerd Font Complete Mono" ];
        serif = [
          "Noto Serif"
          "Source Han Serif"
        ];
        sansSerif = [
          "Noto Sans"
          "Source Han Sans"
        ];
      };
    };
  };

  services = {
    tailscale.enable = true;
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
    kanata.enable = true;
    #fwupd.enable = true; # laptop-framework # don't follow this guide you have a framework 12 intel # https://github.com/NixOS/nixos-hardware/tree/master/framework/13-inch/13th-gen-intel#getting-the-fingerprint-sensor-to-work
    # https://knowledgebase.frame.work/ubuntu-fingerprint-troubleshooting-r1_DA0TMn
    # TODO: pull the hardware flake for 12th gen intel
    # nixos-hardware.nixosModules.framework-12th-gen-intel
    #     devmon.enable = true;
    #     udisks2.enable = true;
    #     gvfs.enable = true;
    flatpak.enable = true;
    dbus.enable = true;
    openssh = {
      enable = true;
      hostKeys = [
        {
          path = "/etc/ssh/ssh_host_ed25519_key";
          type = "ed25519";
        }
      ];
    };

    locate = {
      enable = true;
      package = pkgs.plocate;
      interval = "hourly";
      localuser = null;
    };
    displayManager = {
      #autoLogin = { enable = true; user = "user"; };
      defaultSession = "hyprland";
      sddm.enable = true; # /bin/osh
      #gdm.enable = true;
    };
    xserver = {
      enable = true;
      #            libinput.enable = true;
      desktopManager = {
        #plasma6.enable = true;
        #plasma5.enable = true;
        gnome.enable = true;
      };
      xkb = {
        layout = "us";
        variant = "";
      };
      # #       videoDrivers = [ "nvidia" ]; # If you are using a hybrid laptop add its iGPU manufacturer nvidia amd intel
    };
  };
  programs = {
    partition-manager.enable = true;
    steam = {
      enable = true;
      remotePlay.openFirewall = true; # Open ports in the firewall for Steam Remote Play
      dedicatedServer.openFirewall = true; # Open ports in the firewall for Source Dedicated Server
    };
  };
  environment = {
    sessionVariables.NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
    variables.EDITOR = "nvim";
    # things end up in systempackages if
    # they are required for boot or login
    # have namespace conflicts i don't want to deal with in home manager
    # etc.
    # TODO: cleanup systemPackages
    systemPackages =
      with pkgs;
      [
        hyprlandPlugins.hypr-dynamic-cursors
        xorg.xcursorthemes
        xdg-desktop-portal-hyprland
        hyprland-protocols
        # inputs.omnix.packages.${pkgs.system}.default
        # omnix
        cabal-install
        cabal2nix
        ghc
        hledger
        hledger-ui
        hledger-web
        hledger-iadd
        hledger-utils
        hledger-interest
        zed-editor
        opentofu
        terranix
        playerctl
        brillo
        font-manager
        font-awesome
        fontpreview
        font-awesome_5
        #fontmatrix
        fontforge
        nerdfix
        nerdfonts
        nerdfix
        nerd-font-patcher
        terminus-nerdfont
        hackgen-nf-font
        maple-mono-NF
        udev-gothic-nf
        maple-mono-SC-NF
        fira-code
        hasklig
        maple-mono-woff2
        rictydiminished-with-firacode
        maple-mono-otf
        maple-mono-autohint
        monoid
        fira-code-symbols
        grimblast
        hyprland-monitor-attached
        hyprcursor
        hyprpicker
        hyprshade
        hyprkeys
        hyprlock
        hyprshot
        hyprdim
        lf
        ranger
        zathura
        libsixel
        lsix
        ncdu
        yq
        yazi
        kitty
        kanata
        kitty-img
        kitty-themes
        kitti3
        kittysay
        pixcat
        termpdfpy
        vimPlugins.vim-kitty-navigator
        alacritty-theme
        cinnamon-desktop
        gnomeExtensions.toggle-alacritty
        python312Packages.pycritty
        zathura
        #zed-editor
        nix-output-monitor
        nix-tree
        nix-du
        nix-melt
        nix-query-tree-viewer
        nix-visualize
        niv
        nh
        nvd
        expect # unbuffer
        nushell
        ncurses
        bc
        #grep
        gawk
        choose
        e2fsprogs
        asciinema
        # charm stuff?
        statix
        deadnix

        oils-for-unix # todo: osh default shell?
        # overlays # todo- move into user
        #zigpkgs.master
        #nix-software-center
        alejandra # unused now?
        neovim
        tailscale
        nixfmt-rfc-style
      ]
      ++ [
        # dwm
        xwayland
        waybar
        wayland
        sddm
        lightdm
        gdm
        awesome
      ]
      ++ [
        # build
        gcc
        binutils
        clang
        #clang-tools_9
        llvmPackages.bintools
      ]
      ++ [
        # gtk
        gtk2
        gtk3
        gtk4
      ]
      ++ [
        # fortune
        bsdgames
        haskellPackages.misfortune
        taoup
        rPackages.fortunes
        emacsPackages.fortune-cookie
        fortune
        lolcat
        figlet
        cowsay
        banner
      ];

    ######## STUPID PACKAGES BULLSHIT ABOVE THIS LINE
  };
}
