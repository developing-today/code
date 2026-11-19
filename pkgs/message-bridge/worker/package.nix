{
  lib,
  stdenvNoCC,
  nodejs,
}:

# The message-bridge Worker tree as a flake package. Plain ESM with no
# npm dependencies, so the package is the source tree itself; wrangler
# bundles it at deploy time from a staged copy. The tests run in
# checkPhase.
stdenvNoCC.mkDerivation {
  pname = "message-bridge-worker";
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
    node --test test/*.test.mjs
    runHook postCheck
  '';
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r src package.json wrangler.jsonc $out/
    runHook postInstall
  '';
  meta = {
    description = "Cloudflare Worker: remote MCP connector that sends and receives messages, with a history page";
    license = lib.licenses.mit;
  };
}
