# default.nix - Build and check the id project with Nix.
#
# Usage:
#   nix-build                    # Build the project
#   nix-build -A check           # Run all checks (fmt, clippy, test)
#   nix-shell                    # Enter development environment (see shell.nix)
#
# This derivation builds the release binary and runs the full test suite
# during the check phase.

{
  pkgs ? import <nixpkgs> { },
}:

let
  # Read Cargo.toml for package metadata
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

  pname = cargoToml.package.name;
  version = cargoToml.package.version;

in
{
  # Main package build
  default = pkgs.rustPlatform.buildRustPackage {
    inherit pname version;
    src = ./.;

    cargoLock = {
      lockFile = ./Cargo.lock;
      # If you have git dependencies, add them here:
      # outputHashes = {
      #   "distributed-topic-tracker-0.1.0" = "sha256-...";
      # };
    };

    nativeBuildInputs = with pkgs; [
      pkg-config
    ];

    buildInputs = with pkgs; [
      openssl
    ];

    # Run tests during build
    doCheck = true;

    # Additional check commands (run after cargoTest)
    postCheck = ''
      echo "Running additional checks..."
      cargo fmt -- --check
      cargo clippy --all-targets --all-features -- -D warnings
    '';

    meta = with pkgs.lib; {
      description = "A peer-to-peer file sharing CLI built with Iroh";
      homepage = "https://github.com/example/id";
      license = with licenses; [
        mit
        asl20
      ];
      maintainers = [ ];
    };
  };

  # Standalone check derivation - runs all quality checks
  check = pkgs.stdenv.mkDerivation {
    name = "${pname}-check-${version}";
    src = ./.;

    nativeBuildInputs = with pkgs; [
      rustup
      pkg-config
      openssl
    ];

    buildInputs = with pkgs; [
      openssl
    ];

    # Environment
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

    buildPhase = ''
      export HOME=$(mktemp -d)
      export CARGO_HOME=$HOME/.cargo
      export RUSTUP_HOME=$HOME/.rustup

      # Install Rust toolchain
      rustup default 1.89.0
      rustup component add clippy rustfmt

      echo "══════════════════════════════════════════════════════"
      echo "  Running all checks for ${pname} v${version}"
      echo "══════════════════════════════════════════════════════"

      echo "→ Checking formatting..."
      cargo fmt -- --check

      echo "→ Running clippy..."
      cargo clippy --all-targets --all-features -- -D warnings

      echo "→ Running unit tests..."
      cargo test --lib

      echo "→ Running integration tests..."
      cargo test --test cli_integration

      echo "→ Running doc tests..."
      cargo test --doc

      echo "→ Building documentation..."
      cargo doc --no-deps

      echo "══════════════════════════════════════════════════════"
      echo "  ✓ All checks passed!"
      echo "══════════════════════════════════════════════════════"
    '';

    installPhase = ''
      mkdir -p $out
      echo "All checks passed" > $out/check-results.txt
      echo "Checked at: $(date)" >> $out/check-results.txt
    '';
  };

  # Development shell (re-export from shell.nix)
  shell = import ./shell.nix { inherit pkgs; };
}
