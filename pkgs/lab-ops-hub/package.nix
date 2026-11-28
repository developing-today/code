{ lib, buildNpmPackage }:

# The lab-ops-hub Worker as a flake package — issue #5767, phase 2.
# Deps are pinned by package-lock.json (npm ci in the sandbox),
# including the pinned wrangler devDependency the deploy app runs.
# There is no separate bundle step: wrangler bundles the TypeScript
# source at deploy time from this package's staged copy, so the
# package is the full Worker tree (src, public assets, wrangler.jsonc,
# node_modules). No deploy-time `npm install` against the network.
buildNpmPackage {
  pname = "lab-ops-hub";
  version = "0.1.0";
  src = ./.;
  npmDepsHash = "sha256-DupOnqFVJiSoBudJvD1kZYs7pt6eX/fKmGXvjzz3mOs=";
  dontNpmBuild = true;
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r src public node_modules $out/
    cp package.json wrangler.jsonc $out/
    runHook postInstall
  '';
  meta = {
    description = "Lab Ops Hub Worker tree with lockfile-pinned deps (incl. pinned wrangler)";
    license = lib.licenses.mit;
  };
}
