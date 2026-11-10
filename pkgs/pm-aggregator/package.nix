# pm-aggregator — the PM-stack aggregator (board / roadmap / quota /
# needs-you), built from this repo's pm-stack/aggregator source
# (code#8171: every pm-stack app is a Nix derivation; the units no
# longer `bun run` out of the checkout).
#
# One package, four entry points: the server plus the script entry
# points the timer units run (quota gatherer, digest generator). The
# fleet-feed collector script is deliberately NOT wrapped here: it
# derives repo-root paths from its own file location (the checkout's
# .opencode/opencode.jsonc and a checkout-drift self-inspection), so
# its unit keeps running it from the checkout — see the module.
#
# kaneo-projects.json (pm-stack/kaneo-projects.json) is bundled: the
# aggregator's default for it is cwd-relative (../kaneo-projects.json
# from the old checkout working directory), and the unit now points
# PM_KANEO_PROJECTS_FILE at the bundled copy.
#
# Deps: a fixed-output derivation runs `bun install` once (FODs get
# network) and the app derivation copies the resulting node_modules in
# beside the source. The opencode fork's node_modules.nix is the fleet
# precedent; the openchamber #449 normalization is applied in the FOD:
# bun flips a `.bin` symlink between installs and breaks the whole-tree
# hash, so every `.bin` link dir is stripped from the FOD output
# (nothing at runtime resolves binaries through .bin). --production:
# the tailwind/typescript devDependencies are build-time-only for a
# CSS artifact that is checked in (src/assets/app.css).
{
  lib,
  stdenvNoCC,
  bun,
  makeWrapper,
}:
let
  manifest = lib.importJSON ../../pm-stack/aggregator/package.json;

  src = lib.fileset.toSource {
    root = ../../pm-stack/aggregator;
    fileset = lib.fileset.unions [
      ../../pm-stack/aggregator/package.json
      ../../pm-stack/aggregator/bun.lock
      ../../pm-stack/aggregator/src
      ../../pm-stack/aggregator/scripts
    ];
  };

  nodeModules = stdenvNoCC.mkDerivation {
    name = "pm-aggregator-node_modules-${manifest.version}";
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
    outputHash = "sha256-mUS1vV4fMmlRl73NSQT7X8yBqvknvQSGZ47H1jscRcI=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "pm-aggregator";
  version = manifest.version;
  inherit src;

  nativeBuildInputs = [ makeWrapper ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/libexec/pm-aggregator $out/bin
    cp -R . $out/libexec/pm-aggregator/
    cp -R ${nodeModules}/node_modules $out/libexec/pm-aggregator/node_modules
    cp ${../../pm-stack/kaneo-projects.json} $out/libexec/pm-aggregator/kaneo-projects.json
    # src/sources/fixtures.ts imports ../../../grist/seed/roadmap.json
    # relative to its own file; keep that relative shape inside the
    # package so the import resolves to this bundled copy.
    mkdir -p $out/libexec/grist/seed
    cp ${../../pm-stack/grist/seed/roadmap.json} $out/libexec/grist/seed/roadmap.json
    makeWrapper ${lib.getExe bun} $out/bin/pm-aggregator \
      --add-flags "run $out/libexec/pm-aggregator/src/server.ts"
    makeWrapper ${lib.getExe bun} $out/bin/pm-aggregator-gather-quota \
      --add-flags "run $out/libexec/pm-aggregator/scripts/gather-quota.ts"
    makeWrapper ${lib.getExe bun} $out/bin/pm-aggregator-generate-digest \
      --add-flags "run $out/libexec/pm-aggregator/scripts/generate-digest.ts"
    runHook postInstall
  '';

  dontFixup = true;

  meta = {
    description = "PM aggregator — board / roadmap / quota / needs-you projection for the PM stack";
    mainProgram = "pm-aggregator";
    platforms = lib.platforms.linux;
  };
}
