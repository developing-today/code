{
  inputs = {
    # MANUALLY KEEP NIXPKGS AND NIXPKGS-LIB 1:1.
    # CHANGE ONE CHANGE THE OTHER.
    # master then if it breaks unstable then if it breaks 23.11 or something.
    # "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    home = {
      url = "path:./flakes/home";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    #  hardware.url = "github:nixos/nixos-hardware"; # todo figure out how to use this
    flake-utils.url = "github:numtide/flake-utils"; # inputs.systems
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    # must type follows all out every time
    # because flake inputs are basically static
    # can't make a let var function closure thing around it or whatever
    zig-overlay = {
      url = "github:mitchellh/zig-overlay"; # url = "github:developing-today-forks/zig-overlay/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
    };
    alejandra = {
      url = "github:kamadorueda/alejandra"; # url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flakeCompat.follows = "flake-compat";
    };
    nix-software-center = {
      #       url = "github:vlinkz/nix-software-center";
      url = "github:developing-today-forks/nix-software-center/overlay"; # https://github.com/vlinkz/nix-software-center/pull/50
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.utils.follows = "flake-utils";
    };
    # nix-rice = https://github.com/bertof/nix-rice # todo fork and rename this garbage
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    home,
    zig-overlay,
    alejandra,
    nix-software-center,
    ...
  }: let
    stateVersion = "23.11";
    overlays = [
      zig-overlay.overlays.default
      alejandra.overlay
      nix-software-center.overlay
    ];
    systemNixOsModules = [
      {
        nixpkgs = {
          overlays = overlays; # are overlays needed in home manager? document which/why?
          config.allowUnfree = true;
        };
        system.stateVersion = stateVersion;
      }
      ./modules/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
    ];
    # overlayNixOsModules = ?
    hyprlandNixOsModules = [
      (import ./modules/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      #       (import ./modules/nm-applet.nix)
    ];
    homeManagerNixOsModules = home.homeManagerNixOsModules stateVersion;
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      system = system;
      overlays = overlays;
    };
  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules =
        systemNixOsModules
        ++ hyprlandNixOsModules
        ++ homeManagerNixOsModules;
    };
  };
}
