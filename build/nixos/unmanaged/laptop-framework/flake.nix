{
  inputs = {
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    home = {
      url = "path:./flakes/home";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.beautysh.follows = "beautysh";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.neovim-flake.follows = "neovim-flake";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
      inputs.hercules-ci-agent.follows = "hercules-ci-agent";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.home-manager.follows = "home-manager";
    };
    home-manager = {
      url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
    };
    #  hardware.url = "github:nixos/nixos-hardware"; # todo figure out how to use this
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; #*/ # inputs.systems
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    neovim-flake = {
      url = "github:neovim/neovim?dir=contrib";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
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
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
      inputs.neovim-flake.follows = "neovim-flake";
    }; # need to get this into home/vim flake
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
      url = "https://flakehub.com/f/kamadorueda/alejandra/*.tar.gz"; #*/ # url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flakeCompat.follows = "flake-compat";
    };
    nix-software-center = {
      url = "github:vlinkz/nix-software-center";
      #url = "https://flakehub.com/f/vlinkz/nix-software-center/*.tar.gz"; #*/ # https://github.com/vlinkz/nix-software-center/pull/50
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.utils.follows = "flake-utils";
    };
    nixvim = {
      # url = "github:nix-community/nixvim";
      url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.beautysh.follows = "beautysh";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };

    beautysh = {
      url = "github:lovesegfault/beautysh";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
    };

    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      # todo: drag out these follows
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
      home.vim-overlay
    ];
    systemNixosModules = [
      {
        nixpkgs = {
          overlays = overlays; # are overlays needed in home manager? document which/why?
          config.allowUnfree = true;
        };
        system.stateVersion = stateVersion;
      }
      ./modules/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
    ];
    # overlayNixosModules = ?
    hyprlandNixosModules = [
      (import ./modules/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
      #       (import ./modules/nm-applet.nix)
    ];
    homeManagerNixosModules = home.homeManagerNixosModules stateVersion;
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      system = system;
      overlays = overlays;
    };
  in {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      inherit system;
      modules =
        systemNixosModules
        ++ hyprlandNixosModules
        ++ homeManagerNixosModules;
    };
  };
  # hydra
  # content addressible
}
