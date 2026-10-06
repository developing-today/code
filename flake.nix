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
      lib.make-vim
      lib.make-clan
      lib.make-root-apps
      lib.make-id
    ];
  inputs = {
    nixgl = {
      url = "github:nix-community/nixGL";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      # inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-utils.follows = "flake-utils";
    };
    roc = {
      # old flake.nix was removed from roc-lang/roc main; official flake moved to roc-overlay
      url = "github:roc-lang/roc-overlay"; # ?shallow=1";
      #inputs.nixpkgs.follows = "nixpkgs"; # https://roc.zulipchat.com/#narrow/channel/231634-beginners/topic/roc.20nix.20flake/near/553273845
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    #hyprland-qtutils = {
    #   url = "github:hyprwm/hyprland-qtutils";
    #   inputs.nixpkgs.follows = "hyprland"; #nixpkgs";
    #   inputs.systems.follows = "systems";
    #   inputs.hyprland-qt-support.follows = "hyprland-qt-support";
    # };
    # hyprland-qt-support = {
    #   url = "github:hyprwm/hyprland-qt-support";
    #   inputs.nixpkgs.follows = "hyprland"; #nixpkgs";
    #   inputs.systems.follows = "systems";
    #   inputs.hyprlang.follows = "hyprlang";
    # };
    # solaar flake removed: module upstreamed into nixpkgs as programs.solaar
    #url = "https://flakehub.com/f/Svenum/Solaar-Flake/0.1.1.tar.gz" # uncomment line for solaar version 1.1.13
    #url = "github:Svenum/Solaar-Flake/main"; # Uncomment line for latest unstable version
    # TODO: ?? use git instead of github ?? "git+https://github.com/NixOS/nixpkgs"; #?shallow=1&ref=nixpkgs-unstable";
    #rose-pine-hyprcursor.url = "github:ndom91/rose-pine-hyprcursor"; #?shallow=1";
    nixos-facter-modules.url = "github:numtide/nixos-facter-modules"; # ?shallow=1";
    affinity-nix.url = "github:mrshmllow/affinity-nix/c17bda86504d6f8ded13e0520910b067d6eee50f"; # ?shallow=1"; # need 2.5.7 before can update
    nix-output-monitor = {
      url = "github:maralorn/nix-output-monitor"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    clan-core.url = "https://git.clan.lol/clan/clan-core/archive/main.tar.gz"; # shallow=1
    # TODO: update! way out of date even as of 2026-03
    server.url = "github:developing-today-forks/server.nix/master"; # ?shallow=1";
    microvm.url = "github:astro/microvm.nix"; # ?shallow=1";
    zen-browser.url = "github:0xc000022070/zen-browser-flake"; # ?shallow=1";
    nix-search.url = "github:diamondburned/nix-search"; # ?shallow=1";
    esp-dev.url = "github:mirrexagon/nixpkgs-esp-dev/5287d6e1ca9e15ebd5113c41b9590c468e1e001b";
    # ESP-IDF 6.0.1 packaging candidate; kept separate from the supported 5.5 toolchain.
    esp-dev-6.url = "github:dvdvgt/nixpkgs-esp-dev/f9b1e211262a4cc9c1a265b227def56ef01c2d56";
    nix-flatpak.url = "github:gmodena/nix-flatpak"; # ?shallow=1";
    # determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/0.1"; # "; #?shallow=1
    ssh-to-age.url = "github:Mic92/ssh-to-age"; # ?shallow=1";
    impermanence.url = "github:Nix-community/impermanence"; # ?shallow=1";
    disko = {
      url = "github:nix-community/disko"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    #arunoruto.url = "github:arunoruto/flake"; #?shallow=1";
    # # TODO: update! way out of date even as of 2026-03
    unattended-installer.url = "github:developing-today-forks/nixos-unattended-installer"; # ?shallow=1";

    # 2026-08-21: rebased fork patch (neededForBoot) onto latest master
    # neededForBoot patch applied via patches/nixpkgs/neededforboot-nixos-unstable.patch (see lib/default.nix)
    # That patch only applies to nixos-unstable; it does NOT apply to the old
    # 24.11 tree this input used to be locked to, which made applyPatches (and
    # therefore every nixosConfiguration) fail. Aliased to nixpkgs-unstable so
    # the two can never drift apart again.
    nixpkgs.follows = "nixpkgs-unstable";
    nixpkgs-25.url = "github:NixOS/nixpkgs/nixos-unstable"; # ?shallow=1";
    nixpkgs-stable.url = "github:NixOS/nixpkgs"; # ?shallow=1";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable"; # channel branch: fully cached on cache.nixos.org (master is not)
    nixpkgs-master.url = "github:NixOS/nixpkgs"; # ?shallow=1";

    sops-nix = {
      # TODO: update! way out of date even as of 2026-03
      url = "github:developing-today-forks/sops-nix"; # ?shallow=1";
      # url = "github:mic92/sops-nix";
      inputs.nixpkgs-stable.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager"; # ?shallow=1";
    };
    systems = {
      # TODO: use this?
      # url = "github:nix-systems/default-linux";
      url = "github:nix-systems/default"; # ?shallow=1";
    };
    flake-utils = {
      # TODO: use this?
      url = "github:numtide/flake-utils"; # ?shallow=1
      inputs.systems.follows = "systems";
    };
    flake-compat = {
      # TODO: use this?
      url = "github:edolstra/flake-compat"; # ?shallow=1
      flake = false;
    };
    gitignore = {
      # TODO: use this?
      url = "github:hercules-ci/gitignore.nix"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    waybar = {
      # TODO: use this?
      url = "github:Alexays/Waybar"; # ?shallow=1";
    };
    neovim-src = {
      url = "github:neovim/neovim"; # ?shallow=1";
      flake = false;
    };
    flake-parts = {
      # TODO: use this?
      url = "github:hercules-ci/flake-parts"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      # inputs.nixpkgs-lib.follows = "nixpkgs";
      inputs.nixpkgs-lib.follows = "nixpkgs-unstable";
    };
    hercules-ci-effects = {
      url = "github:hercules-ci/hercules-ci-effects"; # ?shallow=1";
      inputs.flake-parts.follows = "flake-parts";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    neovim-nightly-overlay = {
      url = "github:nix-community/neovim-nightly-overlay"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      # inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-parts.follows = "flake-parts";
      inputs.neovim-src.follows = "neovim-src";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.gitignore.follows = "gitignore";
      inputs.flake-compat.follows = "flake-compat";
    };
    zig-overlay = {
      url = "github:mitchellh/zig-overlay"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-compat.follows = "flake-compat";
      inputs.flake-utils.follows = "flake-utils";
    };
    nixvim = {
      # url = "github:nix-community/nixvim"; # ?shallow=1";
      url = "github:nix-community/nixvim/main"; # ?shallow=1";
      #url = "github:nix-community/nixvim/nixos-25.05"; #?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      # inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-parts.follows = "flake-parts";
    };
    nix-darwin = {
      # TODO: use this?
      url = "github:lnl7/nix-darwin"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    treefmt-nix = {
      # TODO: use this?
      url = "github:numtide/treefmt-nix"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    nix-topology = {
      url = "github:oddlama/nix-topology"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-utils.follows = "flake-utils";
      inputs.devshell.follows = "devshell";
      inputs.pre-commit-hooks.follows = "pre-commit-hooks";
    };
    devshell = {
      # TODO: use this?
      url = "github:numtide/devshell"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
    pre-commit-hooks = {
      # TODO: use this?
      url = "github:cachix/pre-commit-hooks.nix"; # ?shallow=1";
      # TODO: revert to nixpkgs, relates to 26 breaking changings, either impermanence/nix-sops conflict with systemd-mounts change or the breaking wireless hardening changes
      #inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      inputs.flake-compat.follows = "flake-compat";
      inputs.gitignore.follows = "gitignore";
    };
    # hyprlang = {
    #   url = "github:hyprwm/hyprlang";
    #   inputs.nixpkgs.follows = "hyprland"; #nixpkgs";
    #   inputs.systems.follows = "systems";
    # };
    yazi = {
      # TODO: use this?
      url = "github:sxyazi/yazi"; # ?shallow=1";
      # not following to allow using yazi cache
      # inputs.nixpkgs.follows = "nixpkgs";
      # inputs.flake-utils.follows = "flake-utils";
      # inputs.rust-overlay.follows = "rust-overlay";
    };
    omnix.url = "github:juspay/omnix"; # ?shallow=1"; # TODO: use this?
    # switch to flakes for hyprland, use module https://wiki.hypr.land/nix/installing-hyprland-on-nixos/
    # hypr-dynamic-cursors = {
    #   url = "github:VirtCode/hypr-dynamic-cursors"; #?shallow=1";
    #   inputs.hyprland.follows = "hyprland"; # to make sure that the plugin is built for the correct version of hyprland
    # };
    hyprland = {
      # Tracks master. Required: hyprland removed hyprlang/.conf support
      # (#15539, 2026-07-22), so only a master build actually requires the Lua
      # config in config/hypr/hyprland.lua. The newest release (0.56.2, which is
      # what nixpkgs ships) still accepts .conf.
      url = "github:hyprwm/Hyprland";
      # Deliberately NOT following nixpkgs. Overriding hyprland's nixpkgs input
      # invalidates the hyprland.cachix.org cache and forces a full source build
      # of hyprland + mesa + ffmpeg. See https://wiki.hypr.land/nix/cachix/.
      # The old "MESA/OpenGL HW workaround" follows only applied on stable
      # nixpkgs; we are on nixpkgs-unstable, so the mismatch does not arise.
    };
    #  hyprcursor = {
    # url = "git+https://github.com/hyprwm/hyprcursor?submodules=1&shallow=1";
    #   url = "git+https://github.com/dezren39/hyprcursor?ref=patch-1&submodules=1&shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
    #  inputs.systems.follows = "systems";
    #};
    # nix-topology.nixosModules.default
    # terraform-nix-ng https://www.haskellforall.com/2023/01/terraform-nixos-ng-modern-terraform.html https://github.com/Gabriella439/terraform-nixos-ng
    # flakehub fh
    # rust-overlay = { # TODO: use this?
    #   url = "github:oxalica/rust-overlay"; #?shallow=1";
    #   # follows?
    # };
    nixos-hardware.url = "github:nixos/nixos-hardware"; # ?shallow=1";
    opencode = {
      url = "github:anomalyco/opencode";
      # inputs.nixpkgs.follows = "nixpkgs-master";
    };
    # OpenCode 2.x pin (v2.0.23). Used as the default `opencode` on PATH for the system,
    # as well as OpenChamber (which hard-requires opencode >= 2.0.20).
    # The 1.18.x build from `opencode` above is exposed under `opencode-v1`.
    opencode-2x = {
      url = "github:anomalyco/opencode/v2.0.23";
    };
    # Helium: privacy-focused Chromium fork by imputnet (the cobalt.tools org).
    # Not in nixpkgs and unlikely to be soon -- seven `helium: init` PRs have been
    # closed unmerged and the one still open (#498572) has been stalled since
    # 2026-08. This flake tracks upstream AppImage releases and ships NixOS and
    # home-manager modules plus browser policy support.
    helium = {
      url = "github:oxcl/nix-flake-helium-browser";
    };
    # ChatGPT desktop for Linux. OpenAI shipped an official Linux build (preview)
    # distributed from their own APT/RPM repos under persistent.oaistatic.com
    # (`Maintainer: OpenAI <support@openai.com>`). The nixpkgs `chatgpt` attr is
    # still darwin-only and only unpacks the macOS .app, so it cannot be used.
    # This flake verifies and repackages OpenAI's signed upstream Linux payload
    # rather than reimplementing it, is MIT-licensed, and is namespaced as
    # `codex-desktop` to avoid colliding with the official package name.
    chatgpt-desktop = {
      url = "github:ilysenko/codex-desktop-linux";
    };
    # Claude Desktop. Anthropic ships an official Linux build (beta, 2026-06-30)
    # but ONLY as a .deb from their own APT repo -- no AppImage/tar/rpm, and no
    # nixpkgs attr (verified: pkgs/by-name/cl/claude-desktop absent, zero code
    # search hits). This flake repackages that official .deb; since its v3.0.0 it
    # no longer extracts the Windows installer the way k3d3's older flake does,
    # which is why it is preferred here (k3d3 last saw a commit 2025-11-25 and
    # predates the official Linux release entirely).
    # Use the -fhs output: MCP servers are near-universally npx/uvx invocations
    # that break against a pure store path.
    claude-desktop = {
      url = "github:aaddrick/claude-desktop-debian";
    };
    # OpenChamber: agentic dev environment built on opencode.
    # Not in nixpkgs (no attr, no PR ever opened) and upstream ships zero Nix
    # (verified: 6471-path tree, no flake.nix). Of the four third-party flakes
    # that exist, Tarow's is the only one tracking current upstream (2.1.0) and
    # builds from source via buildNpmPackage rather than wrapping the AppImage,
    # which avoids the electron-updater-vs-immutable-store problem.
    openchamber = {
      url = "github:Tarow/openchamber-nix";
      # deliberately NOT following nixpkgs: it vendors a package-lock.json and
      # pins its own nixpkgs for the npm deps hash.
    };
    # --- BEGIN id sub-flake inputs (synced from pkgs/id/flake.nix) ---
    id-nixpkgs.follows = "nixpkgs-master";
    id-systems.follows = "systems";
    id-rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "id-nixpkgs";
    };
    id-flake-utils.follows = "flake-utils";
    id-import-tree.url = "github:vic/import-tree";
    id-flake-parts.follows = "flake-parts";
    id-bun2nix = {
      url = "github:nix-community/bun2nix";
      inputs = {
        flake-parts.follows = "id-flake-parts";
        import-tree.follows = "id-import-tree";
        nixpkgs.follows = "id-nixpkgs";
        systems.follows = "id-systems";
      };
    };
    # --- END id sub-flake inputs ---
    # nix-colors.url = "github:misterio77/nix-colors"; # bertof/nix-rice # TODO: use this?
    # firefox-addons = { # TODO: use this?
    #   url = "gitlab:rycee/nur-expressions?dir=pkgs/firefox-addons&shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-gaming = { # TODO: use this?
    #   url = "github:fufexan/nix-gaming"; #?shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
    # };
    # trustix = { # TODO: use this?
    #   url = "github:nix-community/trustix"; #?shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nix-inspect = { # TODO: use this?
    #   url = "github:bluskript/nix-inspect"; #?shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
    # };
    # nixos-wsl = { # TODO: use this?
    #   url = "github:nix-community/NixOS-WSL"; #?shallow=1";
    # inputs.nixpkgs.follows = "nixpkgs";
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
    trusted-users = [
      "root"
      # Required for the cachix substituters below (incl. hyprland.cachix.org) to
      # be honoured. Without this nix logs "ignoring the client-specified setting
      # ... because you are not a trusted user" and silently builds from source.
      "@wheel"
    ];
    #       trusted-users = [ "user" ];
    use-xdg-base-directories = true;
    builders-use-substitutes = true;
    substituters = [
      # TODO: priority order
      "https://cache.nixos.org"
      "https://yazi.cachix.org"
      "https://hyprland.cachix.org" # required by the hyprland master flake input
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
      "https://hyprland.cachix.org" # required by the hyprland master flake input
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
      "hyprland.cachix.org-1:a7pgxzMz7+chwVL3/pzj6jIBMioiJM7ypFP8PwtkuGc="
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
    # Deliberately false. This is NOT the same knob as `nix.optimise.automatic`
    # (the periodic timer, already commented out in nixos/nix/default.nix).
    # `auto-optimise-store` makes the daemon hard-link EVERY store write into
    # /nix/store/.links, and this flake's nixConfig is fed straight into
    # nix.settings by nixos/nix/settings/default.nix, so setting it here turned
    # it on system-wide. That drove .links to ~9M entries and exhausted the
    # ext4 htree index, producing a flood of
    #   cannot link "/nix/store/.links/...": No space left on device
    # despite the filesystem having 1.9T free and 92% of inodes unused.
    # ext4 never shrinks a directory, so after switching this off the directory
    # must be recreated once to actually recover:
    #   sudo rm -rf /nix/store/.links && sudo mkdir -p /nix/store/.links
    auto-optimise-store = false;
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
