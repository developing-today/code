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

{
  # Build inputs (libraries)
  buildInputs = with pkgs; [ openssl ];

  # Native build inputs (tools, compilers)
  # Note: rustToolchain should be added separately as it's defined differently
  # in flake.nix vs shell.nix
  nativeBuildInputs = with pkgs; [
    # Build dependencies
    pkg-config

    # Cargo plugins
    cargo-watch
    cargo-nextest
    cargo-llvm-cov
    cargo-audit
    cargo-outdated
    cargo-machete
    cargo-edit

    # Development tools
    just
    git
    ripgrep
    fd
    jq
    tokei
    hyperfine

    # Web development tools
    bun # JavaScript bundler and runtime (required for web builds)
    nodePackages.typescript # TypeScript for type checking

    # E2E testing (Playwright)
    chromium # Browser for Playwright E2E tests
    firefox # Browser for Playwright E2E tests
  ];

  # OpenSSL environment variables
  opensslEnv = {
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
  };

  # Shared shell hook for both nix-shell and nix develop
  shellHook = ''
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

    export RUST_BACKTRACE=1

    # Playwright E2E testing: use Nix-provided browsers
    export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
    export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH="${pkgs.chromium}/bin/chromium"
    export PLAYWRIGHT_FIREFOX_EXECUTABLE_PATH="${pkgs.firefox}/bin/firefox"

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
  '';
}
