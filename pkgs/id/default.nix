# default.nix - Build and check the id project with Nix.
#
# Usage:
#   nix-build                    # Build the project
#   nix-build -A check           # Run all checks (fmt, clippy, test)
#   nix-shell                    # Enter development environment (see shell.nix)
#
# This derivation builds the release binary and runs the full test suite
# during the check phase.
#
# Dependency compilation is split from the package build with crane
# (code#8392, following atlas-search #8560 and fleet #8572): crane's
# buildDepsOnly compiles every registry dependency as its OWN
# derivation keyed only on the manifests + Cargo.lock (crane stubs the
# workspace crates' sources in that tier), so a source-only change
# rebuilds just the package derivation instead of the whole dependency
# tree. Callers may pass their own craneLib; standalone nix-build use
# constructs one from the pinned crane rev the repo's flakes already
# carry. The package tier keeps the monolith's exact behaviour —
# same features, same test suite, same postCheck fmt/clippy — except
# that the tests run under the release profile (crane reuses the
# dependency artifacts for the check phase) instead of the ci-check
# profile the monolith's checkType selected.

{
  pkgs ? import <nixpkgs> { },
  craneLib ? null,
}:

let
  # Read Cargo.toml for package metadata
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

  # The Rust version the check installs, from the pin in rust-toolchain.toml
  toolchainToml = builtins.fromTOML (builtins.readFile ./rust-toolchain.toml);
  rustVersion = toolchainToml.toolchain.channel;

  pname = cargoToml.package.name;
  inherit (cargoToml.package) version;

  # Crane, pinned to the rev the code repo's root lock already carries
  # (the same rev atlas-search and fleet use).
  craneLibEffective =
    if craneLib != null then
      craneLib
    else
      import (pkgs.fetchFromGitHub {
        owner = "ipetkov";
        repo = "crane";
        rev = "01bc1d404a51a0a07e9d8759cd50a7903e218c82";
        hash = "sha256-8qxEFpj4dVmIuPn9j9z6NTbU+hrcGjBOvaxTzre5HmM=";
      }) { inherit pkgs; };

  # Crane takes the lockfile as a path and the git-dependency hashes
  # as a separate outputHashes argument (where buildRustPackage nested
  # them under cargoLock), keyed by the lockfile's full source string
  # instead of crate name-version — same checkout, same hash.
  cargoLockPath = ./Cargo.lock;
  outputHashesAttrs = {
    "git+https://github.com/dezren39/p2panda?rev=4a8a7ea4b6e0da187108fba9a0085e0f9679aded#4a8a7ea4b6e0da187108fba9a0085e0f9679aded" =
      "sha256-9QbbXbjFMwsGKCYV6BIPOnOqLegGFexDvoph79yNpOQ=";
  };

  metaAttrs = with pkgs.lib; {
    description = "A peer-to-peer file sharing CLI built with Iroh";
    homepage = "https://github.com/developing-today/code";
    license = with licenses; [
      mit
      asl20
    ];
    maintainers = [ ];
  };

  # Two source tiers. The dependency tier carries only what can change
  # dependency compilation: the workspace manifests and lockfile, the
  # crate sources (crane stubs the workspace members' Rust targets in
  # this tier, so a source edit leaves the tier's output untouched),
  # the vendored path dependencies (compiled for real as dependencies),
  # the package-root build.rs (stubbed by crane here; its real run —
  # the web/dist asset check — happens in the package tier, whose
  # source is the full tree), and .cargo/config.toml.
  depsSrc = pkgs.lib.fileset.toSource {
    root = ./.;
    fileset = pkgs.lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./crates
      ./src
      ./vendor
      ./build.rs
      ./.cargo
    ];
  };

  commonArgs = {
    inherit pname version;
    strictDeps = true;
    cargoLock = cargoLockPath;
    outputHashes = outputHashesAttrs;
    cargoExtraArgs = "--locked --features sandbox,world";

    nativeBuildInputs = with pkgs; [
      pkg-config
    ];

    buildInputs = with pkgs; [
      openssl
    ];
  };

  # The dependency layer: every registry + vendored dependency compiled
  # once, keyed on the deps tier above only.
  cargoArtifacts = craneLibEffective.buildDepsOnly (commonArgs // { src = depsSrc; });

in
{
  # Main package build
  default = craneLibEffective.buildPackage (
    commonArgs
    // {
      # The package tier sees the full tree, exactly what the
      # buildRustPackage build saw (tests read test-data/, the root
      # build.rs checks web/dist, and so on).
      src = ./.;
      inherit cargoArtifacts;

      # Run tests during build (the test suite runs under the release
      # profile here — see the header note).
      doCheck = true;

      # Additional check commands (run after cargo test)
      postCheck = ''
        echo "Running additional checks..."
        cargo fmt -- --check
        cargo clippy --all-targets --all-features -- -D warnings
      '';

      meta = metaAttrs;
    }
  );

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
      rustup default ${rustVersion}
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

      echo "→ Checking the toolchain pin..."
      cargo test --test toolchain_pin

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
