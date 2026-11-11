# pm-dispatch — authenticated intake (launch/track OpenCode sessions +
# queue intake), built from this repo's pm-stack/dispatch source
# (code#8171: every pm-stack app is a Nix derivation; the units no
# longer `bun run` out of the checkout).
#
# The package.json carries no runtime dependencies (only type-level
# devDependencies), so there is no node_modules to fetch: the package
# is the source plus a bun wrapper. The server shells out to `gh`
# (src/github.ts), so the wrapper pins gh on PATH instead of relying
# on the ambient profile.
{
  lib,
  stdenvNoCC,
  bun,
  gh,
  makeWrapper,
}:
let
  manifest = lib.importJSON ../../pm-stack/dispatch/package.json;

  src = lib.fileset.toSource {
    root = ../../pm-stack/dispatch;
    fileset = lib.fileset.unions [
      ../../pm-stack/dispatch/package.json
      ../../pm-stack/dispatch/bun.lock
      ../../pm-stack/dispatch/src
    ];
  };
in
stdenvNoCC.mkDerivation {
  pname = "pm-dispatch";
  version = manifest.version;
  inherit src;

  nativeBuildInputs = [ makeWrapper ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/libexec/pm-dispatch $out/bin
    cp -R . $out/libexec/pm-dispatch/
    makeWrapper ${lib.getExe bun} $out/bin/pm-dispatch \
      --add-flags "run $out/libexec/pm-dispatch/src/server.ts" \
      --prefix PATH : ${lib.makeBinPath [ gh ]}
    runHook postInstall
  '';

  dontFixup = true;

  meta = {
    description = "PM dispatch — authenticated intake for OpenCode sessions and queue work";
    mainProgram = "pm-dispatch";
    platforms = lib.platforms.linux;
  };
}
