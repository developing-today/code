{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  pkgs,
  ...
}:
{
  environment = {
    sessionVariables.NIXOS_OZONE_WL = "1"; # This variable fixes electron apps in wayland
    variables = {
      EDITOR = "nvim";
      NIX_REMOTE = "daemon";
    };
    # things should end up in systempackages if
    # they are required for boot or login or
    # have namespace conflicts i don't want to deal with in home manager
    # or just because
    # etc.
    systemPackages =
      (with inputs; [
        ssh-to-age.packages.${system}.default
      ])
      ++
      (with inputs.nixpkgs-stable.legacyPackages.${system}; [
        activitywatch
      ])
      ++
      (with pkgs; [
      # TODO: cleanup systemPackages
      # build
      # charm stuff?
      # dwm
      # fortune
      # gtk
      # inputs.omnix.packages.${system}.default
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
      usbutils
      usbtop
      usbrip
      usbview
      usbimager
      ns-usbloader
      woeusb
      gparted
      woeusb-ng

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
      rescuetime
      ddrescue
      magicrescue
      ddrutility
      myrescue
      ddrescueview
      unetbootin # can't launch right now? qt platform platform plugin not found
      dd_rescue
      ventoy-full # https://www.ventoy.net/en/doc_search_path.html
      # ventoy
      screen
      sddm
      netboot
      ipxe
      # waitron
      # https://theartofmachinery.com/2016/04/21/partitioned_live_usb.html
      # https://www.system-rescue.org/
      # https://discourse.nixos.org/t/how-to-add-a-rescue-option-to-bootloader/19137
      # specialisation rescue disk
      # specialisation live disk
      # specialisation usb live disk
      # https://nixos.wiki/wiki/Change_root
      # https://nixos.wiki/wiki/Bootloader#From_an_installation_media
      # https://wiki.gentoo.org/wiki/LiveUSB#Linux
      pixiecore
      # yumi # no package yet :(
      # netbootxyz-efi # WARNING: caused failed rebuild
      # netbootxyz
      # tinkerbell
      # matchbox-server
      # terraform-providers.<provider>
      # https://github.com/DeterminateSystems/nix-netboot-serve
      ubootTools
      # uboot<raspberryModel>
      statix
      syslinux
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
      wpa_supplicant_gui
      # wpa_cute # TODO: try this?
      element-web
      element-call
      element-desktop
    ]);
    ######## STUPID PACKAGES BULLSHIT ABOVE THIS LINE
  };
}
