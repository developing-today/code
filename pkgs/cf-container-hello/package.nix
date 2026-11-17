{ lib, buildNpmPackage }:

# The cf-container-hello Worker as a flake package — issue #5767,
# phase 2. Deps are pinned by package-lock.json (npm ci in the
# sandbox; the lockfile itself is new in this slice — the Worker
# previously had only a package.json with a floating range). Wrangler
# bundles the TypeScript source at deploy time from this package's
# staged copy, so the package is the full Worker tree (src,
# wrangler.jsonc, node_modules). No deploy-time `npm install`.
buildNpmPackage {
  pname = "cf-container-hello";
  version = "0.0.1";
  src = ./.;
  npmDepsHash = "sha256-oHQc1BK4vl7DxFkA4cwPf4cw7MdyejoiDs4TeC6zOr4=";
  dontNpmBuild = true;
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r src node_modules $out/
    cp package.json wrangler.jsonc $out/
    runHook postInstall
  '';
  meta = {
    description = "cf-container-hello Worker tree with lockfile-pinned deps";
    license = lib.licenses.mit;
  };
}
