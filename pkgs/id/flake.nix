{
  description = "id - A peer-to-peer file sharing CLI built with Iroh";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    # nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems = {
      url = "github:nix-systems/default";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
    };
    import-tree.url = "github:vic/import-tree";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    bun2nix = {
      url = "github:nix-community/bun2nix";
      inputs = {
        flake-parts.follows = "flake-parts";
        import-tree.follows = "import-tree";
        nixpkgs.follows = "nixpkgs";
        systems.follows = "systems";
      };
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      bun2nix,
      # TODO: consider use systems here?
      ...
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

        # Pre-fetch cargo dependencies for sandbox builds (no network access)
        cargoDeps = pkgs.rustPlatform.importCargoLock {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "distributed-topic-tracker-0.2.8" = "sha256-JCRUY9Q2kcAN8x7HWcyIbcw2O9XMJcigoCHIAJwd348=";
          };
        };

        # Pre-fetch bun dependencies for sandbox builds (no network access)
        bun2nixPkg = bun2nix.packages.${system}.default;
        bunDeps = bun2nixPkg.fetchBunDeps {
          bunNix = ./web/bun.nix;
        };

        # Helper to create a check that runs a just command
        mkCheck =
          name: justCmd:
          pkgs.stdenv.mkDerivation {
            name = "id-${name}";
            src = ./.;
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [ bun2nixPkg.hook ];
            OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
            OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
            OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;
            PKG_CONFIG_PATH = opensslEnv.PKG_CONFIG_PATH;

            # bun2nix hook: install web deps offline via pre-fetched cache
            inherit bunDeps;
            bunRoot = "web";
            bunInstallFlags = [ "--linker=hoisted" ];
            dontUseBunBuild = true;
            dontUseBunCheck = true;
            dontUseBunInstall = true;

            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo

              # Configure cargo to use vendored dependencies (nix sandbox has no network)
              cat >> .cargo/config.toml << EOF

              [source.crates-io]
              replace-with = "vendored-sources"

              [source."git+https://github.com/developing-today-forks/distributed-topic-tracker?branch=main"]
              git = "https://github.com/developing-today-forks/distributed-topic-tracker"
              branch = "main"
              replace-with = "vendored-sources"

              [source.vendored-sources]
              directory = "${cargoDeps}"
              EOF

              # Build web assets (bun2nix hook already installed node_modules via bunNodeModulesInstallPhase)
              (cd web && bun run build)

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

        # Helper to create a runnable app with metadata
        mkApp = drv: description: {
          type = "app";
          program = "${drv}/bin/${drv.name}";
          meta = commonMeta // {
            inherit description;
          };
        };

        # Common metadata for all apps and packages
        commonMeta = {
          homepage = "https://github.com/developing-today/code";
          license = with pkgs.lib.licenses; [
            mit
            asl20
          ];
        };

        # ─── Dynamic app generation from justfile ──────────────────────────
        #
        # Apps are generated from just-recipes.json (produced by `just just-recipes`).
        # To add a new nix app: add a recipe to the justfile, run `just lockfiles`,
        # and the app appears automatically as `nix run .#<recipe-name>`.
        #
        justRecipes = builtins.fromJSON (builtins.readFile ./just-recipes.json);

        # Build a nix app from a just recipe
        mkRecipeApp =
          name: recipe:
          let
            hasParams = (builtins.length (recipe.parameters or [ ])) > 0;
            script = if hasParams then ''just ${name} "$@"'' else "just ${name}";
            description = if recipe.doc != null then recipe.doc else "Run 'just ${name}'";
          in
          mkApp (mkScript name script) description;

        # Filter: exclude private recipes and 'default' (handled separately)
        publicRecipes = pkgs.lib.filterAttrs (
          name: recipe: !(recipe.private or false) && name != "default"
        ) justRecipes.recipes;

      in
      {
        # Development shell: nix develop
        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          nativeBuildInputs = nativeBuildInputs ++ [ bun2nixPkg ];
          inherit (nixCommon) shellHook;

          OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
          OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
          OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;
          PKG_CONFIG_PATH = opensslEnv.PKG_CONFIG_PATH;

          # Anchor treefmt to this flake's directory (not the git root)
          # so it works correctly when nested inside a parent repo.
          TREEFMT_TREE_ROOT_FILE = "treefmt.toml";
        };

        # =======================================================================
        # Formatter: nix fmt
        # Uses treefmt to orchestrate rustfmt + biome
        # =======================================================================
        formatter = pkgs.writeShellScriptBin "formatter" ''
          export PATH="${
            pkgs.lib.makeBinPath [
              rustToolchain
              pkgs.biome
              pkgs.just
              pkgs.gnused
              pkgs.findutils
            ]
          }:$PATH"
          # Strip trailing whitespace from all source files (fixes rustfmt errors)
          find . -type f \( -name '*.rs' -o -name '*.nix' -o -name '*.toml' -o -name '*.json' \
            -o -name '*.md' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.jsx' \
            -o -name '*.css' -o -name '*.html' -o -name '*.sh' \) \
            -not -path '*/.git/*' -not -path '*/node_modules/*' -not -path '*/target/*' \
            -not -path '*/dist/*' \
            -exec sed -i 's/[[:space:]]*$//' {} +
          # Regenerate lockfiles and apply fixes
          just fix
          exec ${pkgs.treefmt}/bin/treefmt --tree-root-file treefmt.toml "$@"
        '';

        # =======================================================================
        # Checks: nix flake check -L
        # Uses 'ci' command (read-only, no auto-fix modifications)
        # =======================================================================
        checks = {
          # CI-safe checks (read-only): cargo-fmt-check web-fmt-check clippy-lint web-lint test-sandbox test-web-unit test-web-typecheck doc build release
          # NixOS VM integration tests (Linux only): nixos-serve nixos-e2e
          default = mkCheck "ci" "ci";

          # Individual checks
          cargo-fmt-check = mkCheck "cargo-fmt-check" "cargo-fmt-check";
          web-fmt-check = mkCheck "web-fmt-check" "web-fmt-check";
          clippy-lint = mkCheck "clippy-lint" "clippy-lint";
          web-lint = mkCheck "web-lint" "web-lint";
          test = mkCheck "test" "test-sandbox";
          test-unit = mkCheck "test-unit" "test-unit";
          test-int = mkCheck "test-int" "test-int-sandbox";
          test-web = mkCheck "test-web" "test-web-sandbox";
          test-web-unit = mkCheck "test-web-unit" "test-web-unit";
          test-web-typecheck = mkCheck "test-web-typecheck" "test-web-typecheck";
          doc = mkCheck "doc" "doc";
          cargo-check = mkCheck "cargo-check" "cargo-check";
          nix-fmt-check = pkgs.stdenv.mkDerivation {
            name = "id-nix-fmt-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.nixfmt ];
            buildPhase = ''
              find . -name '*.nix' | xargs nixfmt --check
            '';
            installPhase = ''
              mkdir -p $out
              echo "nix-fmt-check passed at $(date)" > $out/result.txt
            '';
          };
          treefmt-check = pkgs.stdenv.mkDerivation {
            name = "id-treefmt-check";
            src = ./.;
            nativeBuildInputs = [
              rustToolchain
              pkgs.biome
              pkgs.treefmt
            ];
            buildPhase = ''
              treefmt --ci --tree-root-file treefmt.toml --allow-missing-formatter 2>&1 || true
            '';
            installPhase = ''
              mkdir -p $out
              echo "treefmt-check passed at $(date)" > $out/result.txt
            '';
          };
        }
        // (
          # NixOS VM integration tests (Linux only — VMs require KVM)
          pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            nixos-serve = pkgs.testers.runNixOSTest (
              import ./nix/tests/serve-test.nix { idPackage = self.packages.${system}.id-web; }
            );
            nixos-e2e = pkgs.testers.runNixOSTest (
              import ./nix/tests/e2e-test.nix { idPackage = self.packages.${system}.id-web; }
            );
          }
        );

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
                "distributed-topic-tracker-0.2.8" = "sha256-JCRUY9Q2kcAN8x7HWcyIbcw2O9XMJcigoCHIAJwd348=";
              };
            };

            inherit buildInputs;
            nativeBuildInputs = [
              pkgs.pkg-config
              rustToolchain
              pkgs.bun
              bun2nixPkg.hook
            ];

            # bun2nix: offline web dependency installation
            inherit bunDeps;
            bunRoot = "web";
            bunInstallFlags = [ "--linker=hoisted" ];
            dontUseBunBuild = true;
            dontUseBunCheck = true;
            dontUseBunInstall = true;

            OPENSSL_DIR = opensslEnv.OPENSSL_DIR;
            OPENSSL_LIB_DIR = opensslEnv.OPENSSL_LIB_DIR;
            OPENSSL_INCLUDE_DIR = opensslEnv.OPENSSL_INCLUDE_DIR;

            preBuild = ''
              # Build web assets (bun2nix hook already installed node_modules)
              cd web
              bun run build
              cd ..
            '';

            doCheck = true;

            meta = commonMeta // {
              description = "A peer-to-peer file sharing CLI built with Iroh (with web UI)";
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
                "distributed-topic-tracker-0.2.8" = "sha256-JCRUY9Q2kcAN8x7HWcyIbcw2O9XMJcigoCHIAJwd348=";
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

            meta = commonMeta // {
              description = "A peer-to-peer file sharing CLI built with Iroh";
            };
          };

          # Default = web
          default = self.packages.${system}.id-web;
        };

        # =======================================================================
        # Apps: nix run .#<name>
        # Dynamically generated from justfile recipes (see just-recipes.json)
        # =======================================================================
        apps = pkgs.lib.mapAttrs mkRecipeApp publicRecipes // {
          # Default: run the web-enabled CLI binary
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/id";
            meta = commonMeta // {
              description = "Run the id peer-to-peer file sharing CLI";
            };
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
          cargo-fmt = mkApp (mkScript "cargo-fmt" "just cargo-fmt");
          cargo-fmt-check = mkApp (mkScript "cargo-fmt-check" "just cargo-fmt-check");
          lint = mkApp (mkScript "lint" "just lint");
          lint-fix = mkApp (mkScript "lint-fix" "just lint-fix");
          clippy-lint = mkApp (mkScript "clippy-lint" "just clippy-lint");
          clippy-lint-fix = mkApp (mkScript "clippy-lint-fix" "just clippy-lint-fix");
          web-fmt = mkApp (mkScript "web-fmt" "just web-fmt");
          web-fmt-check = mkApp (mkScript "web-fmt-check" "just web-fmt-check");
          web-lint = mkApp (mkScript "web-lint" "just web-lint");
          web-lint-fix = mkApp (mkScript "web-lint-fix" "just web-lint-fix");
          cargo-check = mkApp (mkScript "cargo-check" "just cargo-check");

          # ─────────────────────────────────────────────────────────────────────
          # Lockfiles
          # ─────────────────────────────────────────────────────────────────────

          lockfiles = mkApp (mkScript "lockfiles" "just lockfiles");
          bun2nix = mkApp (mkScript "bun2nix" "just bun2nix");

          # ─────────────────────────────────────────────────────────────────────
          # Tests
          # ─────────────────────────────────────────────────────────────────────

          test = mkApp (mkScript "test" "just test");
          test-sandbox = mkApp (mkScript "test-sandbox" "just test-sandbox");
          test-unit = mkApp (mkScript "test-unit" "just test-unit");
          test-int = mkApp (mkScript "test-int" "just test-int");
          test-int-sandbox = mkApp (mkScript "test-int-sandbox" "just test-int-sandbox");
          test-one = mkApp (mkScript "test-one" ''just test-one "$@"'');
          test-web = mkApp (mkScript "test-web" "just test-web");
          test-web-sandbox = mkApp (mkScript "test-web-sandbox" "just test-web-sandbox");
          test-web-unit = mkApp (mkScript "test-web-unit" "just test-web-unit");
          test-web-typecheck = mkApp (mkScript "test-web-typecheck" "just test-web-typecheck");
          test-verbose = mkApp (mkScript "test-verbose" "just test-verbose");

          # E2E tests (Playwright - requires network, not run in sandbox)
          test-e2e = mkApp (mkScript "test-e2e" "just test-e2e");
          test-e2e-chromium = mkApp (mkScript "test-e2e-chromium" "just test-e2e-chromium");
          test-e2e-firefox = mkApp (mkScript "test-e2e-firefox" "just test-e2e-firefox");
          test-e2e-report = mkApp (mkScript "test-e2e-report" "just test-e2e-report");

          # All tests (requires network for serve_tests + E2E)
          test-all = mkApp (mkScript "test-all" "just test-all");

          # NixOS VM integration tests (Linux only, requires KVM)
          test-nixos = mkApp (mkScript "test-nixos" "just test-nixos");
          test-nixos-serve = mkApp (mkScript "test-nixos-serve" "just test-nixos-serve");
          test-nixos-e2e = mkApp (mkScript "test-nixos-e2e" "just test-nixos-e2e");

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
          build-web-force = mkApp (mkScript "build-web-force" "just build-web-force");
          build-cargo = mkApp (mkScript "build-cargo" "just build-cargo");
          build-web-cargo = mkApp (mkScript "build-web-cargo" "just build-web-cargo");
          build-lib-cargo = mkApp (mkScript "build-lib-cargo" "just build-lib-cargo");
          release = mkApp (mkScript "release" "just release");
          release-lib = mkApp (mkScript "release-lib" "just release-lib");
          release-force = mkApp (mkScript "release-force" "just release-force");
          release-lib-force = mkApp (mkScript "release-lib-force" "just release-lib-force");
          release-web-force = mkApp (mkScript "release-web-force" "just release-web-force");
          release-web-cargo = mkApp (mkScript "release-web-cargo" "just release-web-cargo");
          release-lib-cargo = mkApp (mkScript "release-lib-cargo" "just release-lib-cargo");

          # ─────────────────────────────────────────────────────────────────────
          # Web assets
          # ─────────────────────────────────────────────────────────────────────

          assets = mkApp (mkScript "assets" "just assets");
          web = mkApp (mkScript "web" "just web");
          web-assets = mkApp (mkScript "web-assets" "just web-assets");
          web-force = mkApp (mkScript "web-force" "just web-force");
          web-assets-force = mkApp (mkScript "web-assets-force" "just web-assets-force");
          web-dev = mkApp (mkScript "web-dev" "just web-dev");
          web-assets-dev = mkApp (mkScript "web-assets-dev" "just web-assets-dev");

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
          sleep = mkApp (mkScript "sleep" ''just sleep "$@"'');
          kill-serve = mkApp (mkScript "kill-serve" ''just kill-serve "$@"'');

          # ─────────────────────────────────────────────────────────────────────
          # Combined commands
          # ─────────────────────────────────────────────────────────────────────

          check-serve = mkApp (mkScript "check-serve" ''just check-serve "$@"'');
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
          chown = mkApp (mkScript "chown" "just chown");

          # ─────────────────────────────────────────────────────────────────────
          # Flake input management
          # ─────────────────────────────────────────────────────────────────────

          update-input = mkApp (mkScript "update-input" ''just update-input "$@"'');
          update-inputs-all = mkApp (mkScript "update-inputs-all" "just update-inputs-all");
          update-nixpkgs = mkApp (mkScript "update-nixpkgs" "just update-nixpkgs");
          update-nixpkgs-all = mkApp (mkScript "update-nixpkgs-all" "just update-nixpkgs-all");
          update-nixpkgs-master = mkApp (mkScript "update-nixpkgs-master" "just update-nixpkgs-master");
          update-nixpkgs-unstable = mkApp (mkScript "update-nixpkgs-unstable" "just update-nixpkgs-unstable");
          update-nixpkgs-all-only = mkApp (mkScript "update-nixpkgs-all-only" "just update-nixpkgs-all-only");
          update-nixpkgs-master-only = mkApp (
            mkScript "update-nixpkgs-master-only" "just update-nixpkgs-master-only"
          );
          update-nixpkgs-unstable-only = mkApp (
            mkScript "update-nixpkgs-unstable-only" "just update-nixpkgs-unstable-only"
          );

          # ─────────────────────────────────────────────────────────────────────
          # Legacy aliases (backwards compatibility)
          # ─────────────────────────────────────────────────────────────────────

          check-all = mkApp (mkScript "check-all" "just check");
          check-nix = mkApp (mkScript "check-nix" "just check-nix");
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
