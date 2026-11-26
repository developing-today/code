{
  lib,
  stdenvNoCC,
  nodejs,
}:

# The d1-gateway Worker tree as a flake package. Plain ESM with no npm
# dependencies, so the package is the source tree; wrangler bundles it
# at deploy time (deploy.sh). The tests run in checkPhase against a D1
# stand-in built on node:sqlite.
stdenvNoCC.mkDerivation {
  pname = "d1-gateway-worker";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./src
      ./test
      ./package.json
      ./wrangler.jsonc
    ];
  };
  nativeCheckInputs = [ nodejs ];
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    flag=""
    if node --experimental-sqlite -e "" 2>/dev/null; then flag=--experimental-sqlite; fi
    node $flag --test test/*.test.mjs
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r src package.json wrangler.jsonc $out/
    runHook postInstall
  '';
  meta = {
    description = "Cloudflare Worker: Turso/libSQL-compatible Hrana HTTP endpoint over D1";
    license = lib.licenses.mit;
  };
}
