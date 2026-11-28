{ lib, buildNpmPackage }:

# The cold-store-rehydrator Worker bundle as a flake package — issue
# #5767, phase 2. Deps are pinned by package-lock.json (npm ci in the
# sandbox); the build is the package's own esbuild bundle script, so
# $out/worker.js is the built artifact the deploy app uploads through
# the versions flow — no deploy-time `npm install`, and the committed
# dist/ bundle is no longer the deploy artifact.
buildNpmPackage {
  pname = "cold-store-rehydrator";
  version = "1.0.0";
  src = ./.;
  npmDepsHash = "sha256-+yH6zVXfQxZKAzjXU2oyqEX4ZnNKUb8BMHtLf9NALAc=";
  npmBuildScript = "build";
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp dist/worker.js $out/worker.js
    runHook postInstall
  '';
  meta = {
    description = "Cold-store rehydrator Worker bundle (esbuild, lockfile-pinned deps)";
    license = lib.licenses.mit;
  };
}
