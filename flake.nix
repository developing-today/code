{
  inputs = {
    #nixpkgs.url = "github:NixOS/nixpkgs";
    nixpkgs.url = "github:dezren39/nixpkgs/master";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2305.491756.tar.gz"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "github:DeterminateSystems/nixpkgs/nix_2_18_1";
    sops-nix = {
      url = "github:mic92/sops-nix";
      #inputs.nixpkgs-stable.follows ="nixpkgs";
      #inputs.nixpkgs.follows ="nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager"; # */
      #url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
    };
    #  hardware.url = "github:nixos/nixos-hardware"; # todo figure out how to use this
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
    flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects";
      inputs.flake-parts.follows = "flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
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
      url = "https://flakehub.com/f/kamadorueda/alejandra/*.tar.gz"; # */ # url = "github:developing-today-forks/alejandra/quote-urls";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flakeCompat.follows = "flake-compat";
    };
    #nix-software-center = {
    #  url = "github:snowfallorg/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "github:vlinkz/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "github:vlinkz/nix-software-center"; #https://flakehub.com/f/vlinkz/nix-software-center/0.1.2.tar.gz";
    #  #url = "https://flakehub.com/f/vlinkz/nix-software-center/*.tar.gz"; #*/ # https://github.com/vlinkz/nix-software-center/pull/50
    #  inputs.nixpkgs.follows = "nixpkgs";
    #  inputs.utils.follows = "flake-utils";
    #};
    nixvim = {
      url = "github:nix-community/nixvim";
      #url = "github:developing-today-forks/nix-community_nixvim";
      #url = "github:developing-today-forks/nixvim-flake";
      #inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./src/vim";
      #url = "github:developing-today/code?dir=src/vim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixvim.follows = "nixvim";
      inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
    };
    # nix-rice = https://github.com/bertof/nix-rice # todo fork and rename this garbage
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      zig-overlay,
      alejandra,
      sops-nix,
      vim,
      home-manager,
      #nix-software-center,
      ...
    }@inputs:
    let
      inherit (self) outputs;
      yes = "yes";
      stateVersion = "23.11";
      #stateVersion = "23.05";
      overlays = [
        zig-overlay.overlays.default
        alejandra.overlay
        #nix-software-center.overlay
        vim.overlay.${system}
      ];
      systemNixosModules = [
        {
          nixpkgs = {
            overlays = overlays; # are overlays needed in home manager? document which/why?
            config = {
              allowUnfree = true;
              permittedInsecurePackages = [
                "electron" # le sigh
              ];
            };
          };
          system.stateVersion = stateVersion;
        }
        ./modules/configuration.nix # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
        #sops-nix.nixosModules.sops
        #./modules/sops.nix
      ];
      # overlayNixosModules = ?
      hyprlandNixosModules = [
        (import ./modules/hyprland.nix) # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
        #       (import ./modules/nm-applet.nix)
      ];
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        system = system;
        overlays = overlays;
      };
      lib = nixpkgs.lib;
      zed-editor = pkgs.callPackage "${nixpkgs}/pkgs/by-name/ze/zed-editor/package.nix" { };

      zed-fhs = pkgs.buildFHSUserEnv {
        name = "zed";
        targetPkgs = pkgs: [ zed-editor ];
        runScript = "zed";
      };

      homeManagerNixosModules = [
        (
          { ... }:
          {
            imports = [
              home-manager.nixosModules.home-manager
              vim.nixosModules.${system}
            ];

            home-manager.useUserPackages = true;
            home-manager.useGlobalPkgs = true;
            home-manager.backupFileExtension = "backup";
            home-manager.users.user = import ./home/user/user.nix {
              inherit stateVersion;
              pkgs = import nixpkgs {
                inherit system;
                overlays = overlays;
                config = {
                  allowUnfree = true;
                  permittedInsecurePackages = [
                    "electron" # le sigh
                  ];
                };
              };
            };
          }
        )
      ];

      devShellInner = pkgs.mkShell { buildInputs = [ zed-fhs ]; };

    in
    {
      inherit lib pkgs devShellInner;
      nixosConfigurations.nixos = lib.nixosSystem {
        inherit system;
        modules = systemNixosModules ++ hyprlandNixosModules ++ homeManagerNixosModules;
      };
      specialArgs = {
        inherit inputs outputs;
      };
      devShell.${system} = devShellInner;
    };
      # hydra
  # content addressible
}
