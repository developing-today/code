{
  lib,
  buildNpmPackage,
  fetchurl,
  nodejs,
}:

# Cloudflare `cf` CLI -- Wrangler's successor.
#   https://blog.cloudflare.com/cloudflare-cf-cli-launch/
#   Source: https://github.com/cloudflare/cf (packages/cli)
#
# Why this exists rather than using pkgs.cf / nixpkgs:
#   `cf` entered open beta 2026-09-28 and is not in nixpkgs. It covers the
#   whole Cloudflare API (2,900+ commands generated from the API's OpenAPI
#   schema, vs Wrangler's ~280 hand-written ones), prints JSON by default,
#   and uses a TypeScript config (cloudflare.config.ts); `cf migrate`
#   converts existing wrangler.jsonc projects. Cloudflare's stated plan:
#   when the beta ends, Wrangler gets one final major release pointing at
#   `cf`, then 18 months of maintenance.
#
# Standing direction (Drew, 2026-10-20): prefer `cf` for Cloudflare work;
# Wrangler (../wrangler) stays installed for existing wrangler.jsonc
# projects until they are migrated.
#
# Packaging notes (mirrors ../wrangler):
#   * npm is the only distribution channel (`npm i -g cf` upstream).
#   * The npm tarball ships no package-lock.json, so one is vendored here,
#     generated from the published package.json with devDependencies /
#     scripts stripped and `npm install --package-lock-only`. The same
#     strip is reapplied in postPatch so the lock matches package.json.
#   * Runtime deps are small: miniflare (which pulls the workerd binary),
#     @cloudflare/config, @cloudflare/runtime-types,
#     @cloudflare/build-output-utils, blake3-wasm.
#   * Upstream publishes two bin names -- cf and cloudflare -- both
#     pointing at ./bin/cf. npmInstallHook creates both.
#   * To update: bump version, update `hash`, regenerate package-lock.json
#     as above, then set npmDepsHash from the build failure message.

let
  version = "1.0.0-beta.20";
in
buildNpmPackage {
  pname = "cf";
  inherit version nodejs;

  src = fetchurl {
    url = "https://registry.npmjs.org/cf/-/cf-${version}.tgz";
    hash = "sha256-ITv9lUJXSut0qeOU2cfXkxq1dFxJQorMj18bqLUlMzE=";
  };

  postPatch = ''
    cp ${./package-lock.json} package-lock.json
    chmod +w package.json package-lock.json

    ${lib.getExe nodejs} -e '
      const fs = require("fs");
      const p = JSON.parse(fs.readFileSync("package.json", "utf8"));
      delete p.devDependencies;
      delete p.scripts;
      delete p.workspaces;
      fs.writeFileSync("package.json", JSON.stringify(p, null, 2));
    '
  '';

  npmDepsHash = "sha256-PX7yp2bL1C+bqA5B2lKXe9b0NyCsJ9SicMLbefZQy00=";

  npmFlags = [
    "--ignore-scripts"
    "--no-audit"
    "--no-fund"
  ];

  dontNpmBuild = true;

  meta = {
    description = "The Cloudflare CLI — manage your Cloudflare resources (Wrangler's successor, open beta)";
    homepage = "https://github.com/cloudflare/cf";
    downloadPage = "https://www.npmjs.com/package/cf";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "cf";
    platforms = lib.platforms.all;
  };
}
