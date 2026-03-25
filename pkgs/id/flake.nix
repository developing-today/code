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
            ]
          }:$PATH"
          exec ${pkgs.treefmt}/bin/treefmt "$@"
        '';

        # =======================================================================
        # Checks: nix flake check -L
        # Uses 'ci' command (read-only, no auto-fix modifications)
        # =======================================================================
        checks = {
          # CI-safe checks (read-only): cargo-fmt-check web-fmt-check clippy-lint web-lint test-sandbox test-web-unit test-web-typecheck doc build release
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

          # Run just with any arguments (fallback for unlisted commands)
          just = mkApp (pkgs.writeShellScriptBin "just-runner" ''
            exec ${pkgs.just}/bin/just "$@"
          '') "Run just with any arguments";
        };
      }
    );
}
