# pm-dispatch — authenticated intake (launch/track OpenCode sessions +
# queue intake), built from this repo's pm-stack/dispatch source
# (code#8171: every pm-stack app is a Nix derivation; the units no
# longer `bun run` out of the checkout).
#
# The wrapper pins DISPATCH_BUILD_VERSION to this build's source hash
# (`nix-<12 chars of the src store hash>`). It used to come only from
# the install.sh-rendered ~/pm-stack/dispatch/.env, which the NixOS unit
# still reads, so after the move to Nix packaging /health kept reporting
# whatever commit install.sh last rendered (Nov 2026: a Nov 15 commit),
# and a laptop left days behind main looked current. `--set` wins over
# the EnvironmentFile value because the wrapper exports it at exec.
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

  buildVersion = "nix-${lib.substring 0 12 (baseNameOf "${src}")}";
in
stdenvNoCC.mkDerivation {
  pname = "pm-dispatch";
  inherit (manifest) version;
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
      --set DISPATCH_BUILD_VERSION ${lib.escapeShellArg buildVersion} \
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
