{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    #     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";

    # todo: figure out that hyprland flake
    home-manager = {
      url = "github:nix-community/home-manager"; #*/
      #url = "https://flakehub.com/f/nix-community/home-manager/*.tar.gz"; #*/
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; #*/ # inputs.systems
    flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    neovim-flake = {
      url = "github:neovim/neovim?dir=contrib";
      #inputs.nixpkgs.follows = "nixpkgs"; # harpoon broken in unstable with nightly neovim
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
    };

    nixvim = {
      # url = "github:nix-community/nixvim";
      url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./programs/nixvim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.nixvim.follows = "nixvim";
      inputs.beautysh.follows = "beautysh";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
      inputs.neovim-flake.follows = "neovim-flake";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
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
  };
  outputs = {
    self,
    nixpkgs,
    home-manager,
    vim,
    ...
  } @ inputs: let
    system = "x86_64-linux"; # something something flake-utils
    vimOverlay = vim.overlay.${system};
    overlays = [
      vimOverlay
    ];
  in {
    homeManagerNixosModules = stateVersion: [
      ({...}: {
        imports = [
          home-manager.nixosModules.home-manager
          vim.nixosModules.${system}
        ];

        home-manager.useUserPackages = true;
        home-manager.useGlobalPkgs = true;
        home-manager.users.user = import ./users/user.nix {
          inherit stateVersion;
          pkgs = import nixpkgs {
            inherit system;
            overlays = overlays;
            config.allowUnfree = true;
          };
        };
      })
    ];
    vim-overlay = vimOverlay;
  };
}
