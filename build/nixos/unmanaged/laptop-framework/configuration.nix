{ config, pkgs, ... }:

let
  vscode-insiders = pkgs.stdenv.mkDerivation rec {
    pname = "vscode-insiders";
    version = "latest";

    src = pkgs.fetchurl {
      url = "https://update.code.visualstudio.com/latest/linux-x64/insider";
      sha256 = "9dd143e87499eac31382cdd5feeecde1d06debfe791a9b070e8a357ced0a81f5";
    };

    nativeBuildInputs = [ pkgs.autoPatchelfHook ];
    buildInputs = with pkgs; [
      stdenv.cc.cc.lib
      makeWrapper
      glib
      krb5
      at-spi2-atk
      xorg.libX11
      xorg.libxkbfile
      libsecret
      xorg.libXfixes
      xorg.libXrandr
      mesa
      libxkbcommon
      alsaLib
    ];

    unpackPhase = ''
      tar -xzf $src -C .
    '';

    installPhase = ''
      mkdir -p $out/bin
      cp -r ./* $out/
      ln -s $out/VSCode-linux-x64/code-insiders $out/bin/code-insiders
      wrapProgram $out/bin/code-insiders \
        --prefix LD_LIBRARY_PATH : "${pkgs.stdenv.cc.cc.lib}/lib:${pkgs.stdenv.cc.cc.lib}/lib64:${pkgs.glib}/lib:${pkgs.krb5}/lib:${pkgs.at-spi2-atk}/lib:${pkgs.xorg.libX11}/lib:${pkgs.xorg.libxkbfile}/lib:${pkgs.libsecret}/lib:${pkgs.xorg.libXfixes}/lib:${pkgs.xorg.libXrandr}/lib:${pkgs.mesa}/lib:${pkgs.libxkbcommon}/lib:${pkgs.alsaLib}/lib"
    '';

    meta = with pkgs.lib; {
      description = "Visual Studio Code - Insiders";
      homepage = "https://code.visualstudio.com/insiders/";
      license = licenses.mit;
      platforms = platforms.linux;
    };
  };
in
{

  imports =
    [ # Include the results of the hardware scan.
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
    extraGroups = [ "networkmanager" "wheel" ];
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
    vscode
    vscode-insiders
  ];

  programs.neovim = {
    enable = true;
    defaultEditor = true;
  };

  system.stateVersion = "23.05";

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  virtualisation.libvirtd.enable = true;
}
