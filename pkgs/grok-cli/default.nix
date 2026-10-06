{
  lib,
  stdenvNoCC,
  buildNpmPackage,
  fetchurl,
  nodejs,
  bun,
  makeWrapper,
}:

# Grok CLI -- open-source coding agent for the Grok / xAI API.
#   https://github.com/superagent-ai/grok-cli  (3.5k stars, MIT)
#
# NAMING: the repo is `grok-cli` but it publishes to npm as `grok-dev`. The npm
# packages actually called `grok-cli` and `grok` are unrelated -- `grok-cli`
# 1.0.5 is a third-party shim that proxies Grok into claude-code (stale since
# 2025-07), and `grok` 0.0.4 is a 2013 package with nothing to do with xAI.
#
# Why this one: t3code's `grok` driver shells out to a binary named `grok`
# (`grokSettings.binaryPath || "grok"`), negotiates over ACP
# (`discoverGrokMetadataViaAcpInitialize`), and reads credentials from
# `~/.grok/auth.json` or $GROK_AUTH keyed on `https://auth.x.ai::...`. This
# package provides `bin.grok`, so it drops into that driver directly.
#
# RUNTIME: this is a **Bun** application, not a Node one, even though package.json
# points `bin.grok` at a plain .js file. Running it under Node fails in three
# escalating ways, all of which are really the same fact:
#   1. ERR_IMPORT_ATTRIBUTE_MISSING -- `import ... from "../package.json"` with
#      no `with { type: "json" }`.
#   2. ERR_MODULE_NOT_FOUND on extensionless relative imports
#      (`from "./agent/agent"`), which strict ESM rejects.
#   3. ERR_UNSUPPORTED_ESM_URL_SCHEME -- it imports `bun:` builtins.
# Only (3) is unfixable by patching, and it is decisive. Bun is lenient about
# (1) and (2) anyway, so running under Bun needs no source edits at all.
#
# node_modules still comes from buildNpmPackage (reproducible, vendored lock);
# only the entrypoint is executed by Bun.
#
# To update: bump version + hash, regenerate package-lock.json from the
# published package.json with devDependencies stripped and --legacy-peer-deps
# (there is an unmet `react` peer), then refresh npmDepsHash.

let
  version = "1.1.7";

  deps = buildNpmPackage {
    pname = "grok-cli-deps";
    inherit version nodejs;

    src = fetchurl {
      url = "https://registry.npmjs.org/grok-dev/-/grok-dev-${version}.tgz";
      hash = "sha256-BQkWm8korUmsc/nqKiI5jpLt39Xveoe4XbY75VDAZ3c=";
    };

    postPatch = ''
      cp ${./package-lock.json} package-lock.json
      chmod +w package.json package-lock.json
      ${lib.getExe nodejs} -e '
        const fs = require("fs");
        const p = JSON.parse(fs.readFileSync("package.json", "utf8"));
        delete p.devDependencies; delete p.scripts; delete p.workspaces;
        fs.writeFileSync("package.json", JSON.stringify(p, null, 2));
      '
    '';

    npmDepsHash = "sha256-4ok7AsxVZBEh3LauKfhd59mK8Yxkq7sL66WoTmfoGnc=";
    npmFlags = [
      "--ignore-scripts"
      "--no-audit"
      "--no-fund"
      "--legacy-peer-deps"
    ];
    dontNpmBuild = true;

    # The npm-generated bin wrapper invokes node; it is replaced below.
    meta.mainProgram = "grok";
  };
in
stdenvNoCC.mkDerivation {
  pname = "grok-cli";
  inherit version;

  dontUnpack = true;
  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin"
    makeWrapper ${lib.getExe bun} "$out/bin/grok" \
      --add-flags "run" \
      --add-flags "${deps}/lib/node_modules/grok-dev/dist/index.js"
    runHook postInstall
  '';

  meta = {
    description = "Open-source coding agent for the Grok (xAI) API";
    homepage = "https://github.com/superagent-ai/grok-cli";
    license = lib.licenses.mit;
    mainProgram = "grok";
    platforms = lib.platforms.unix;
  };
}
