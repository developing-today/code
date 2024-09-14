{
  description = "developing.today NixOS configuration";
  inputs = {
    # switch to flakes, use module https://wiki.hyprland.org/Nix/Hyprland-on-NixOS/
    #nixpkgs.url = "github:NixOS/nixpkgs";
    #nixpkgs.url = "github:dezren39/nixpkgs";
    nixpkgs.url = "github:dezren39/nixpkgs/main";
    #nixpkgs.url = "github:dezren39/nixpkgs/rev";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2305.491756.tar.gz"; # /nixos-23.11";
    #nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1.0.tar.gz"; # /nixos-unstable"; # /nixos-23.11";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.05";
    sops-nix = {
      url = "github:mic92/sops-nix";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    systems = {
      # url = "github:nix-systems/default-linux";
      url = "github:nix-systems/default";
      # inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # */ # inputs.systems
      inputs.systems.follows = "systems";
    };
    flake-compat = {
      url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz";
      flake = false;
    };
    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    waybar = {
      url = "github:Alexays/Waybar";
      #inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-src = {
      url = "github:neovim/neovim";
      flake = false;
    };
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
      inputs.git-hooks.follows = "git-hooks";
      inputs.neovim-src.follows = "neovim-src";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.gitignore.follows = "gitignore";
      inputs.flake-compat.follows = "flake-compat";
    };
    zig-overlay = {
      url = "github:mitchellh/zig-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
      # inputs.flake-parts.follows = "flake-parts";
    };
    nixvim = {
      url = "github:nix-community/nixvim";
      #url = "github:developing-today-forks/nix-community_nixvim";
      #url = "github:developing-today-forks/nixvim-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
      inputs.devshell.follows = "devshell";
      # inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.git-hooks.follows = "git-hooks";
      inputs.treefmt-nix.follows = "treefmt-nix";
      inputs.nix-darwin.follows = "nix-darwin";
    };
    nix-darwin = {
      url = "github:lnl7/nix-darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./pkgs/vim";
      #url = "github:developing-today/code?dir=src/vim";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixvim.follows = "nixvim";
      inputs.systems.follows = "systems";
      inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.neovim-nightly-overlay.follows = "neovim-nightly-overlay";
      inputs.git-hooks.follows = "git-hooks";
      inputs.treefmt-nix.follows = "treefmt-nix";
      inputs.nix-darwin.follows = "nix-darwin";
      inputs.gitignore.follows = "gitignore";
      inputs.devshell.follows = "devshell";
      inputs.neovim-src.follows = "neovim-src";
    };
    #     TODO: flakehub fh
    nix-topology = {
      url = "github:oddlama/nix-topology";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.devshell.follows = "devshell";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };
    devshell = {
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
    };
    pre-commit-hooks = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
      inputs.gitignore.follows = "gitignore";
    };
    # rust-overlay = {
    #   url = "github:oxalica/rust-overlay";
    #   # follows?
    # };
    yazi = {
      url = "github:sxyazi/yazi";
      # not following to allow using yazi cache
      # inputs.nixpkgs.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
      # inputs.rust-overlay.follows = "rust-overlay";
    };
    omnix.url = "github:juspay/omnix";
    #     hardware.url = "github:nixos/nixos-hardware";
    #     systems.url = "github:nix-systems/default-linux";
    #     hardware.url = "github:nixos/nixos-hardware";
    #     nix-colors.url = "github:misterio77/nix-colors"; # bertof/nix-rice
    #     firefox-addons = {
    #       url = "gitlab:rycee/nur-expressions?dir=pkgs/firefox-addons";
    #       inputs.nixpkgs.follows = "nixpkgs";
    #     };
    #     nix-gaming = {
    #       url = "github:fufexan/nix-gaming";
    #       inputs.nixpkgs.follows = "nixpkgs";
    #     };
    #     trustix = {
    #       url = "github:nix-community/trustix";
    #       inputs.nixpkgs.follows = "nixpkgs";
    #     };
    #     impermanence = {
    #       url = "github:Nix-community/impermanence";
    #     };
    #     nix-inspect = {
    #       url = "github:bluskript/nix-inspect";
    #       inputs.nixpkgs.follows = "nixpkgs";
    #     };
    #     nixos-wsl = {
    #       url = "github:nix-community/NixOS-WSL";
    #       inputs.nixpkgs.follows = "nixpkgs";
    #     };
  };
  #  lib.fakeSha256 and lib.fakeSha512
    outputs = inputs@{ self, nixpkgs, flake-parts, ... }:
      flake-parts.lib.mkFlake { inherit self; } {
        systems = [ "x86_64-linux" ];
        perSystem = { config, self', inputs', pkgs, system, ... }: {
          _module.args.pkgs = import nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              permittedInsecurePackages = [
                "olm-3.2.16"
                "electron"
                "qtwebkit-5.212.0-alpha4"
              ];
            };
            overlays = [
              inputs.vim.overlay.${system}
              inputs.yazi.overlays.default
              inputs.waybar.overlays.default
              (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
            ];
          };
        };

        flake =
        # let
        #     system = "x86_64-linux";
            # pkgs = import nixpkgs {
            #   inherit system;
            #   config = {
            #     allowUnfree = true;
            #     permittedInsecurePackages = [
            #       "olm-3.2.16"
            #       "electron"
            #       "qtwebkit-5.212.0-alpha4"
            #     ];
            #   };
            #   overlays = [
            #     inputs.vim.overlay.${system}
            #     inputs.yazi.overlays.default
            #     inputs.waybar.overlays.default
            #     # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
            #   ];
            # };
        # in {
        {
          nixosConfigurations = (import ./hosts) {
            inherit inputs;
            lib = inputs.nixpkgs.lib // inputs.home-manager.lib;
            outputs = self;
          };
        };
      };
      # TODO: rootPath = ./.; # self.outPath # builtins.path
      #       TODO: terraform-nix-ng https://www.haskellforall.com/2023/01/terraform-nixos-ng-modern-terraform.html https://github.com/Gabriella439/terraform-nixos-ng
      #     nix-topology.nixosModules.default
      #         homeConfigurations = {
      #           "user@laptop-framework" = lib.homeManagerConfiguration {
  nixConfig = {
    # should match nix.settings
    experimental-features = [
      "auto-allocate-uids"
      "ca-derivations"
      "cgroups"
      "dynamic-derivations"
      "fetch-closure"
      "flakes"
      "git-hashing"
      # "local-overlay-store" # look into this
      # "mounted-ssh-store" # look into this
      "nix-command"
      # "no-url-literals" # <- removed no-url-literals for flakehub testing
      "parse-toml-timestamps"
      "read-only-local-store"
      "recursive-nix"
      "verified-fetches"
    ];
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    #     trusted-users = [ "root" "@wheel" ];
    trusted-users = [ "root" ];
    substituters = [
      # TODO: priority order
      "https://cache.nixos.org" # priority
      "https://yazi.cachix.org"
      #         "https://nix-community.cachix.org"
      #         "https://nix-gaming.cachix.org"
      #         "https://cache.m7.rs"
      #         "https://nrdxp.cachix.org"
      #         "https://numtide.cachix.org"
      #         "https://colmena.cachix.org"
      #         "https://sylvorg.cachix.org"
    ];
    trusted-substituters = [
      "https://cache.nixos.org" # priority
      "https://yazi.cachix.org"
      #         "https://nix-community.cachix.org"
      #         "https://nix-gaming.cachix.org"
      #         "https://cache.m7.rs"
      #         "https://nrdxp.cachix.org"
      #         "https://numtide.cachix.org"
      #         "https://colmena.cachix.org"
      #         "https://sylvorg.cachix.org"
    ];
    /*
      extra-substituters = [ "https://yazi.cachix.org" ];
      extra-trusted-public-keys = [ "yazi.cachix.org-1:Dcdz63NZKfvUCbDGngQDAZq6kOroIrFoyO064uvLh8k=" ];
    */
    trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "yazi.cachix.org-1:Dcdz63NZKfvUCbDGngQDAZq6kOroIrFoyO064uvLh8k="
      #         "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      #         "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
      #         "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
      #         "nrdxp.cachix.org-1:Fc5PSqY2Jm1TrWfm88l6cvGWwz3s93c6IOifQWnhNW4="
      #         "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      #         "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
      #         "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
    ];
    extra-substituters = [ ];
    extra-trusted-substituters = [ ];
    extra-trusted-public-keys = [ ];
    http-connections = 100; # 128 default:25
    max-substitution-jobs = 64; # 128 default:16
    # Store:querySubstitutablePaths Store::queryMissing binary-caches-parallel-connections fileTransferSettings.httpConnections
    keep-outputs = true; # Nice for developers
    keep-derivations = true; # Idem
    accept-flake-config = true;
    #     allow-dirty = false;
    allow-dirty = true;
    #     builders-use-substitutes = true;
    fallback = true;
    log-lines = 128;
    #     pure-eval = true;
    # run-diff-hook = true;
    # secret-key-files
    show-trace = true;
    # tarball-ttl = 0;
    # trace-function-calls = true;
    trace-verbose = true;
    # use-xdg-base-directories = true;
    #     allow-dirty = false;
    #       buildMachines = [ ];
    #       distributedBuilds = true;
    #       # optional, useful when the builder has a faster internet connection than yours
    #       extraOptions = ''
    #         builders-use-substitutes = true
    #       '';
    auto-optimise-store = true;
    #pure-eval = true;
    pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
    restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
    use-registries = true;
    use-cgroups = true;
    #     };
    #     package = pkgs.nixVersions.nix_2_23;
    #     optimise.automatic = true;
    # auto-optimise-store = true;
    #     gc = {
    #       automatic = true;
    #       dates = "weekly";
    #       options = "--delete-older-than 180d";
    #     };
  };
}
