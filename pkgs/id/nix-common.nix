# Shared Nix configuration for shell.nix and flake.nix
#
# This file ensures both environments have identical packages, environment
# variables, and shell hooks. The flake.lock provides exact version pinning.
#
# Usage in flake.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };
#
# Usage in shell.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };

{ pkgs }:

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
    bash
    just
    gnused
    findutils
  ]);

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

  TREEFMT_TREE_ROOT_FILE = "treefmt.toml";

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
    echo "  Quick commands:"
    echo "    just                 - List all available tasks"
    echo "    just check           - Run fix + ci (primary check)"
    echo "    just ci              - Run read-only checks (CI-safe)"
    echo "    just build           - Build with web UI [bun]"
    echo "    just build-lib       - Build Rust only (no web/bun)"
    echo "    just serve           - Build and serve with web UI"
    echo "    just serve-lib       - Serve without web UI"
    echo ""
    echo "  Web development:"
    echo "    just web-build       - Build web assets with Bun"
    echo "    just web-dev         - Start web dev server with hot reload"
    echo "    just serve-web 3000  - Serve with web UI on port 3000"
    echo ""
    echo "  Testing & Quality:"
    echo "    just test            - Run all tests"
    echo "    just test-lib        - Run unit tests only (fast)"
    echo "    just test-e2e        - Run Playwright E2E tests"
    echo "    just lint            - Run clippy linting"
    echo "    just coverage        - Generate coverage report"
    echo ""
    echo "  Nix commands:"
    echo "    nix run .#<cmd>      - Run any just command via Nix"
    echo "    nix run .#just <cmd> - Run just (fallback for missing apps)"
    echo "    nix fmt              - Run formatter (just fix)"
    echo "    nix flake check -L   - Run all Nix checks"
    echo "    nix build            - Build web-enabled package (default)"
    echo "    nix build .#id-lib   - Build library-only package"
    echo "════════════════════════════════════════════════════════════"
    echo ""
    echo "Welcome to the id development shell!"
    echo ""
  '';
}
