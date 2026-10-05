{
  lib,
  buildNpmPackage,
  fetchurl,
  nodejs,
}:

# @google/jules-merge -- experimental multi-agent orchestration for Jules.
#   https://github.com/google-labs-code/jules-sdk
#
# Google's own description is "very experimental, just for fun". It reads
# markdown goal files from .fleet/goals/, opens Jules analyzer sessions, files
# GitHub issues labelled `fleet`, spawns worker sessions and merges the
# resulting PRs sequentially. Driven either by a GitHub Actions cron or the CLI.
#
# Packaging notes:
#   * Not self-contained -- `node dist/cli/index.mjs` fails without its 10
#     runtime deps, so buildNpmPackage rather than a bare unpack.
#   * The npm tarball ships no package-lock.json; one is vendored here,
#     generated from the published package.json with devDependencies/scripts
#     stripped. The same strip is reapplied in postPatch so lock and manifest
#     agree.
#   * Needs JULES_API_KEY at runtime (see programs.bash.initExtra in
#     home/common), and a GitHub token for the issue/PR side.
let
  version = "0.1.0";
in
buildNpmPackage {
  pname = "jules-merge";
  inherit version nodejs;
  src = fetchurl {
    url = "https://registry.npmjs.org/@google/jules-merge/-/jules-merge-${version}.tgz";
    hash = "sha256-k1z7Dy+1POoZzp3J2yTQWQbUGOc44XiCnKprLEAZN24=";
  };
  postPatch = ''
    cp ${./package-lock.json} package-lock.json
    chmod +w package.json package-lock.json
    ${lib.getExe nodejs} -e '
      const fs=require("fs");const p=JSON.parse(fs.readFileSync("package.json","utf8"));
      delete p.devDependencies; delete p.scripts; delete p.workspaces;
      fs.writeFileSync("package.json",JSON.stringify(p,null,2));'
  '';
  npmDepsHash = "sha256-KhOJSDnnLJN38RaiqM9MsoY4qOo+CGvPbv/IlJDhDxM=";
  npmFlags = [ "--ignore-scripts" "--no-audit" "--no-fund" ];
  dontNpmBuild = true;
  meta = {
    description = "Reconcile overlapping PRs from parallel Jules agents";
    homepage = "https://github.com/google-labs-code/jules-sdk";
    license = lib.licenses.asl20;
    mainProgram = "jules-merge";
    platforms = lib.platforms.all;
  };
}
