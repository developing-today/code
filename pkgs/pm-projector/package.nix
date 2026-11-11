# pm-projector — projector + link registry for the PM stack, built from
# this repo's pm-stack/projector source (code#8171: every pm-stack app is
# a Nix derivation; the units no longer `bun run` out of the checkout).
#
# Deps: a fixed-output derivation runs `bun install` once (FODs get
# network) and the app derivation copies the resulting node_modules in
# beside the source. The opencode fork's node_modules.nix is the fleet
# precedent; the openchamber #449 normalization is applied in the FOD:
# bun flips a `.bin` symlink between installs and breaks the whole-tree
# hash, so every `.bin` link dir is stripped from the FOD output
# (nothing at runtime resolves binaries through .bin).
{
  lib,
  stdenvNoCC,
  bun,
  makeWrapper,
}:
let
  manifest = lib.importJSON ../../pm-stack/projector/package.json;

  src = lib.fileset.toSource {
    root = ../../pm-stack/projector;
    fileset = lib.fileset.unions [
      ../../pm-stack/projector/package.json
      ../../pm-stack/projector/bun.lock
      ../../pm-stack/projector/schema.sql
      ../../pm-stack/projector/src
    ];
  };

  nodeModules = stdenvNoCC.mkDerivation {
    name = "pm-projector-node_modules-${manifest.version}";
    inherit src;

    nativeBuildInputs = [ bun ];

    impureEnvVars = lib.fetchers.proxyImpureEnvVars ++ [
      "GIT_PROXY_COMMAND"
      "SOCKS_SERVER"
    ];

    dontConfigure = true;

    buildPhase = ''
      runHook preBuild
      export HOME=$TMPDIR
      export BUN_INSTALL_CACHE_DIR=$(mktemp -d)
      bun install \
        --production \
        --frozen-lockfile \
        --ignore-scripts \
        --linker=hoisted \
        --no-progress
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p $out
      cp -R node_modules $out/node_modules
      find $out -type d -name .bin -prune -exec rm -rf {} +
      runHook postInstall
    '';

    dontFixup = true;

    outputHashAlgo = "sha256";
    outputHashMode = "recursive";
    outputHash = "sha256-nzFNa8cYt6peCVO3PnpYpHxnuyc6sgRL6ZchqhSThcI=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "pm-projector";
  version = manifest.version;
  inherit src;

  nativeBuildInputs = [ makeWrapper ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/libexec/pm-projector $out/bin
    cp -R . $out/libexec/pm-projector/
    cp -R ${nodeModules}/node_modules $out/libexec/pm-projector/node_modules
    makeWrapper ${lib.getExe bun} $out/bin/pm-projector \
      --add-flags "run $out/libexec/pm-projector/src/index.ts"
    # The reconcile CLI (package.json's "reconcile" script) is the same
    # code against the same DB; ship it beside the server.
    makeWrapper ${lib.getExe bun} $out/bin/pm-projector-cli \
      --add-flags "run $out/libexec/pm-projector/src/cli.ts"
    runHook postInstall
  '';

  # The libsql native binaries are prebuilt upstream artifacts; patchELF
  # / strip would only risk mangling them.
  dontFixup = true;

  meta = {
    description = "PM projector + link registry — GitHub projection for the PM aggregator";
    mainProgram = "pm-projector";
    platforms = lib.platforms.linux;
  };
}
