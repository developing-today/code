{
  description = "id - A peer-to-peer file sharing CLI built with Iroh";

  inputs = {
    # Track nixos-unstable (cached on cache.nixos.org) so the id devshell and
    # the root devshell/system all resolve the same tree.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
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

        # Import shared configuration (includes rustToolchain from rust-overlay)
        nixCommon = import ./nix-common.nix {
          inherit pkgs;
          extraFmtBins = [ bun2nixPkg ];
        };

        # Inherit from shared config
        inherit (nixCommon)
          buildInputs
          opensslEnv
          rustToolchain
          fmtBins
          rocCompiler
          buildWasmHost
          ;
        inherit (nixCommon) nativeBuildInputs;

        # Pre-fetch cargo dependencies for sandbox builds (no network access)
        cargoDeps = pkgs.rustPlatform.importCargoLock {
          lockFile = ./Cargo.lock;
          # Keyed by crate name-version, not rev: a p2panda rev bump keeps the
          # key, so these hashes (and default.nix's) must be updated by hand (#643).
          outputHashes = {
            "p2panda-core-0.7.1" = "sha256-yq+ySfo14ucU1BWOUdJM0n7/FL3+Ria3Z+MxMHFa4Ls=";
          };
        };

        # Pre-fetch bun dependencies for sandbox builds (no network access)
        bun2nixPkg = bun2nix.packages.${system}.default;
        bunDeps = bun2nixPkg.fetchBunDeps {
          bunNix = ./web/bun.nix;
        };
        e2eBunDeps = bun2nixPkg.fetchBunDeps {
          bunNix = ./e2e/bun.nix;
        };

        # Pre-built integration test binary for NixOS VM tests.
        # Compiles `cargo test --test cli_integration --no-run` in the sandbox,
        # producing a standalone test binary that can be executed inside a VM
        # where networking is available (serve_tests need bind/listen).
        integrationTestRunner = pkgs.stdenv.mkDerivation {
          name = "id-integration-test-runner";
          src = ./.;
          inherit buildInputs;
          nativeBuildInputs = nativeBuildInputs ++ [
            bun2nixPkg.hook
            pkgs.rustPlatform.cargoSetupHook
          ];
          inherit cargoDeps;
          inherit (opensslEnv) OPENSSL_DIR;
          inherit (opensslEnv) OPENSSL_LIB_DIR;
          inherit (opensslEnv) OPENSSL_INCLUDE_DIR;
          inherit (opensslEnv) PKG_CONFIG_PATH;

          # bun2nix hook: install web deps offline via pre-fetched cache
          inherit bunDeps;
          bunRoot = "web";
          bunInstallFlags = [ "--linker=hoisted" ];
          dontUseBunBuild = true;
          dontUseBunCheck = true;
          dontUseBunInstall = true;
          dontUseCmakeConfigure = true;

          buildPhase = ''
            export HOME=$(mktemp -d)
            export CARGO_HOME=$HOME/.cargo

            # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
            export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

            # Build web assets (bun2nix hook already installed node_modules via bunNodeModulesInstallPhase)
            (cd web && bun run build)

            # Build the integration test binary (--no-run: compile only, don't execute)
            # --all-features includes web feature for serve web port tests
            cargo test --all-features --test cli_integration --no-run 2>&1
          '';
          installPhase = ''
            mkdir -p $out/bin $out/share/id-test
            # The test binary is in target/debug/deps/cli_integration-<hash>
            TEST_BIN=$(find target/debug/deps -name 'cli_integration-*' -executable -type f | head -1)
            if [ -z "$TEST_BIN" ]; then
              echo "ERROR: Could not find cli_integration test binary"
              exit 1
            fi
            cp "$TEST_BIN" $out/bin/cli_integration_test
            cp -r examples $out/share/id-test/examples
          '';
        };

        # Pre-built e2e test directory with all dependencies installed.
        # Used by the NixOS VM Playwright test (nixos-playwright-e2e) where
        # the test runner needs to be a self-contained nix store path that
        # gets copied into a writable directory inside the VM.
        e2eTestRunner = pkgs.stdenv.mkDerivation {
          name = "id-e2e-runner";
          src = ./e2e;
          nativeBuildInputs = [ pkgs.bun ];
          buildPhase = ''
            E2E_CACHE_DIR=$(mktemp -d)
            cp -r "${e2eBunDeps}"/share/bun-cache/. "$E2E_CACHE_DIR"
            BUN_INSTALL_CACHE_DIR="$E2E_CACHE_DIR" \
              bun install --frozen-lockfile --linker=hoisted
          '';
          installPhase = ''
            mkdir -p $out
            cp -r . $out/
          '';
        };

        # Helper to create a check that runs a just command
        mkCheck =
          name: justCmd:
          pkgs.stdenv.mkDerivation {
            name = "id-${name}";
            # Every check builds its own debug test binaries in parallel on a
            # runner with limited disk. Full debuginfo and the incremental cache
            # filled the disk (ENOSPC, then a linker bus error). Line tables keep
            # file:line in panics and backtraces.
            CARGO_INCREMENTAL = "0";
            CARGO_PROFILE_TEST_DEBUG = "line-tables-only";
            CARGO_PROFILE_DEV_DEBUG = "line-tables-only";
            src = ./.;
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [
              bun2nixPkg.hook
              pkgs.rustPlatform.cargoSetupHook
            ];
            inherit cargoDeps;
            inherit (opensslEnv) OPENSSL_DIR;
            inherit (opensslEnv) OPENSSL_LIB_DIR;
            inherit (opensslEnv) OPENSSL_INCLUDE_DIR;
            inherit (opensslEnv) PKG_CONFIG_PATH;

            # bun2nix hook: install web deps offline via pre-fetched cache
            inherit bunDeps;
            bunRoot = "web";
            bunInstallFlags = [ "--linker=hoisted" ];
            dontUseBunBuild = true;
            dontUseBunCheck = true;
            dontUseBunInstall = true;
            dontUseCmakeConfigure = true;

            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo

              # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

              # Build web assets (bun2nix hook already installed node_modules via bunNodeModulesInstallPhase)
              (cd web && bun run build)

              ${buildWasmHost}

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
        mkApp =
          drv:
          {
            description ? drv.name,
          }:
          {
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
            description = recipe.doc or name;
          in
          mkApp (mkScript name script) { inherit description; };

        # Build a nix app from a just alias (uses target recipe's doc and params)
        mkAliasApp =
          name: alias:
          let
            targetRecipe = justRecipes.recipes.${alias.target};
          in
          mkRecipeApp name targetRecipe;

        # Filter: exclude private recipes and 'default' (handled separately)
        publicRecipes = pkgs.lib.filterAttrs (
          name: recipe: !(recipe.private or false) && name != "default"
        ) justRecipes.recipes;

      in
      {
        # Development shell: nix develop
        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          inherit nativeBuildInputs;
          inherit (nixCommon) shellHook TREEFMT_TREE_ROOT_CMD;

          inherit (opensslEnv) OPENSSL_DIR;
          inherit (opensslEnv) OPENSSL_LIB_DIR;
          inherit (opensslEnv) OPENSSL_INCLUDE_DIR;
          inherit (opensslEnv) PKG_CONFIG_PATH;
        };

        # =======================================================================
        # Formatter: nix fmt
        # Uses treefmt to orchestrate rustfmt + biome + prettier + nixfmt + statix + shfmt + taplo
        # =======================================================================
        formatter = pkgs.writeShellScriptBin "formatter" ''
          export PATH="${pkgs.lib.makeBinPath fmtBins}:$PATH"
          # fix = strip-whitespace + lockfiles + fmt (treefmt with --config-file/--tree-root)
          just fix "$@"
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
          # PR-gate shards of the integration suite (#869): the four
          # shards together run every target test-sandbox runs, in
          # parallel jobs instead of one serial ~25-minute check.
          # (3 → 4 shards in #551: the 3-way shards had drifted to
          # ~12.5 min, close to the fast-lane exit.)
          test-shard-0 = mkCheck "test-shard-0" "test-shard-sandbox 0 4";
          test-shard-1 = mkCheck "test-shard-1" "test-shard-sandbox 1 4";
          test-shard-2 = mkCheck "test-shard-2" "test-shard-sandbox 2 4";
          test-shard-3 = mkCheck "test-shard-3" "test-shard-sandbox 3 4";
          test-web = mkCheck "test-web" "test-web-sandbox";
          test-web-unit = mkCheck "test-web-unit" "test-web-unit";
          test-web-typecheck = mkCheck "test-web-typecheck" "test-web-typecheck";
          doc = mkCheck "doc" "doc";
          cargo-check = mkCheck "cargo-check" "cargo-check";
          crash-gate = mkCheck "crash-gate" "crash-gate";
          # PR-gate splits of crash-gate (#551): the aggregate above runs
          # every heavy full sweep serially (~15 min, the slowest PR
          # check). These four checks partition the same tests by name
          # filter and run in parallel on PRs; crash-gate itself stays
          # for local use and the nightly full flake check, and is
          # excluded from the PR list in check-id-pr-checks.sh.
          crash-gate-sampled = mkCheck "crash-gate-sampled" "crash-gate-sampled";
          crash-gate-sweep = mkCheck "crash-gate-sweep" "crash-gate-sweep";
          crash-gate-control = mkCheck "crash-gate-control" "crash-gate-control";
          crash-gate-ingest = mkCheck "crash-gate-ingest" "crash-gate-ingest";

          # Playwright E2E tests (requires building the binary + browser binaries)
          test-e2e = pkgs.stdenv.mkDerivation {
            name = "id-test-e2e";
            src = ./.;
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [
              bun2nixPkg.hook
              pkgs.rustPlatform.cargoSetupHook
              # TODO: Switch back to `bunx playwright test` once Bun supports Playwright's
              # ESM config loader (.esm.preflight virtual imports). Bun's runtime doesn't handle
              # the Node.js-specific ESM hooks that Playwright uses for TypeScript config loading.
              # Tracking: https://github.com/oven-sh/bun/pull/28610
              pkgs.nodejs
            ];
            inherit cargoDeps;
            inherit (opensslEnv) OPENSSL_DIR;
            inherit (opensslEnv) OPENSSL_LIB_DIR;
            inherit (opensslEnv) OPENSSL_INCLUDE_DIR;
            inherit (opensslEnv) PKG_CONFIG_PATH;

            # bun2nix hook: install web deps offline via pre-fetched cache
            inherit bunDeps;
            bunRoot = "web";
            bunInstallFlags = [ "--linker=hoisted" ];
            dontUseBunBuild = true;
            dontUseBunCheck = true;
            dontUseBunInstall = true;
            dontUseCmakeConfigure = true;

            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo

              # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

              # Build web assets (bun2nix hook already installed node_modules via bunNodeModulesInstallPhase)
              (cd web && bun run build)

              # Build the binary with web feature
              cargo build --features web

              # Install e2e deps from pre-fetched cache (separate from web deps)
              E2E_CACHE_DIR=$(mktemp -d)
              cp -r "${e2eBunDeps}"/share/bun-cache/. "$E2E_CACHE_DIR"
              (cd e2e && BUN_INSTALL_CACHE_DIR="$E2E_CACHE_DIR" \
                bun install --frozen-lockfile --linker=hoisted)

              # Configure Playwright to use nix-provided browsers
              export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
              export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers}"

              # Unset LD_LIBRARY_PATH before running Playwright — nix-patched Chromium
              # has its own RPATH and the extra libstdc++ path (needed for @parcel/watcher
              # during web build above) can interfere with Chromium's library resolution,
              # causing it to hang on launch in the nix sandbox.
              unset LD_LIBRARY_PATH

              # Run Playwright E2E tests — one invocation per spec file (#229).
              # playwright.config.ts starts ONE ephemeral server per browser
              # project for the whole invocation, so a single full-suite run
              # lets every spec observe the files/state earlier specs created
              # on that shared server (the Firefox shared-state failure
              # cluster: e.g. basic.spec's "empty state" test ran after
              # autosave.spec had already created files). A fresh invocation
              # per spec file gives each file a fresh ephemeral server.
              # All files run even if one fails, so one run reports every
              # failing file; the check fails if any file failed.
              # TODO: Switch to `bunx playwright test` once Bun's ESM loader supports
              # Playwright's .esm.preflight virtual imports for TypeScript config loading.
              # Tracking: https://github.com/oven-sh/bun/pull/28610
              (
                cd e2e
                e2e_status=0
                for spec in tests/*.spec.ts; do
                  echo "=== test-e2e isolated per-spec run: $spec ==="
                  node node_modules/@playwright/test/cli.js test "$spec" || e2e_status=1
                done
                exit $e2e_status
              )
            '';
            installPhase = ''
              mkdir -p $out
              echo "test-e2e passed at $(date)" > $out/result.txt
            '';
          };

          nix-fmt-check = pkgs.stdenv.mkDerivation {
            name = "id-nix-fmt-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.nixfmt ];
            buildPhase = ''
              find . -name '*.nix' -not -path './web/bun.nix' -not -path './e2e/bun.nix' | xargs nixfmt --check
            '';
            installPhase = ''
              mkdir -p $out
              echo "nix-fmt-check passed at $(date)" > $out/result.txt
            '';
          };
          treefmt-check = pkgs.stdenv.mkDerivation {
            name = "id-treefmt-check";
            src = ./.;
            nativeBuildInputs = fmtBins;
            buildPhase = ''
              treefmt --config-file ./treefmt.toml --tree-root "$(pwd)" --ci 2>&1
            '';
            installPhase = ''
              mkdir -p $out
              echo "treefmt-check passed at $(date)" > $out/result.txt
            '';
          };

          # Per-formatter checks (read-only validation)
          biome-check = pkgs.stdenv.mkDerivation {
            name = "id-biome-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.biome ];
            buildPhase = ''
              biome format \
                --files-ignore-unknown=true \
                $(find . \( -name '*.js' -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' \
                  -o -name '*.css' -o -name '*.json' -o -name '*.graphql' \) \
                  -not -path '*/node_modules/*' -not -path '*/target/*' \
                  -not -path '*/dist/*')
            '';
            installPhase = ''
              mkdir -p $out
              echo "biome-check passed at $(date)" > $out/result.txt
            '';
          };
          rustfmt-check = pkgs.stdenv.mkDerivation {
            name = "id-rustfmt-check";
            src = ./.;
            nativeBuildInputs = [ rustToolchain ];
            buildPhase = ''
              find . -name '*.rs' -not -path '*/target/*' \
                -exec rustfmt --check --edition 2024 {} +
            '';
            installPhase = ''
              mkdir -p $out
              echo "rustfmt-check passed at $(date)" > $out/result.txt
            '';
          };
          statix-check = pkgs.stdenv.mkDerivation {
            name = "id-statix-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.statix ];
            buildPhase = ''
              # Collect failures across files instead of swallowing each
              # one: a pipe into `while` would run the loop in a subshell
              # and lose the flag, so feed it via process substitution.
              fail=0
              while IFS= read -r -d "" f; do
                statix check -- "$f" || fail=1
              done < <(find . -name '*.nix' -print0)
              if [ "$fail" -ne 0 ]; then
                echo "statix-check: statix reported problems above" >&2
                false
              fi
            '';
            installPhase = ''
              mkdir -p $out
              echo "statix-check passed at $(date)" > $out/result.txt
            '';
          };
          shfmt-check = pkgs.stdenv.mkDerivation {
            name = "id-shfmt-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.shfmt ];
            buildPhase = ''
              find . -name '*.sh' -not -path '*/node_modules/*' \
                -exec shfmt -d -i 2 -s {} +
            '';
            installPhase = ''
              mkdir -p $out
              echo "shfmt-check passed at $(date)" > $out/result.txt
            '';
          };
          shellcheck-check = pkgs.stdenv.mkDerivation {
            name = "id-shellcheck-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.shellcheck ];
            buildPhase = ''
              # Enforced at warning severity (#702): under sandbox
              # conditions shellcheck reports only four info findings
              # (SC2012, `ls` glob picks in the examples' build.sh
              # files) and no warning-or-above findings, so warnings
              # and errors fail the gate while those info findings
              # are worked down separately.
              find . -name '*.sh' -not -path '*/node_modules/*' \
                -exec shellcheck --severity=warning {} +
            '';
            installPhase = ''
              mkdir -p $out
              echo "shellcheck-check passed at $(date)" > $out/result.txt
            '';
          };
          taplo-check = pkgs.stdenv.mkDerivation {
            name = "id-taplo-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.taplo ];
            buildPhase = ''
              find . -name '*.toml' -not -path '*/target/*' \
                -exec taplo check {} +
            '';
            installPhase = ''
              mkdir -p $out
              echo "taplo-check passed at $(date)" > $out/result.txt
            '';
          };
          prettier-check = pkgs.stdenv.mkDerivation {
            name = "id-prettier-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.prettier ]; # nodepackages remove 2026-04-03
            buildPhase = ''
              find . \( -name '*.html' -o -name '*.md' -o -name '*.mdx' \
                -o -name '*.scss' -o -name '*.yaml' \) \
                -not -path '*/node_modules/*' -not -path '*/target/*' \
                -not -path '*/dist/*' \
                -exec prettier --check {} +
            '';
            installPhase = ''
              mkdir -p $out
              echo "prettier-check passed at $(date)" > $out/result.txt
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
            nixos-playwright-e2e = pkgs.testers.runNixOSTest (
              import ./nix/tests/playwright-e2e-test.nix {
                idPackage = self.packages.${system}.id-web;
                inherit e2eTestRunner;
                playwrightBrowsers = pkgs.playwright-driver.browsers;
              }
            );
            nixos-integration = pkgs.testers.runNixOSTest (
              import ./nix/tests/integration-test.nix {
                idPackage = self.packages.${system}.id-web;
                inherit integrationTestRunner;
              }
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

            # Enable the web feature (default features are empty in Cargo.toml)
            buildFeatures = [
              "web"
              "sandbox"
              "world"
            ];

            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "p2panda-core-0.7.1" = "sha256-yq+ySfo14ucU1BWOUdJM0n7/FL3+Ria3Z+MxMHFa4Ls=";
              };
            };

            inherit buildInputs;
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.cmake
              rustToolchain
              pkgs.bun
              bun2nixPkg.hook
              # The tests compile roc worlds. zig is not listed: its setup hook would
              # replace this derivation's build phase (buildWasmHost finds it by path).
              rocCompiler
            ];

            # bun2nix: offline web dependency installation
            inherit bunDeps;
            bunRoot = "web";
            bunInstallFlags = [ "--linker=hoisted" ];
            dontUseBunBuild = true;
            dontUseBunCheck = true;
            dontUseBunInstall = true;
            dontUseCmakeConfigure = true;

            inherit (opensslEnv) OPENSSL_DIR;
            inherit (opensslEnv) OPENSSL_LIB_DIR;
            inherit (opensslEnv) OPENSSL_INCLUDE_DIR;

            preBuild = ''
              # Build web assets (bun2nix hook already installed node_modules)
              # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              cd web
              bun run build
              cd ..

              ${buildWasmHost}
            '';

            doCheck = true;
            # serve_tests require networking (bind/listen), unavailable in nix sandbox
            checkFlags = [
              "--skip"
              "serve_tests"
            ];

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
                "p2panda-core-0.7.1" = "sha256-yq+ySfo14ucU1BWOUdJM0n7/FL3+Ria3Z+MxMHFa4Ls=";
              };
            };

            # Disable default web feature for lib-only build
            buildNoDefaultFeatures = true;
            buildFeatures = [
              "sandbox"
              "world"
            ];
            dontUseCmakeConfigure = true;

            inherit buildInputs;
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.cmake
              rustToolchain
              rocCompiler
            ];

            inherit (opensslEnv) OPENSSL_DIR;
            inherit (opensslEnv) OPENSSL_LIB_DIR;
            inherit (opensslEnv) OPENSSL_INCLUDE_DIR;

            preBuild = buildWasmHost;

            doCheck = true;
            # serve_tests require networking (bind/listen), unavailable in nix sandbox
            checkFlags = [
              "--skip"
              "serve_tests"
            ];

            meta = commonMeta // {
              description = "A peer-to-peer file sharing CLI built with Iroh";
            };
          };

          # Default = web
          default = self.packages.${system}.id-web;
        };

        # =======================================================================
        # Apps: nix run .#<name>
        # Dynamically generated from just-recipes.json (recipes + aliases).
        # Only 'default' and 'just' are manually defined.
        # =======================================================================
        apps =
          pkgs.lib.mapAttrs mkRecipeApp publicRecipes
          // pkgs.lib.mapAttrs mkAliasApp (justRecipes.aliases or { })
          // {
            # Default: run the web-enabled CLI binary
            default = {
              type = "app";
              program = "${self.packages.${system}.default}/bin/id";
              meta = commonMeta // {
                description = "Run the id peer-to-peer file sharing CLI";
              };
            };

            # Run just with any arguments (fallback for commands not added as apps)
            just = mkApp (pkgs.writeShellScriptBin "just-runner" ''
              exec ${pkgs.just}/bin/just "$@"
            '') { description = "Run just with any arguments"; };
          };
      }
    );
}
