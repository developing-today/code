{
  lib,
  stdenvNoCC,
  fetchurl,
  makeWrapper,
  nodejs,
}:

# Vercel CLI.
#   https://vercel.com/docs/cli
#   Source: https://github.com/vercel/vercel (packages/cli), Apache-2.0
#
# Packaging notes:
#   * Not in nixpkgs. The `vercel-pkg` attribute there is the unrelated legacy
#     `pkg` bundler, not this. An init PR exists (NixOS/nixpkgs#561169, opened
#     2026-09-08) but has been stale since 2026-09-29 and is already several
#     minor versions behind, so we package it here rather than wait.
#   * npm is the only distribution channel. `vercel/vercel` cuts per-package
#     monorepo tags with no attached binary assets, so there is no GitHub
#     release artifact to fetch -- the registry tarball is the artifact.
#   * Deliberately NOT buildNpmPackage. The published tarball is already fully
#     bundled (esbuild/ncc output in dist/chunks/*), and `node dist/vc.js
#     --version` was verified to run correctly with no node_modules present at
#     all. The 40+ entries in package.json `dependencies` are build-time inputs
#     that are already inlined into dist; installing them would download a large
#     tree to satisfy imports that never resolve at run time.
#     This also sidesteps needing a vendored package-lock.json, which the npm
#     tarball does not ship.
#   * Upstream publishes two bins, `vercel` and `vc`, both pointing at the same
#     dist/vc.js. Both are provided here.
#   * Self-update: the CLI checks for newer versions and prints a notice. It
#     cannot write to /nix/store, so the check is harmless, but
#     NO_UPDATE_NOTIFIER is set to keep it quiet.
#   * To update: bump version, then
#       nix hash file "$(curl -sSL "$(curl -sS https://registry.npmjs.org/vercel \
#         | jq -r '.versions[.["dist-tags"].latest].dist.tarball')" -o /tmp/v.tgz; echo /tmp/v.tgz)"

let
  version = "62.2.0";
in
stdenvNoCC.mkDerivation {
  pname = "vercel-cli";
  inherit version;

  src = fetchurl {
    url = "https://registry.npmjs.org/vercel/-/vercel-${version}.tgz";
    hash = "sha256-aKgc28X43QtaY9QfxwLZfSjWXG0bRzyy9VFC6qvVviY=";
  };

  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall

    mkdir -p "$out/lib/vercel"
    cp -r dist package.json "$out/lib/vercel/"

    for bin in vercel vc; do
      makeWrapper ${lib.getExe nodejs} "$out/bin/$bin" \
        --add-flags "$out/lib/vercel/dist/vc.js" \
        --set NO_UPDATE_NOTIFIER 1
    done

    runHook postInstall
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    got="$("$out/bin/vercel" --version 2>&1 | tail -n1)"
    if [ "$got" != "${version}" ]; then
      echo "version mismatch: expected ${version}, got '$got'" >&2
      exit 1
    fi
    runHook postInstallCheck
  '';

  meta = {
    description = "Command-line interface for Vercel";
    homepage = "https://vercel.com/docs/cli";
    downloadPage = "https://www.npmjs.com/package/vercel";
    license = lib.licenses.asl20;
    mainProgram = "vercel";
    platforms = lib.platforms.all;
  };
}
