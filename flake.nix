{
  outputs =
    inputs: # flake-parts.lib.mkFlake
    rec {
      lib = import ./lib inputs;
      hosts = import ./hosts inputs; # inputs.host?
      configurations = lib.make-nixos-configurations hosts;
      vm-configurations = lib.make-vm-configurations hosts;
      unattended-installer-configurations = lib.make-unattended-installer-configurations configurations;
      nixosConfigurations = configurations // vm-configurations // unattended-installer-configurations;
    };
  inputs = {
    microvm.url = "github:astro/microvm.nix";
    zen-browser.url = "github:0xc000022070/zen-browser-flake";
    nix-search.url = "github:diamondburned/nix-search";
    nix-flatpak.url = "github:gmodena/nix-flatpak";
    determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/0.1";
    ssh-to-age.url = "github:Mic92/ssh-to-age";
    impermanence.url = "github:Nix-community/impermanence";
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    arunoruto.url = "github:arunoruto/flake";
    unattended-installer.url = "github:developing-today-forks/nixos-unattended-installer";
    nixpkgs.url = "github:developing-today-forks/nixpkgs";
    # nixpkgs.url = "github:dezren39/nixpkgs";
    # nixpkgs.url = "github:NixOS/nixpkgs";
    # nixpkgs-stable.url = "github:developing-today-forks/nixpkgs";
    # nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.11";
    nixpkgs-stable.url = "github:nixos/nixpkgs/nixos-24.05";
    # nixpkgs-stable.url = "github:dezren39/nixpkgs";
    # nixpkgs-stable.url = "github:NixOS/nixpkgs";
    sops-nix = {
      url = "github:developing-today-forks/sops-nix";
      # url = "github:mic92/sops-nix";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    systems = {
      # TODO: use this?
      # url = "github:nix-systems/default-linux";
      url = "github:nix-systems/default";
      # inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      # TODO: use this?
      url = "https://flakehub.com/f/numtide/flake-utils/*.tar.gz";
      inputs.systems.follows = "systems";
    };
    flake-compat = {
      # TODO: use this?
      url = "https://flakehub.com/f/edolstra/flake-compat/1.0.1.tar.gz";
      flake = false;
    };
    gitignore = {
      # TODO: use this?
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    waybar = {
      # TODO: use this?
      url = "github:Alexays/Waybar";
      # inputs.nixpkgs.follows = "nixpkgs";
    };
    neovim-src = {
      url = "github:neovim/neovim";
      flake = false;
    };
    flake-parts = {
      # TODO: use this?
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
    };
    nixvim = {
      url = "github:nix-community/nixvim";
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
      url = "github:lnl7/nix-darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      # TODO: use this?
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    vim = {
      url = "path:./pkgs/vim";
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
    nix-topology = {
      url = "github:oddlama/nix-topology";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
      inputs.devshell.follows = "devshell";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };
    devshell = {
      # TODO: use this?
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pre-commit-hooks = {
      # TODO: use this?
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.flake-compat.follows = "flake-compat";
      inputs.gitignore.follows = "gitignore";
    };
    yazi = {
      # TODO: use this?
      url = "github:sxyazi/yazi";
      # not following to allow using yazi cache
      # inputs.nixpkgs.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
      # inputs.rust-overlay.follows = "rust-overlay";
    };
    omnix.url = "github:juspay/omnix"; # TODO: use this?
    # switch to flakes for hyprland, use module https://wiki.hyprland.org/Nix/Hyprland-on-NixOS/
    hyprland = {
      url = "git+https://github.com/hyprwm/Hyprland?submodules=1";
      # url = "github:hyprwm/Hyprland";
      inputs.nixpkgs.follows = "nixpkgs"; # MESA/OpenGL HW workaround
    };
    # nix-topology.nixosModules.default
    # terraform-nix-ng https://www.haskellforall.com/2023/01/terraform-nixos-ng-modern-terraform.html https://github.com/Gabriella439/terraform-nixos-ng
    # flakehub fh
    # rust-overlay = { # TODO: use this?
    #   url = "github:oxalica/rust-overlay";
    #   # follows?
    # };
    nixos-hardware.url = "github:nixos/nixos-hardware";
    # nix-colors.url = "github:misterio77/nix-colors"; # bertof/nix-rice # TODO: use this?
    # firefox-addons = { # TODO: use this?
    #   url = "gitlab:rycee/nur-expressions?dir=pkgs/firefox-addons";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-gaming = { # TODO: use this?
    #   url = "github:fufexan/nix-gaming";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # trustix = { # TODO: use this?
    #   url = "github:nix-community/trustix";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-inspect = { # TODO: use this?
    #   url = "github:bluskript/nix-inspect";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nixos-wsl = { # TODO: use this?
    #   url = "github:nix-community/NixOS-WSL";
    #   inputs.nixpkgs.follows = "nixpkgs";
    # };
  };
  nixConfig = {
    # unfortunately can't import, but this should be equal to ./hosts/nixconfig.nix
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
    trusted-users = [ "root" ];
    # trusted-users = [ "user" ];
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    substituters = [
      "https://cache.nixos.org"
      "https://yazi.cachix.org"
      # TODO: priority order
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
    # allow-dirty = false;
    # builders-use-substitutes = true;
    fallback = true;
    log-lines = 128;
    # pure-eval = true;
    # run-diff-hook = true;
    # secret-key-files
    show-trace = true;
    # tarball-ttl = 0;
    # trace-function-calls = true;
    trace-verbose = true;
    # use-xdg-base-directories = true;
    allow-dirty = true;
    # buildMachines = [ ];
    # distributedBuilds = true;
    # # optional, useful when the builder has a faster internet connection than yours
    # extraOptions = ''
    #   builders-use-substitutes = true
    # '';
    auto-optimise-store = true;
    # pure-eval = true; # this should be true..
    pure-eval = false; # sometimes home-manager needs to change manifest.nix ? idk i just code here
    restrict-eval = false; # could i even make a conclusive list of domains to allow access to?
    use-registries = true;
    use-cgroups = true;
  };
  description = "developing.today NixOS configuration";
}
