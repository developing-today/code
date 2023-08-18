{ config, pkgs, ... }:
let
  vscode-insiders = (pkgs.vscode.override { isInsiders = true; }).overrideAttrs (oldAttrs: rec {
    src = (builtins.fetchTarball {
      url = "https://code.visualstudio.com/sha/download?build=insider&os=linux-x64";
      sha256 = "16fzxqs6ql4p2apq9aw7l10h4ag1r7jwlfvknk5rd2zmkscwhn6z";
    });
    version = "latest";
    buildInputs = oldAttrs.buildInputs ++ [ pkgs.krb5 ];
  });
in
{
  imports = [ ./hardware-configuration.nix ./cachix.nix ];

  boot.loader = {
    systemd-boot.enable = true;
    efi.canTouchEfiVariables = true;
  };

  networking = {
    hostName = "nixos";
    networkmanager.enable = true;
  };

  i18n = {
    defaultLocale = "en_US.UTF-8";
    extraLocaleSettings = {
      LC_ADDRESS        = "en_US.UTF-8";
      LC_IDENTIFICATION = "en_US.UTF-8";
      LC_MEASUREMENT    = "en_US.UTF-8";
      LC_MONETARY       = "en_US.UTF-8";
      LC_NAME           = "en_US.UTF-8";
      LC_NUMERIC        = "en_US.UTF-8";
      LC_PAPER          = "en_US.UTF-8";
      LC_TELEPHONE      = "en_US.UTF-8";
      LC_TIME           = "en_US.UTF-8";
    };
  };

  time.timeZone = "America/Chicago";
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nixpkgs.config.allowUnfree = true;
  sound.enable = true;
  hardware = {
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

  users.users.user = {
    isNormalUser = true;
    description = "user";
    extraGroups = [ "networkmanager" "wheel" "docker" ];
    packages = with pkgs; [ firefox kate ];
  };

  fonts = {
    packages = with pkgs; [
      noto-fonts noto-fonts-cjk noto-fonts-emoji font-awesome
      source-han-sans source-han-sans-japanese source-han-serif-japanese
      (nerdfonts.override { fonts = [ "Meslo" ]; })
    ];
    fontconfig = {
      enable = true;
      defaultFonts = {
        monospace  = [ "Meslo LG M Regular Nerd Font Complete Mono" ];
        serif      = [ "Noto Serif" "Source Han Serif" ];
        sansSerif  = [ "Noto Sans" "Source Han Sans" ];
      };
    };
  };

  services = {
    printing.enable = true;
    pipewire = {
      enable = true;
      audio.enable = true;
      pulse.enable = true;
      alsa = { enable = true; support32Bit = true; };
      jack.enable = true;
    };
    locate = {
      enable = true;
      locate = pkgs.plocate;
      interval = "hourly";
      localuser = null;
    };
    xserver = {
      enable = true;
      displayManager = {
        autoLogin = { enable = true; user = "user"; };
#         defaultSession = "plasmawayland";
        sddm.enable = true;
      };
      desktopManager.plasma5.enable = true;
      layout = "us";
      xkbVariant = "";
#       videoDrivers = [ "nvidia" ]; # If you are using a hybrid laptop add its iGPU manufacturer nvidia amd intel
    };
  };

  programs = {
    hyprland = {
      enable = true;
      xwayland.enable = true;
#       nvidiaPatches = true; # ONLY use this line if you have an nvidia card
    };
    neovim = {
      enable = true;
      defaultEditor = true;
      viAlias = true;
      vimAlias = true;
    };
  };

  environment = {
    sessionVariables.NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
    variables.EDITOR = "nvim";
    systemPackages = with pkgs; [
    #
    git
    kitty
    xwayland
    direnv
    vscode-insiders
    openssl
    zigpkgs.default
    libsForQt5.yakuake
    tmux
    alacritty
    ripgrep
    ripgrep-all
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
    #rmesg # unknown
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
  nodePackages.prettier
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

] ++
    [
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
    ] ++ [
      glib
      grim
      slurp
#       sway
#       swayidle
#       swaylock
      waybar
      wayland
      wdisplays
      wl-clipboard
    ] ++ [
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
# lightdm # added by ilh
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
# swaycons
terminus-nerdfont
tldr
trash-cli
unzip
variety
vscode
xclip
    ];
};
}
