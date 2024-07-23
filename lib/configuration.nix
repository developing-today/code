{
  #outputs,
  inputs,
  config,
  lib,
  pkgs,
  ...
}:
{
  imports = [ ./sops.nix ];
  boot = {
    tmp = {
      cleanOnBoot = true;
    };
    loader = {
      systemd-boot = {
        enable = true;
        configurationLimit = 64;
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

  /*
    nixpkgs.overlays = [
      outputs.overlays.default
    ];
    nixpkgs.config.allowUnfree = true;
  */
  nix = {
    # This will add each flake input as a registry
    # To make nix3 commands consistent with your flake
    registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs);

    # This will additionally add your inputs to the system's legacy channels
    # Making legacy nix commands consistent as well, awesome!
    nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry;
    settings = {
      experimental-features = [
        "auto-allocate-uids"
        "ca-derivations"
        "cgroups"
        "dynamic-derivations"
        "fetch-closure"
        "flakes"
        "git-hashing"
        # "local-overlay-store" # look into this
        # "mounted-ssh-store" # look into this
        "nix-command"
        # "no-url-literals" # <- removed no-url-literals for flakehub testing
        "parse-toml-timestamps"
        "read-only-local-store"
        "recursive-nix"
        "verified-fetches"
      ];
      #       trusted-users = [ "user" ];
      use-xdg-base-directories = true;
      builders-use-substitutes = true;
      #       substituters = [ "https://cache.nixos.org" ];
      #       trusted-public-keys = [ "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=" ];

      trusted-users = [ "root" ];
      #     trusted-user = "root";
      substituters = [
        "https://cache.nixos.org"
        #"https://hydra.nixos.org"
        "https://nix-community.cachix.org"
        "https://numtide.cachix.org"
        "https://colmena.cachix.org"
        "https://nix-gaming.cachix.org"
        "https://nrdxp.cachix.org"
        "https://cache.m7.rs"
        "https://sylvorg.cachix.org"
      ];
      extra-substituters = [
        #       "https://cache.nixos.org"
        #       #"https://hydra.nixos.org"
        #       "https://nix-community.cachix.org"
        #       "https://numtide.cachix.org"
        #       "https://colmena.cachix.org"
        #       "https://nix-gaming.cachix.org"
        #       "https://nrdxp.cachix.org"
        #       "https://cache.m7.rs"
        #       "https://sylvorg.cachix.org"
      ];
      trusted-substituters = [
        "https://cache.nixos.org"
        #"https://hydra.nixos.org"
        "https://nix-community.cachix.org"
        "https://numtide.cachix.org"
        "https://colmena.cachix.org"
        "https://nix-gaming.cachix.org"
        "https://nrdxp.cachix.org"
        "https://cache.m7.rs"
        "https://sylvorg.cachix.org"
      ];
      extra-trusted-substituters = [
        #       "https://cache.nixos.org"
        #       #"https://hydra.nixos.org"
        #       "https://nix-community.cachix.org"
        #       "https://numtide.cachix.org"
        #       "https://colmena.cachix.org"
        #       "https://nix-gaming.cachix.org"
        #       "https://nrdxp.cachix.org"
        #       "https://cache.m7.rs"
        #       "https://sylvorg.cachix.org"
      ];
      trusted-public-keys = [
        "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
        "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
        "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
        "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
        "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
        "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
      ];
      extra-trusted-public-keys = [
        #       "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
        #       "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
        #       "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        #       "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
        #       "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        #       "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
        #       "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
        #       "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
      ];
      http-connections = 128;
      max-substitution-jobs = 128;
      keep-outputs = true; # Nice for developers
      keep-derivations = true; # Idem
      accept-flake-config = true;
      #     allow-dirty = false;
      #     builders-use-substitutes = true;
      fallback = true;
      log-lines = 128;
      #     pure-eval = true;
      # run-diff-hook = true;
      # secret-key-files
      show-trace = true;
      # tarball-ttl = 0;
      # trace-function-calls = true;
      trace-verbose = true;
      # use-xdg-base-directories = true;
      allow-dirty = true;

      /*
        buildMachines = [ ];
        distributedBuilds = true;
        # optional, useful when the builder has a faster internet connection than yours
        extraOptions = ''
          builders-use-substitutes = true
        '';
      */
      auto-optimise-store = true;
      #pure-eval = true;
      pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
      restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
      use-registries = true;
      use-cgroups = true;
    };
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
      "electron" # le sigh
    ];
  };
  #sound.enable = true;
  hardware = {
    #     bluetooth.enable = true;
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
    defaultUserShell = pkgs.nushell; # oils-for-unix; #nushell;
    users.user = {
      isNormalUser = true;
      description = "user";
      extraGroups = [
        "trusted-users"
        "networkmanager"
        "wheel"
        "docker"
        "video"
        "kvm"
        "beep"
      ];
      packages = with pkgs; [
        firefox
        kate
      ];
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
    #fwupd.enable = true; # laptop-framework # don't follow this guide you have a framework 12 intel # https://github.com/NixOS/nixos-hardware/tree/master/framework/13-inch/13th-gen-intel#getting-the-fingerprint-sensor-to-work
    # https://knowledgebase.frame.work/ubuntu-fingerprint-troubleshooting-r1_DA0TMn
    # TODO: pull the hardware flake for 12th gen intel
    # nixos-hardware.nixosModules.framework-12th-gen-intel
    #     devmon.enable = true;
    #     udisks2.enable = true;
    #     gvfs.enable = true;
    #     blueman.enable = true;
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
      sddm.enable = true;
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
    systemPackages =
      with pkgs;
      [
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
        gnome.gdm
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
