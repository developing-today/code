{
  inputs = {
    # MANUALLY KEEP NIXPKGS AND NIXPKGS-LIB 1:1.
    # CHANGE ONE CHANGE THE OTHER.
    # master then if it breaks unstable then if it breaks 23.11 or something.
    # "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"
    nixpkgs.url = "github:NixOS/nixpkgs"; # /nixos-unstable"; # /nixos-23.11";
    nixpkgs-lib = {
      url = "github:NixOS/nixpkgs?dir=lib"; # /nixos-unstable?dir=lib"; # /nixos-23.11?dir=lib";
    };
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
    hercules-ci-agent = {
      url = "github:hercules-ci/hercules-ci-agent";
      inputs.flake-parts.follows = "flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-agent.follows = "hercules-ci-agent";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    zig-overlay = {
      url = "github:mitchellh/zig-overlay"; # url = "github:developing-today-forks/zig-overlay/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
    };
    neovim-flake = {
      url = "github:neovim/neovim?dir=contrib";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
      inputs.neovim-flake.follows = "neovim-flake";
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs-lib";
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
    neovim-nightly-overlay,
    alejandra,
    nix-software-center,
    ...
  }: let
    stateVersion = "23.11";
    overlays = [
      zig-overlay.overlays.default
      neovim-nightly-overlay.overlay
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
