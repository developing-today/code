{
  description = "id - A peer-to-peer file sharing CLI built with Iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Rust toolchain from rust-toolchain.toml
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Import shared configuration
        nixCommon = import ./nix-common.nix { inherit pkgs; };

        # Inherit from shared config
        inherit (nixCommon) buildInputs opensslEnv;
        nativeBuildInputs = [ rustToolchain ] ++ nixCommon.nativeBuildInputs;

        # Helper to create a check that runs a just command
        mkCheck =
          name: justCmd:
          pkgs.stdenv.mkDerivation {
            name = "id-${name}";
            src = ./.;
            inherit buildInputs nativeBuildInputs;
            OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
            OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
            OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;
            PKG_CONFIG_PATH = opensslEnv.PKG_CONFIG_PATH;
            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo
              just ${justCmd}
            '';
            installPhase = ''
              mkdir -p $out
              echo "${name} passed at $(date)" > $out/result.txt
            '';
          };

        # Helper to create a script that runs in the project directory
        mkScript =
          name: script:
          pkgs.writeShellScriptBin name ''
            cd ${self}
            export OPENSSL_DIR="${opensslEnv.OPENSSL_DIR}"
            export OPENSSL_LIB_DIR="${opensslEnv.OPENSSL_LIB_DIR}"
            export OPENSSL_INCLUDE_DIR="${opensslEnv.OPENSSL_INCLUDE_DIR}"
            export PKG_CONFIG_PATH="${opensslEnv.PKG_CONFIG_PATH}"
            ${script}
          '';

        # Helper to create a runnable app
        mkApp = drv: {
          type = "app";
          program = "${drv}/bin/${drv.name}";
        };

      in
      {
        # Development shell: nix develop
        devShells.default = pkgs.mkShell {
          inherit buildInputs nativeBuildInputs;
          inherit (nixCommon) shellHook;

          OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
          OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
          OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;
          PKG_CONFIG_PATH = opensslEnv.PKG_CONFIG_PATH;
        };

        # =======================================================================
        # Formatter: nix fmt
        # Runs 'just fix' to format Rust and web code
        # =======================================================================
        formatter = pkgs.writeShellScriptBin "formatter" ''
          exec ${pkgs.just}/bin/just fix
        '';

        # =======================================================================
        # Checks: nix flake check
        # Uses 'ci' command (read-only, no auto-fix modifications)
        # =======================================================================
        checks = {
          # CI-safe checks (read-only): fmt-check lint test test-web-unit test-web-typecheck doc
          default = mkCheck "ci" "ci";

          # Individual checks
          fmt-check = mkCheck "fmt-check" "fmt-check";
          lint = mkCheck "lint" "lint";
          test = mkCheck "test" "test";
          test-unit = mkCheck "test-unit" "test-unit";
          test-int = mkCheck "test-int" "test-int";
          test-web = mkCheck "test-web" "test-web";
          test-web-unit = mkCheck "test-web-unit" "test-web-unit";
          test-web-typecheck = mkCheck "test-web-typecheck" "test-web-typecheck";
          doc = mkCheck "doc" "doc";
        };

        # =======================================================================
        # Packages: nix build
        # =======================================================================
        packages = {
          # Web-enabled package (primary product)
          id-web = pkgs.rustPlatform.buildRustPackage {
            pname = "id";
            version = "0.1.0";
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "distributed-topic-tracker-0.2.5" = "sha256-9wMDB1PGHuzxWiZeRQpGs1m9mTTcjbB7y3kSJomNjeY=";
              };
            };

            inherit buildInputs;
            nativeBuildInputs = [
              pkgs.pkg-config
              rustToolchain
              pkgs.bun
            ];

            OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
            OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
            OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;

            preBuild = ''
              # Build web assets with Bun
              cd web
              bun install --frozen-lockfile
              bun run build
              cd ..
            '';

            doCheck = true;

            meta = with pkgs.lib; {
              description = "A peer-to-peer file sharing CLI built with Iroh (with web UI)";
              license = with licenses; [
                mit
                asl20
              ];
            };
          };

          # Library-only package (no web UI, no bun required)
          id-lib = pkgs.rustPlatform.buildRustPackage {
            pname = "id-lib";
            version = "0.1.0";
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "distributed-topic-tracker-0.2.5" = "sha256-9wMDB1PGHuzxWiZeRQpGs1m9mTTcjbB7y3kSJomNjeY=";
              };
            };

            # Disable default web feature for lib-only build
            buildNoDefaultFeatures = true;

            inherit buildInputs;
            nativeBuildInputs = [
              pkgs.pkg-config
              rustToolchain
            ];

            OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
            OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
            OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;

            doCheck = true;

            meta = with pkgs.lib; {
              description = "A peer-to-peer file sharing CLI built with Iroh";
              license = with licenses; [
                mit
                asl20
              ];
            };
          };

          # Default = web
          default = self.packages.${system}.id-web;
        };

        # =======================================================================
        # Apps: nix run .#<name>
        # Mirrors all just commands for Nix-native execution
        # =======================================================================
        apps = {
          # Default: run the web-enabled CLI
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/id";
          };

          # Run just with any arguments (fallback for commands not added as apps)
          just = mkApp (
            pkgs.writeShellScriptBin "just-runner" ''
              exec ${pkgs.just}/bin/just "$@"
            ''
          );

          # ─────────────────────────────────────────────────────────────────────
          # Quality checks
          # ─────────────────────────────────────────────────────────────────────

          check = mkApp (mkScript "check" "just check");
          ci = mkApp (mkScript "ci" "just ci");
          fix = mkApp (mkScript "fix" "just fix");
          fmt = mkApp (mkScript "fmt" "just fmt");
          fmt-check = mkApp (mkScript "fmt-check" "just fmt-check");
          lint = mkApp (mkScript "lint" "just lint");
          lint-fix = mkApp (mkScript "lint-fix" "just lint-fix");

          # ─────────────────────────────────────────────────────────────────────
          # Tests
          # ─────────────────────────────────────────────────────────────────────

          test = mkApp (mkScript "test" "just test");
          test-unit = mkApp (mkScript "test-unit" "just test-unit");
          test-int = mkApp (mkScript "test-int" "just test-int");
          test-one = mkApp (mkScript "test-one" ''just test-one "$@"'');
          test-web = mkApp (mkScript "test-web" "just test-web");
          test-web-unit = mkApp (mkScript "test-web-unit" "just test-web-unit");
          test-web-typecheck = mkApp (mkScript "test-web-typecheck" "just test-web-typecheck");
          test-verbose = mkApp (mkScript "test-verbose" "just test-verbose");

          # ─────────────────────────────────────────────────────────────────────
          # Documentation
          # ─────────────────────────────────────────────────────────────────────

          doc = mkApp (mkScript "doc" "just doc");
          doc-open = mkApp (mkScript "doc-open" "just doc-open");

          # ─────────────────────────────────────────────────────────────────────
          # Coverage
          # ─────────────────────────────────────────────────────────────────────

          coverage = mkApp (mkScript "coverage" "just coverage");
          coverage-open = mkApp (mkScript "coverage-open" "just coverage-open");
          coverage-summary = mkApp (mkScript "coverage-summary" "just coverage-summary");

          # ─────────────────────────────────────────────────────────────────────
          # Build commands
          # ─────────────────────────────────────────────────────────────────────

          build = mkApp (mkScript "build" "just build");
          build-lib = mkApp (mkScript "build-lib" "just build-lib");
          build-force = mkApp (mkScript "build-force" "just build-force");
          build-lib-force = mkApp (mkScript "build-lib-force" "just build-lib-force");
          release = mkApp (mkScript "release" "just release");
          release-lib = mkApp (mkScript "release-lib" "just release-lib");
          release-force = mkApp (mkScript "release-force" "just release-force");
          release-lib-force = mkApp (mkScript "release-lib-force" "just release-lib-force");

          # ─────────────────────────────────────────────────────────────────────
          # Web assets
          # ─────────────────────────────────────────────────────────────────────

          web = mkApp (mkScript "web" "just web");
          web-force = mkApp (mkScript "web-force" "just web-force");
          web-dev = mkApp (mkScript "web-dev" "just web-dev");

          # ─────────────────────────────────────────────────────────────────────
          # Run commands
          # ─────────────────────────────────────────────────────────────────────

          run = mkApp (mkScript "run" ''just run "$@"'');
          repl = mkApp (mkScript "repl" "just repl");

          # ─────────────────────────────────────────────────────────────────────
          # Serve commands
          # ─────────────────────────────────────────────────────────────────────

          serve = mkApp (mkScript "serve" ''just serve "$@"'');
          serve-web = mkApp (mkScript "serve-web" ''just serve-web "$@"'');
          serve-lib = mkApp (mkScript "serve-lib" ''just serve-lib "$@"'');
          build-serve = mkApp (mkScript "build-serve" ''just build-serve "$@"'');
          kill = mkApp (mkScript "kill" "just kill");
          kill-serve = mkApp (mkScript "kill-serve" ''just kill-serve "$@"'');

          # ─────────────────────────────────────────────────────────────────────
          # Combined commands
          # ─────────────────────────────────────────────────────────────────────

          build-check = mkApp (mkScript "build-check" "just build-check");
          build-check-serve = mkApp (mkScript "build-check-serve" ''just build-check-serve "$@"'');
          build-check-serve-lib = mkApp (mkScript "build-check-serve-lib" "just build-check-serve-lib");
          build-serve-lib = mkApp (mkScript "build-serve-lib" "just build-serve-lib");

          # ─────────────────────────────────────────────────────────────────────
          # Watch commands
          # ─────────────────────────────────────────────────────────────────────

          watch = mkApp (mkScript "watch" "just watch");
          watch-test = mkApp (mkScript "watch-test" "just watch-test");
          watch-lint = mkApp (mkScript "watch-lint" "just watch-lint");

          # ─────────────────────────────────────────────────────────────────────
          # Dependency management
          # ─────────────────────────────────────────────────────────────────────

          outdated = mkApp (mkScript "outdated" "just outdated");
          audit = mkApp (mkScript "audit" "just audit");
          machete = mkApp (mkScript "machete" "just machete");
          update = mkApp (mkScript "update" "just update");
          tree = mkApp (mkScript "tree" "just tree");

          # ─────────────────────────────────────────────────────────────────────
          # Utilities
          # ─────────────────────────────────────────────────────────────────────

          clean = mkApp (mkScript "clean" "just clean");
          loc = mkApp (mkScript "loc" "just loc");

          # ─────────────────────────────────────────────────────────────────────
          # Legacy aliases (backwards compatibility)
          # ─────────────────────────────────────────────────────────────────────

          check-all = mkApp (mkScript "check-all" "just check");
          test-lib = mkApp (mkScript "test-lib" "just test-unit");
          build-web = mkApp (mkScript "build-web" "just build");
          build-web-release = mkApp (mkScript "build-web-release" "just build-web-release");
          build-release = mkApp (mkScript "build-release" "just release");
          build-lib-release = mkApp (mkScript "build-lib-release" "just release-lib");
          web-build = mkApp (mkScript "web-build" "just web");
          web-typecheck = mkApp (mkScript "web-typecheck" "just test-web");
          watch-build = mkApp (mkScript "watch-build" "just watch");
        };
      }
    );
}
