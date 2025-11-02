rec {
  outputs =
    inputs: # flake-parts.lib.mkFlake
    let
      lib = import ./lib inputs;
    in
    lib.merge [
      rec {
        inherit lib nixConfig description;
        hosts = import ./nixos/hosts inputs; # inputs.host?
        configurations = lib.make-nixos-configurations hosts;
        vm-configurations = lib.make-vm-configurations hosts;
        unattended-installer-configurations = lib.make-unattended-installer-configurations configurations;
        nixosConfigurations = lib.merge [
          configurations
          vm-configurations
          unattended-installer-configurations
        ];
      }
      (lib.make-vim)
      (lib.make-clan)
    ];
  inputs = {
    nixgl = {
      url = "github:nix-community/nixGL";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    roc = {
      url = "github:roc-lang/roc?shallow=1";
      #inputs.nixpkgs.follows = "nixpkgs"; # https://roc.zulipchat.com/#narrow/channel/231634-beginners/topic/roc.20nix.20flake/near/553273845
      # inputs.rust-overlay.follows = "rust-overlay";
      inputs.flake-utils.follows = "flake-utils";
      inputs.flake-compat.follows = "flake-compat";
    };
    hyprland-qtutils = {
      url = "github:hyprwm/hyprland-qtutils";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
      inputs.hyprland-qt-support.follows = "hyprland-qt-support";
    };
    hyprland-qt-support = {
      url = "github:hyprwm/hyprland-qt-support";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
      inputs.hyprlang.follows = "hyprlang";
    };
    solaar = {
      url = "https://flakehub.com/f/Svenum/Solaar-Flake/*.tar.gz"; # For latest stable version
      #url = "https://flakehub.com/f/Svenum/Solaar-Flake/0.1.1.tar.gz" # uncomment line for solaar version 1.1.13
      #url = "github:Svenum/Solaar-Flake/main"; # Uncomment line for latest unstable version
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # TODO: ?? use git instead of github ?? "git+https://github.com/NixOS/nixpkgs?shallow=1&ref=nixpkgs-unstable";
    rose-pine-hyprcursor.url = "github:ndom91/rose-pine-hyprcursor?shallow=1";
    nixos-facter-modules.url = "github:numtide/nixos-facter-modules?shallow=1";
    affinity-nix.url = "github:mrshmllow/affinity-nix/c17bda86504d6f8ded13e0520910b067d6eee50f?shallow=1"; # need 2.5.7 before can update
    nix-output-monitor = {
      url = "github:maralorn/nix-output-monitor?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    clan-core.url = "https://git.clan.lol/clan/clan-core/archive/main.tar.gz"; # shallow=1
    server.url = "github:developing-today-forks/server.nix/master?shallow=1";
    microvm.url = "github:astro/microvm.nix?shallow=1";
    zen-browser.url = "github:0xc000022070/zen-browser-flake?shallow=1";
    nix-search.url = "github:diamondburned/nix-search?shallow=1";
    nix-flatpak.url = "github:gmodena/nix-flatpak?shallow=1";
    # determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/0.1"; # ?shallow=1
    ssh-to-age.url = "github:Mic92/ssh-to-age?shallow=1";
    impermanence.url = "github:Nix-community/impermanence?shallow=1";
    disko = {
      url = "github:nix-community/disko?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    #arunoruto.url = "github:arunoruto/flake?shallow=1";
    unattended-installer.url = "github:developing-today-forks/nixos-unattended-installer?shallow=1";

    # nixpkgs-inner.url = "github:developing-today-forks/nixpkgs?shallow=1";
    #nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable?shallow=1";
    #nixpkgs.url = "github:developing-today-forks/nixpkgs?shallow=1";
    #nixpkgs.url = "github:developing-today-forks/nixpkgs/e5fcba7ae622ed9f40c214a0d61e0bcf8f49b32";
    #nixpkgs.url = "github:developing-today-forks/nixpkgs/2025-11-01_nixos-unstable?shallow=1";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05?shallow=1";
    # nixpkgs.url = "github:dezren39/nixpkgs?shallow=1";
    # nixpkgs = {
    #nixpkgs-master.url = "github:NixOS/nixpkgs/master?shallow=1";
    nixpkgs-master.url = "github:NixOS/nixpkgs/nixos-25.05?shallow=1";
    #nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable?shallow=1";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-25.05?shallow=1";
    #   url = "github:numtide/nixpkgs-unfree?ref=nixos-unstable?shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs-inner";
    # };
    # nixpkgs-stable.url = "github:developing-today-forks/nixpkgs?shallow=1";
    #nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.11?shallow=1";
    # nixpkgs-stable.url = "github:dezren39/nixpkgs?shallow=1";
    # nixpkgs-stable.url = "github:NixOS/nixpkgs?shallow=1";
    #nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.05?shallow=1";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-25.05?shallow=1";
    sops-nix = {
      url = "github:developing-today-forks/sops-nix?shallow=1";
      # url = "github:mic92/sops-nix";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      #url = "github:developing-today-forks/home-manager?shallow=1";
      # inputs.nixpkgs.follows = "nixpkgs";
      #url = "github:nix-community/home-manager?shallow=1";
      url = "github:nix-community/home-manager/release-25.05?shallow=1";
      #inputs.nixpkgs.follows = "nixpkgs-master";
    };
    systems = {
      # TODO: use this?
      # url = "github:nix-systems/default-linux";
      url = "github:nix-systems/default?shallow=1";
      # inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      # TODO: use this?
      url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz"; # ?shallow=1
      inputs.systems.follows = "systems";
    };
    flake-compat = {
      # TODO: use this?
      url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz"; # ?shallow=1
      flake = false;
    };
    gitignore = {
      # TODO: use this?
      url = "github:hercules-ci/gitignore.nix?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    waybar = {
      # TODO: use this?
      url = "github:Alexays/Waybar?shallow=1";
      # inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-src = {
      url = "github:neovim/neovim?shallow=1";
      flake = false;
    };
    flake-parts = {
      # TODO: use this?
      url = "github:hercules-ci/flake-parts?shallow=1";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects?shallow=1";
      inputs.flake-parts.follows = "flake-parts";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
      inputs.hercules-ci-effects.follows = "hercules-ci-effects";
      inputs.flake-compat.follows = "flake-compat";
      inputs.git-hooks.follows = "git-hooks";
      inputs.neovim-src.follows = "neovim-src";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.gitignore.follows = "gitignore";
      inputs.flake-compat.follows = "flake-compat";
    };
    zig-overlay = {
      url = "github:mitchellh/zig-overlay?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
    };
    nixvim = {
      url = "github:nix-community/nixvim?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.home-manager.follows = "home-manager";
      inputs.devshell.follows = "devshell";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-parts.follows = "flake-parts";
      inputs.git-hooks.follows = "git-hooks";
      inputs.treefmt-nix.follows = "treefmt-nix";
      inputs.nix-darwin.follows = "nix-darwin";
    };
    nix-darwin = {
      # TODO: use this?
      url = "github:lnl7/nix-darwin?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      # TODO: use this?
      url = "github:numtide/treefmt-nix?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-topology = {
      url = "github:oddlama/nix-topology?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.devshell.follows = "devshell";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };
    devshell = {
      # TODO: use this?
      url = "github:numtide/devshell?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pre-commit-hooks = {
      # TODO: use this?
      url = "github:cachix/pre-commit-hooks.nix?shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.gitignore.follows = "gitignore";
    };
    hyprlang = {
      url = "github:hyprwm/hyprlang";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
    };
    yazi = {
      # TODO: use this?
      url = "github:sxyazi/yazi?shallow=1";
      # not following to allow using yazi cache
      # inputs.nixpkgs.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
      # inputs.rust-overlay.follows = "rust-overlay";
    };
    omnix.url = "github:juspay/omnix?shallow=1"; # TODO: use this?
    # switch to flakes for hyprland, use module https://wiki.hyprland.org/Nix/Hyprland-on-NixOS/
    hypr-dynamic-cursors = {
      url = "github:VirtCode/hypr-dynamic-cursors?shallow=1";
      inputs.hyprland.follows = "hyprland"; # to make sure that the plugin is built for the correct version of hyprland
    };
    hyprland = {
      url = "git+https://github.com/hyprwm/Hyprland?submodules=1&shallow=1";
      # url = "github:hyprwm/Hyprland";
      inputs.nixpkgs.follows = "nixpkgs"; # MESA/OpenGL HW workaround
      inputs.hyprcursor.follows = "hyprcursor";
      inputs.hyprlang.follows = "hyprlang";
    };
    hyprcursor = {
      # url = "git+https://github.com/hyprwm/hyprcursor?submodules=1&shallow=1";
      url = "git+https://github.com/dezren39/hyprcursor?ref=patch-1&submodules=1&shallow=1";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.systems.follows = "systems";
    };
    # nix-topology.nixosModules.default
    # terraform-nix-ng https://www.haskellforall.com/2023/01/terraform-nixos-ng-modern-terraform.html https://github.com/Gabriella439/terraform-nixos-ng
    # flakehub fh
    # rust-overlay = { # TODO: use this?
    #   url = "github:oxalica/rust-overlay?shallow=1";
    #   # follows?
    # };
    nixos-hardware.url = "github:nixos/nixos-hardware?shallow=1";
    # nix-colors.url = "github:misterio77/nix-colors"; # bertof/nix-rice # TODO: use this?
    # firefox-addons = { # TODO: use this?
    #   url = "gitlab:rycee/nur-expressions?dir=pkgs/firefox-addons&shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-gaming = { # TODO: use this?
    #   url = "github:fufexan/nix-gaming?shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # trustix = { # TODO: use this?
    #   url = "github:nix-community/trustix?shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-inspect = { # TODO: use this?
    #   url = "github:bluskript/nix-inspect?shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nixos-wsl = { # TODO: use this?
    #   url = "github:nix-community/NixOS-WSL?shallow=1";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
  };
  nixConfig = {
    experimental-features = [
      "auto-allocate-uids"
      "ca-derivations"
      "cgroups"
      "dynamic-derivations"
      "fetch-closure"
      "fetch-tree"
      "flakes"
      "git-hashing"
      # "local-overlay-store" # look into this
      # "mounted-ssh-store" # look into this
      "nix-command"
      # "no-url-literals" # <- removed no-url-literals for flakehub testing
      "parse-toml-timestamps"
      "pipe-operators"
      "read-only-local-store"
      "recursive-nix"
      "verified-fetches"
    ];
    trusted-users = [ "root" ];
    #       trusted-users = [ "user" ];
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    substituters = [
      # TODO: priority order
      "https://cache.nixos.org"
      "https://yazi.cachix.org"
      # "https://binary.cachix.org"
      # "https://nix-community.cachix.org"
      # "https://nix-gaming.cachix.org"
      # "https://cache.m7.rs"
      # "https://nrdxp.cachix.org"
      # "https://numtide.cachix.org"
      # "https://colmena.cachix.org"
      # "https://sylvorg.cachix.org"
    ];
    trusted-substituters = [
      "https://cache.nixos.org"
      "https://yazi.cachix.org"
      # "https://binary.cachix.org"
      # "https://nix-community.cachix.org"
      # "https://nix-gaming.cachix.org"
      # "https://cache.m7.rs"
      # "https://nrdxp.cachix.org"
      # "https://numtide.cachix.org"
      # "https://colmena.cachix.org"
      # "https://sylvorg.cachix.org"
    ];
    trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "yazi.cachix.org-1:Dcdz63NZKfvUCbDGngQDAZq6kOroIrFoyO064uvLh8k="
      # "binary.cachix.org-1:66/C28mr67KdifepXFqZc+iSQcLENlwPqoRQNnc3M4I="
      # "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      # "nix-gaming.cachix.org-1:nbjlureqMbRAxR1gJ/f3hxemL9svXaZF/Ees8vCUUs4="
      # "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg="
      # "nrdxp.cachix.org-1:Fc5PSqY2Jm1TrWfm88l6cvGWwz3s93c6IOifQWnhNW4="
      # "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
      # "colmena.cachix.org-1:7BzpDnjjH8ki2CT3f6GdOk7QAzPOl+1t3LvTLXqYcSg="
      # "sylvorg.cachix.org-1:xd1jb7cDkzX+D+Wqt6TemzkJH9u9esXEFu1yaR9p8H8="
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
    #     builders-use-substitutes = true;
    fallback = true;
    log-lines = 128;
    #     pure-eval = true;
    # run-diff-hook = true;
    # secret-key-files
    show-trace = true;
    # tarball-ttl = 0;
    tarball-ttl = 259200; # 3600 * 72;
    # trace-function-calls = true;
    trace-verbose = true;
    # use-xdg-base-directories = true;
    allow-dirty = true;
    /*
      buildMachines = [ ];
      distributedBuilds = true;
      # optional, useful when the builder has a faster internet connection than yours
      extraOptions = ''
        builders-use-substitutes = true
      '';
    */
    # extraOptions = ''
    #   flake-registry = ""
    # '';
    auto-optimise-store = true;
    #pure-eval = true;
    pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
    restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
    use-registries = true; # clan and others rely on flake registry
    use-cgroups = true;
  };
  description = "developing.today NixOS configuration";
}

#TODO:
# make optional https://git.clan.lol/clan/clan-core/src/branch/main/flake.nix#L115
# make private/local https://git.clan.lol/clan/clan-core/src/branch/main/flake.nix#L53
