# Shared Nix configuration for shell.nix and flake.nix
#
# This file ensures both environments have identical packages, environment
# variables, and shell hooks. The flake.lock provides exact version pinning.
#
# Usage in flake.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; extraFmtBins = [ bun2nixPkg ]; };
#
# Usage in shell.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };

{
  pkgs,
  extraFmtBins ? [ ],
}:

let
  # Rust toolchain from rust-toolchain.toml (includes rustc, cargo, rustfmt, clippy)
  # Works because both shell.nix and flake.nix apply rust-overlay before importing
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

  # Formatter binaries (keep in sync with treefmt.toml)
  # Used by formatter wrapper (nix fmt), nix flake checks, and devShell PATH
  fmtBins = [
    # Rust toolchain (includes rustfmt)
    rustToolchain
  ]
  ++ (with pkgs; [
    # Formatter orchestrator
    treefmt
    # Formatters and linters (keep in sync with treefmt.toml)
    biome
    nodePackages.prettier
    nixfmt
    statix
    shfmt
    shellcheck
    taplo
    # Utilities needed by formatter wrapper
    file
    bash
    just
    gnused
    findutils
  ])
  ++ extraFmtBins;

  # Native build inputs (tools, compilers)
  nativeBuildInputs =
    fmtBins
    ++ (with pkgs; [
      # Build dependencies
      pkg-config
      openssl

      # Cargo plugins
      cargo-watch
      cargo-nextest
      cargo-llvm-cov
      cargo-audit
      cargo-outdated
      cargo-machete
      cargo-edit

      # Development tools
      git
      ripgrep
      fd
      jq
      uv
      tokei
      hyperfine

      # Web development tools
      bun # JavaScript bundler and runtime (required for web builds)
      nodePackages.typescript # TypeScript for type checking

      # E2E testing (Playwright) — use playwright-driver.browsers for
      # pre-patched browser binaries that work with Playwright's CDP protocol.
      # Pin @playwright/test in e2e/package.json to match this version.
      playwright-driver.browsers

      # Manual linters (not in treefmt, run manually)
      deadnix
    ]);
in
{
  inherit rustToolchain fmtBins nativeBuildInputs;

  # Build inputs (libraries for Rust compilation)
  buildInputs = with pkgs; [ openssl ];

  # Anchor bare treefmt to current directory (for ad-hoc devshell use)
  # Without this, treefmt walks up to .git root which is wrong for pkgs/id
  # (just recipes and nix fmt use explicit --config-file/--tree-root instead)
  TREEFMT_TREE_ROOT_CMD = "pwd";

  # Packages for shell.nix / nix develop (nativeBuildInputs = all tools)
  packages = nativeBuildInputs;

  # OpenSSL environment variables
  opensslEnv = {
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
  };

  # Shared shell hook for both nix-shell and nix develop
  shellHook = ''
    export RUST_BACKTRACE=1

    # Playwright E2E testing: use Nix-provided browsers
    export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
    export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers}"

    # nix develop reconstructs PATH, losing HM session paths.
    # Restore them: devshell → HM → system
    # Guard: only do PATH reordering once (multiple use flake in .envrc)
    if [ -z "$__DEVSHELL_HM_PATH_DONE" ]; then
      export __DEVSHELL_HM_PATH_DONE=1

      # Source HM session vars and extract any PATH additions
      _pre_profile_path="$PATH"
      unset __HM_SESS_VARS_SOURCED
      . "$HOME/.profile" 2>/dev/null || true
      _hm_paths="''${PATH%:$_pre_profile_path}"
      [ "$_hm_paths" = "$PATH" ] && _hm_paths=""

      # Separate devshell from system using NIX_PROFILES (NixOS env var)
      # Non-NixOS: NIX_PROFILES is unset, everything stays in devshell, PATH unchanged
      _devshell_only=""
      _system_only=""
      _remaining="$_pre_profile_path"
      while [ -n "$_remaining" ]; do
        _e="''${_remaining%%:*}"
        [ "$_e" = "$_remaining" ] && _remaining="" || _remaining="''${_remaining#*:}"
        _is_sys=0
        for _p in $NIX_PROFILES; do
          case "$_e" in "$_p"/*) _is_sys=1; break ;; esac
        done
        if [ "$_is_sys" = 1 ]; then
          _system_only="$_system_only''${_system_only:+:}$_e"
        else
          _devshell_only="$_devshell_only''${_devshell_only:+:}$_e"
        fi
      done

      # Reconstruct: devshell → HM → system
      export PATH="$_devshell_only''${_hm_paths:+:$_hm_paths}''${_system_only:+:$_system_only}"
      unset _pre_profile_path _hm_paths _devshell_only _system_only _remaining _e _is_sys _p
    fi

    echo ""
    just --list
    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo "  id - P2P File Sharing CLI Development Environment"
    echo "════════════════════════════════════════════════════════════"
    echo ""
    echo "  Toolchain:"
    echo "    Rust:  $(rustc --version 2>/dev/null || echo 'not found')"
    echo "    Cargo: $(cargo --version 2>/dev/null || echo 'not found')"
    echo "    Bun:   $(bun --version 2>/dev/null || echo 'not found')"
    echo ""
    echo "  Build & Run:"
    echo "    just                 - Kill, build, and serve with web UI (alias: kill-serve)"
    echo "    just build           - Build with web UI [bun]"
    echo "    just build-lib       - Build Rust only (no web/bun)"
    echo "    just build-web       - Build web frontend assets with Bun"
    echo "    just serve           - Build and serve with web UI"
    echo "    just serve-lib       - Serve without web UI"
    echo "    just kill            - Kill any running 'id serve' processes"
    echo ""
    echo "  Testing:"
    echo "    just check           - Fix + CI (run before committing)"
    echo "    just ci              - CI-safe read-only checks"
    echo "    just test            - All fast tests (Rust + TS unit + typecheck)"
    echo "    just test-rust       - All Rust tests (unit + integration)"
    echo "    just test-unit       - Unit tests only (fast)"
    echo "    just test-web        - All web tests (Rust + TS unit + typecheck)"
    echo "    just test-e2e        - Playwright E2E (chromium + firefox)"
    echo "    just test-nix        - All 27 sandboxed checks (nix flake check)"
    echo ""
    echo "  Nix (every just recipe is also: nix run .#<recipe>):"
    echo "    just build-nix       - Build nix package (nix build)"
    echo "    just build-nix-lib   - Build lib-only package (nix build .#id-lib)"
    echo "    just test-nixos      - All 4 NixOS VM tests"
    echo "    just check-one NAME  - Run single nix check by name"
    echo ""
    echo "  Utility:"
    echo "    just list            - Show all available recipes"
    echo "════════════════════════════════════════════════════════════"
    echo ""
    echo "Welcome to the id development shell!"
    echo ""
  '';
}
