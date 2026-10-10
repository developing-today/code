{
  lib,
  stdenvNoCC,
  bun,
  nodejs_24,
  makeWrapper,
  git,
  openssh,
  opencode,
  src,
}:
let
  cleanSrc = lib.cleanSourceWith {
    inherit src;
    filter = path: _type: !(lib.hasInfix "/node_modules" path);
  };

  version = (lib.importJSON "${cleanSrc}/packages/web/package.json").version;

  # Fetched with bun, not npm: the fork's patchedDependencies (bun-patches/) cannot be applied by npm.
  node_modules = stdenvNoCC.mkDerivation {
    pname = "openchamber-node_modules";
    inherit version;
    src = cleanSrc;

    impureEnvVars = lib.fetchers.proxyImpureEnvVars;

    nativeBuildInputs = [ bun ];
    dontConfigure = true;

    buildPhase = ''
      runHook preBuild
      export HOME=$TMPDIR
      export BUN_INSTALL_CACHE_DIR=$(mktemp -d)
      bun install --frozen-lockfile --ignore-scripts --no-progress
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p $out
      find . -type d -name node_modules -prune -exec cp -R --parents {} $out \;
      runHook postInstall
    '';

    dontFixup = true;
    outputHashAlgo = "sha256";
    outputHashMode = "recursive";
    outputHash = "sha256-Kmu8bmrytwUG+AdFEDKTDrIJUlQBvMNERuFqd33LqSw=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "openchamber";
  inherit version;
  src = cleanSrc;

  nativeBuildInputs = [
    bun
    nodejs_24
    makeWrapper
  ];

  dontConfigure = true;

  buildPhase = ''
    runHook preBuild
    export HOME=$TMPDIR
    cp -R ${node_modules}/. .
    chmod -R u+w .
    patchShebangs node_modules packages
    bun run --cwd packages/web build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/openchamber
    cp -R . $out/lib/openchamber/
    makeWrapper ${lib.getExe nodejs_24} $out/bin/openchamber \
      --add-flags $out/lib/openchamber/packages/web/bin/cli.js \
      --prefix PATH : ${
        lib.makeBinPath [
          git
          openssh
          opencode
        ]
      } \
      --set DISABLE_AUTOUPDATER 1 \
      --set npm_config_update_notifier false
    runHook postInstall
  '';

  passthru = { inherit node_modules; };

  meta = {
    description = "OpenChamber built from source (route-pools fork)";
    license = lib.licenses.mit;
    mainProgram = "openchamber";
    platforms = lib.platforms.linux;
  };
}
