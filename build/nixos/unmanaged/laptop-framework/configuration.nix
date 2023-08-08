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
    git
    direnv
    vscode-insiders
    openssl
    zigpkgs.default
  ];

  programs.neovim = {
    enable = true;
    defaultEditor = true;
  };

  system.stateVersion = "23.05";

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  virtualisation.libvirtd.enable = true;
  virtualisation.docker.enable = true;
}
