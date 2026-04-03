# Lootbox — "Code Mode for LLMs"
# https://github.com/jx-codes/lootbox
#
# Two-phase build:
#   1. FOD (fixed-output derivation) caches all Deno deps (jsr, npm, esm.sh)
#   2. Pure derivation builds UI and compiles the standalone binary
#
# To update after a lootbox version bump:
#   just update-lootbox
#
{ pkgs }:

let
  version = "0.0.54";
  rev = "587a5a1b2694d0d00168665d8f1a536bc54e0f1a";

  src = pkgs.fetchFromGitHub {
    owner = "jx-codes";
    repo = "lootbox";
    inherit rev;
    hash = "sha256-uY8VETshvwIbGjq10NRVc8ts4IEsKypvdBcjLqOLqu0=";
  };

  # Phase 1: Fixed-output derivation — download all dependencies.
  # FODs get network access because nix trusts their content hash.
  deps = pkgs.stdenv.mkDerivation {
    name = "lootbox-deps-${version}";
    inherit src;

    nativeBuildInputs = [ pkgs.deno ];

    # FOD configuration
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-t9Vzb0e3F4SPN2LD+fOeCP1bcC7Y1IWH8NnIDYct/4M=";

    buildPhase = ''
      runHook preBuild

      export HOME=$TMPDIR
      export DENO_DIR=$TMPDIR/deno-cache

      # Cache root project deps (jsr, npm, esm.sh imports)
      deno install --lock=deno.lock --entrypoint src/lootbox-cli.ts

      # Cache UI deps (npm packages for vite build)
      cd ui
      deno install
      cd ..

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp -r $DENO_DIR $out
      runHook postInstall
    '';
  };

in
pkgs.stdenv.mkDerivation {
  pname = "lootbox";
  inherit version src;

  nativeBuildInputs = [ pkgs.deno ];

  buildPhase = ''
    runHook preBuild

    export HOME=$TMPDIR
    export DENO_DIR=$TMPDIR/deno-cache

    # Copy cached deps (deno needs write access for cache metadata)
    cp -r ${deps} $DENO_DIR
    chmod -R u+w $DENO_DIR

    # Create node_modules/ from cached deps (no network — cached-only)
    export DENO_NO_REMOTE=1
    deno install
    cd ui
    deno install
    cd ..

    # Build the UI (vite produces ui/dist/)
    cd ui
    deno run -A npm:vite build
    cd ..

    # Compile standalone binary
    deno compile --allow-all --include ui/dist -o lootbox src/lootbox-cli.ts

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp lootbox $out/bin/
    runHook postInstall
  '';

  meta = with pkgs.lib; {
    description = "Code Mode for LLMs — TypeScript tool execution engine";
    homepage = "https://github.com/jx-codes/lootbox";
    license = licenses.mit;
    mainProgram = "lootbox";
  };
}
