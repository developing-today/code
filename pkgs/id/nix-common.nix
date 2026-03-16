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
    echo "    just lint            - Run clippy linting"
    echo "    just coverage        - Generate coverage report"
    echo ""
    echo "  Nix commands:"
    echo "    nix run .#<cmd>      - Run any just command via Nix"
    echo "    nix run .#just <cmd> - Run just (fallback for missing apps)"
    echo "    nix fmt              - Run formatter (just fix)"
    echo "    nix flake check      - Run all Nix checks"
    echo "    nix build            - Build web-enabled package (default)"
    echo "    nix build .#id-lib   - Build library-only package"
    echo "════════════════════════════════════════════════════════════"

    export RUST_BACKTRACE=1
  '';
}
