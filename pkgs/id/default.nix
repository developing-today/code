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
# Dependency compilation is split from the package build when a
# craneLib is supplied (pkgs/id/flake.nix passes one, built from the
# crane rev the repo's root lock already carries): crane's
# buildDepsOnly compiles every registry/git dependency as its OWN
# derivation keyed only on the manifests, the lockfile, the crate
# tree's manifests, and the vendored path deps, so a change to
# workspace sources rebuilds just the workspace crates instead of
# the whole dependency tree (code#8392, #8574). Callers that do not
# pass craneLib keep the monolithic buildRustPackage build below —
# same sources, same checks, no split (the pkgs/fleet and
# pkgs/atlas-search pattern).

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

  meta = with pkgs.lib; {
    description = "A peer-to-peer file sharing CLI built with Iroh";
    homepage = "https://github.com/developing-today/code";
    license = with licenses; [
      mit
      asl20
    ];
    maintainers = [ ];
  };

  buildFeatures = [
    "sandbox"
    "world"
  ];

in
{
  # Main package build
  default =
    if craneLib != null then
      let
        # Two source tiers. The dependency layer sees only what can
        # change dependency compilation: the workspace manifests and
        # lockfile, the crate tree (crane's dummy-source pass reads
        # the member manifests and build scripts from it), the root
        # build.rs, the cargo config, and vendor/ — p2panda-sync /
        # p2panda-encryption are path dependencies, so they compile
        # AS dependencies and belong to this tier. The package layer
        # is the full tree, exactly what the monolithic build saw:
        # real sources additionally read apps/ (id-cli embeds the
        # tic-tac-toe wasm via include_bytes!), tests, and
        # test-vectors, none of which may key the dependency layer.
        depsSrc = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./.cargo/config.toml
            ./build.rs
            ./crates
            ./vendor
          ];
        };

        commonArgs = {
          inherit pname version;
          src = ./.;
          strictDeps = true;

          # buildRustPackage's buildFeatures, in crane's spelling.
          # (The workspace default is already sandbox+world; the
          # explicit flag keeps the two builds from drifting.)
          cargoExtraArgs = "--features ${pkgs.lib.concatStringsSep "," buildFeatures}";

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs; [
            openssl
          ];
        };

        # The dependency layer: every non-workspace crate compiled
        # once, keyed on the deps tier above only. A workspace-source
        # change leaves this derivation — and its store path —
        # untouched.
        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { src = depsSrc; });
      in
      craneLib.buildPackage (
        commonArgs
        // {
          inherit cargoArtifacts meta;

          # Run tests during build, as the monolithic build does.
          doCheck = true;

          # Compile and run the test phase under the ci-check
          # profile (#3195) instead of release: same opt-level, no
          # LTO / codegen-units=1 / strip costs for test binaries.
          # The release binary build is unchanged. (buildRustPackage
          # spells this `checkType`; crane takes it as a test flag.)
          cargoTestExtraArgs = "--profile ci-check";

          # Additional check commands (run after cargo test), as in
          # the monolithic build. build.rs is exercised here in its
          # real form: with the web feature off (sandbox,world) it
          # returns before touching web/dist, which is why the deps
          # tier needs the script but not the web tree.
          postCheck = ''
            echo "Running additional checks..."
            cargo fmt -- --check
            cargo clippy --all-targets --all-features -- -D warnings
          '';
        }
      )
    else
      pkgs.rustPlatform.buildRustPackage {
        inherit pname version;
        src = ./.;

        inherit buildFeatures;

        cargoLock = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "p2panda-core-0.7.1" = "sha256-yq+ySfo14ucU1BWOUdJM0n7/FL3+Ria3Z+MxMHFa4Ls=";
          };
        };

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        buildInputs = with pkgs; [
          openssl
        ];

        # Run tests during build
        doCheck = true;

        # Compile and run the test phase under the ci-check profile (#3195)
        # instead of release: same opt-level, no LTO / codegen-units=1 /
        # strip costs for test binaries. The release binary build
        # (cargoBuildType) is unchanged.
        checkType = "ci-check";

        # Additional check commands (run after cargoTest)
        postCheck = ''
          echo "Running additional checks..."
          cargo fmt -- --check
          cargo clippy --all-targets --all-features -- -D warnings
        '';

        inherit meta;
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
