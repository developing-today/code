{
  lib,
  stdenvNoCC,
  nodejs,
}:

# The queue-transport-alert Worker tree as a flake package (code#6369).
# Plain ESM with no npm dependencies, so the package is the source tree
# itself; cf bundles it at deploy time from a staged copy. The
# contract tests run in checkPhase.
stdenvNoCC.mkDerivation {
  pname = "queue-transport-alert-worker";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./src
      ./test
      ./package.json
      ./wrangler.jsonc
      ./cloudflare.config.ts
      ./deploy-lib.mjs
    ];
  };
  nativeCheckInputs = [ nodejs ];
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    node --test test/*.test.mjs
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r src package.json wrangler.jsonc cloudflare.config.ts $out/
    runHook postInstall
  '';
  meta = {
    description = "Cloudflare Worker that alerts on a repo-queue broker outage";
    license = lib.licenses.mit;
  };
}
