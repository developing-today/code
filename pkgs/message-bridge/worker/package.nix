{
  lib,
  stdenvNoCC,
  nodejs,
}:

# The message-bridge Worker tree as a flake package. Plain ESM with no
# npm dependencies, so the package is the source tree itself; cf
# (delegating to wrangler) bundles it at deploy time from a staged copy. The tests run in
# checkPhase.
stdenvNoCC.mkDerivation {
  pname = "message-bridge-worker";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./src
      ./test
      ./cloudflare.config.ts
      ./deploy.mjs # its argument check is tested
      ./deploy-args.mjs
      ./deploy-probe.mjs
      ./bridge.mjs
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
    cp -r src cloudflare.config.ts package.json wrangler.jsonc bridge.mjs $out/
    runHook postInstall
  '';
  meta = {
    description = "Cloudflare Worker: remote MCP connector that sends and receives messages, with a history page";
    license = lib.licenses.mit;
  };
}
