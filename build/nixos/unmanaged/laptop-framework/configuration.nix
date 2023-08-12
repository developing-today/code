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
  imports =
    [
      ./hardware-configuration.nix
      ./cachix.nix
    ];

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  networking.hostName = "nixos";
  networking.networkmanager.enable = true;

  time.timeZone = "America/Chicago";

  i18n.defaultLocale = "en_US.UTF-8";
  i18n.extraLocaleSettings = {
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

  services.xserver.enable = true;
  services.xserver.displayManager.sddm.enable = true;
  services.xserver.desktopManager.plasma5.enable = true;
  services.xserver = {
    layout = "us";
    xkbVariant = "";
  };

  services.printing.enable = true;
  sound.enable = true;
  hardware.pulseaudio.enable = false;
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  users.users.user = {
    isNormalUser = true;
    description = "user";
    extraGroups = [ "networkmanager" "wheel" "docker" ];
    packages = with pkgs; [
      firefox
      kate
    ];
  };

  services.xserver.displayManager.autoLogin.enable = true;
  services.xserver.displayManager.autoLogin.user = "user";

  nixpkgs.config.allowUnfree = true;

  environment.systemPackages = with pkgs; [
    #
    git
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
  ];

  programs.neovim = {
    enable = true;
    defaultEditor = true;
    viAlias = true;
    vimAlias = true;
  };

  environment.variables.EDITOR = "nvim";
  #system.stateVersion = "23.05";

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  virtualisation.libvirtd.enable = true;
  virtualisation.docker.enable = true;

  services.locate = {
        enable = true;
        locate = pkgs.plocate;
        interval = "hourly";
        localuser = null;
  };
}

