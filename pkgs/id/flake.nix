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
          ;
        inherit (nixCommon) nativeBuildInputs;

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
        e2eBunDeps = bun2nixPkg.fetchBunDeps {
          bunNix = ./e2e/bun.nix;
        };

        # Helper to create a check that runs a just command
        mkCheck =
          name: justCmd:
          pkgs.stdenv.mkDerivation {
            name = "id-${name}";
            src = ./.;
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [ bun2nixPkg.hook ];
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

            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo

              # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

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
          test-web = mkCheck "test-web" "test-web-sandbox";
          test-web-unit = mkCheck "test-web-unit" "test-web-unit";
          test-web-typecheck = mkCheck "test-web-typecheck" "test-web-typecheck";
          doc = mkCheck "doc" "doc";
          cargo-check = mkCheck "cargo-check" "cargo-check";

          # Playwright E2E tests (requires building the binary + browser binaries)
          test-e2e = pkgs.stdenv.mkDerivation {
            name = "id-test-e2e";
            src = ./.;
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [
              bun2nixPkg.hook
              # TODO: Switch back to `bunx playwright test` once Bun supports Playwright's
              # ESM config loader (.esm.preflight virtual imports). Bun's runtime doesn't handle
              # the Node.js-specific ESM hooks that Playwright uses for TypeScript config loading.
              # Tracking: https://github.com/oven-sh/bun/pull/28610
              pkgs.nodejs
            ];
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

            buildPhase = ''
              export HOME=$(mktemp -d)
              export CARGO_HOME=$HOME/.cargo

              # @tailwindcss/cli uses @parcel/watcher (native module) which needs libstdc++
              export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

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

              # Run Playwright E2E tests
              # TODO: Switch to `bunx playwright test` once Bun's ESM loader supports
              # Playwright's .esm.preflight virtual imports for TypeScript config loading.
              # Tracking: https://github.com/oven-sh/bun/pull/28610
              (cd e2e && node node_modules/@playwright/test/cli.js test)
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
              treefmt --config-file ./treefmt.toml --tree-root "$(pwd)" --ci 2>&1 || true
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
                -exec rustfmt --check --edition 2024 {} + \
                || true
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
              find . -name '*.nix' -print0 | while IFS= read -r -d "" f; do
                statix check -- "$f" || true
              done
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
                -exec shfmt -d -i 2 -s {} + \
                || true
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
              find . -name '*.sh' -not -path '*/node_modules/*' \
                -exec shellcheck {} + \
                || true
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
                -exec taplo check {} + \
                || true
            '';
            installPhase = ''
              mkdir -p $out
              echo "taplo-check passed at $(date)" > $out/result.txt
            '';
          };
          prettier-check = pkgs.stdenv.mkDerivation {
            name = "id-prettier-check";
            src = ./.;
            nativeBuildInputs = [ pkgs.nodePackages.prettier ];
            buildPhase = ''
              find . \( -name '*.html' -o -name '*.md' -o -name '*.mdx' \
                -o -name '*.scss' -o -name '*.yaml' \) \
                -not -path '*/node_modules/*' -not -path '*/target/*' \
                -not -path '*/dist/*' \
                -exec prettier --check {} + \
                || true
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
            buildFeatures = [ "web" ];

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

            inherit (opensslEnv) OPENSSL_DIR;
            inherit (opensslEnv) OPENSSL_LIB_DIR;
            inherit (opensslEnv) OPENSSL_INCLUDE_DIR;

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
