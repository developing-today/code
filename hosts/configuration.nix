{
  inputs,
  config,
  lib,
  host,
  hostName,
  pkgs,
  ...
}:
{
  imports = [ (lib.from-root "hosts/sops") ];
  # imports = [ ../../home/yazi.nix ];
  boot = {
    tmp = {
      cleanOnBoot = true;
    };
    loader = {
      systemd-boot = {
        enable = true;
        configurationLimit = 2048;
      };
      efi.canTouchEfiVariables = true;
    };
  };
  sops.secrets."wireless" = {
    sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-wi-1.yaml";
  };
  networking = {
    inherit hostName;
    # hostId = deadbeef # 8 unique hex chars
    useDHCP = true;
    networkmanager = {
      enable = false;
    };
    wireless = {
        enable = true;
        scanOnLowSignal = true;
        fallbackToWPA2 = true;
        secretsFile = config.sops.secrets.wireless.path;
        networks = import (lib.from-root "hosts/networking/wireless/us-wi-1");
    };

    firewall = {
      allowedUDPPorts = [ config.services.tailscale.port ]; # needed?
    };
  };
  # impermanence?
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
    registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs); # This will add each flake input as a registry. To make nix3 commands consistent with your flake
    nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry; # This will additionally add your inputs to the system's legacy channels. Making legacy nix commands consistent as well, awesome!
    settings = (import ./nixconfig.nix); # imports instead?
    package = pkgs.nixVersions.nix_2_23;
    optimise.automatic = true;
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 400d";
    };
  };
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
  #sound.enable = true; # not needed?
  hardware = {
    brillo.enable = false;
    pulseaudio.enable = false;
  };

  security.rtkit.enable = true;

  virtualisation = {
    libvirtd.enable = true;
    docker.enable = true;
  };

  users = {
    # remove from here?
    defaultUserShell = pkgs.oils-for-unix; # pkgs.nushell; # oils-for-unix; #nushell; # per user?
    # mutableUsers = false;
    users = {
      # todo modules
      user = import (lib.from-root "hosts/users/user") { inherit pkgs config; }; # imports
      backup = import (lib.from-root "hosts/users/backup") { inherit pkgs config; }; # imports
    };
  };
  sops.secrets."users/backup/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/backup/password_backup.yaml";
  };
  sops.secrets."users/user/passwordHash" = {
    neededForUsers = true;
    sopsFile = lib.from-root "secrets/sops/users/user/password_user.yaml";
  };
  fonts = {
    packages = with pkgs; [
      # only desktops not servers?
      noto-fonts
      noto-fonts-cjk
      noto-fonts-emoji
      font-awesome
      source-han-sans
      source-han-sans-japanese
      source-han-serif-japanese
      (nerdfonts.override { fonts = [ "Meslo" ]; })
    ]; # missing other fonts
    fontconfig = {
      # ligatures just give me ligatures what is this
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
    # desktop only
    tailscale.enable = true; # needed?
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
    flatpak.enable = true;
    dbus.enable = true;
    openssh = {
      enable = true;
      hostKeys = [
        {
          path = "/etc/ssh/ssh_host_ed25519_key";
          type = "ed25519";
        }
      ] ++ lib.optionals host.bootstrap [
        {
          path = "/bootstrap/ssh_host_ed25519_key";
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
      #autoLogin = { enable = true; user = "user"; }; # security risk?
      defaultSession = "hyprland"; # for better or worse
      sddm.enable = true;
      #gdm.enable = true; # two at once bad
    };
    xserver = {
      enable = true;
      # libinput.enable = true;
      desktopManager = {
        # backup in case hyprland gets broken again
        #plasma6.enable = true; # bloat but kinda pretty
        #plasma5.enable = true; # bloat but kinda pretty
        gnome.enable = true;
      };
      xkb = {
        layout = "us";
        variant = "";
      };
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
    # things should end up in systempackages if
    # they are required for boot or login or
    # have namespace conflicts i don't want to deal with in home manager
    # or just because
    # etc.
    systemPackages = with pkgs; [
      # TODO: cleanup systemPackages
      # build
      # charm stuff?
      # dwm
      # fortune
      # gtk
      # inputs.omnix.packages.${pkgs.system}.default
      # omnix
      # overlays # todo- move into user
      # clang-tools_9
      # fontmatrix
      # grep
      # nix-software-center
      # zed-editor
      # zigpkgs.master
      alacritty-theme
      alejandra # unused now?
      asciinema
      awesome
      banner
      bc
      binutils
      brillo
      bsdgames
      cabal-install
      cabal2nix
      choose
      cinnamon-desktop
      clang
      cowsay
      deadnix
      e2fsprogs
      emacsPackages.fortune-cookie
      expect # unbuffer
      figlet
      fira-code
      fira-code-symbols
      font-awesome
      font-awesome_5
      font-manager
      fontforge
      fontpreview
      fortune
      gawk
      gcc
      gdm
      ghc
      gnomeExtensions.toggle-alacritty
      grimblast
      gtk2
      gtk3
      gtk4
      hackgen-nf-font
      haskellPackages.misfortune
      hasklig
      hledger
      hledger-iadd
      hledger-interest
      hledger-ui
      hledger-utils
      hledger-web
      hyprcursor
      hyprdim
      hyprkeys
      hyprland-monitor-attached
      hyprland-protocols
      hyprlandPlugins.hypr-dynamic-cursors
      hyprlock
      hyprpicker
      hyprshade
      hyprshot
      kanata
      kitti3
      kitty
      kitty-img
      kitty-themes
      kittysay
      lf
      libsixel
      libusb
      libusb-compat-0_1
      pkg-config
      libusb1
      hidapi
      lightdm
      llvmPackages.bintools
      lolcat
      lsix
      maple-mono-NF
      maple-mono-SC-NF
      maple-mono-autohint
      maple-mono-otf
      maple-mono-woff2
      minicom
      monoid
      ncdu
      ncurses
      neovim
      nerd-font-patcher
      nerdfix
      nerdfix
      nerdfonts
      nh
      niv
      nix-du
      nix-melt
      nix-output-monitor
      nix-query-tree-viewer
      nix-tree
      nix-visualize
      nixfmt-rfc-style
      nushell
      nvd
      oils-for-unix # todo: osh default shell?
      opentofu
      pixcat
      playerctl
      python312Packages.pycritty
      rPackages.fortunes
      ranger
      rictydiminished-with-firacode
      screen
      sddm
      statix
      tailscale
      taoup
      terminus-nerdfont
      # termpdfpy # 2024-09-17 ⚠ python3.12-pymupdf-1.24.8 failed with exit code 1 after ⏱ 1m55s in pythonImportsCheckPhase
      terranix
      udev-gothic-nf
      vimPlugins.vim-kitty-navigator
      waybar
      wayland
      xdg-desktop-portal-hyprland
      xorg.xcursorthemes
      xwayland
      yazi
      yq
      zathura
      zathura
      zed-editor
      magic-wormhole-rs
      wormhole-william
      magic-wormhole
      webwormhole
      portal
      cdrkit
      cdrtools
      age
      libisoburn # xorriso
      ];
    ######## STUPID PACKAGES BULLSHIT ABOVE THIS LINE
  };
}
