{
  config,
  pkgs,
  ...
}: {
  imports = [./hardware-configuration/laptop-framework.nix ./cachix.nix];

  boot.loader = {
    systemd-boot = {
      enable = true;
      configurationLimit = 64;
    };
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
      experimental-features = ["nix-command" "flakes" "auto-allocate-uids" "ca-derivations" "cgroups" "repl-flake"]; # "no-url-literals" # <- removed no-url-literals for flakehub testing
      trusted-users = ["user"];
      use-xdg-base-directories = true;
      builders-use-substitutes = true;
      trusted-public-keys = [
        "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      ];
      substituters = ["https://cache.nixos.org"];
      auto-optimise-store = true;
      #pure-eval = true;
      pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
      restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
      use-registries = true;
      use-cgroups = true;
    };
    package = pkgs.nixVersions.nix_2_18;
    optimise.automatic = true;
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 60d";
    };
  };
  nixpkgs.config.allowUnfree = true;
  sound.enable = true;
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
    defaultUserShell = pkgs.nushell;
    users.user = {
      isNormalUser = true;
      description = "user";
      extraGroups = ["trusted-users" "networkmanager" "wheel" "docker" "video" "kvm" "beep"];
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
    systemPackages = with pkgs;
      [
        # overlays # todo- move into user
        zigpkgs.master
        #nix-software-center
        alejandra
        neovim
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
