{
  lib,
  buildNpmPackage,
  fetchurl,
  nodejs,
}:

# supergateway — stdio <-> HTTP bridge for MCP servers.
#   https://github.com/supercorp-ai/supergateway
#
# WHY THIS PACKAGE EXISTS:
#   The self-hosted Desktop Commander endpoint (#13404) needs the
#   packaged Desktop Commander (a stdio MCP server) reachable over
#   streamable HTTP on the laptop, behind the Cloudflare Tunnel +
#   Access bridge door. supergateway is that bridge: one process wraps
#   the stdio server and serves /mcp on 127.0.0.1:8931 (see the
#   desktop-commander-bridge unit in home/common/default.nix).
#
# Packaging notes (same shape as ../desktop-commander):
#   * Not in nixpkgs as a maintained attr we can rely on; npm is the
#     distribution channel, so the registry tarball is the artifact.
#   * dist/ is prebuilt upstream, so there is no build step:
#     dontNpmBuild = true.
#   * The tarball ships a package-lock.json generated from the full
#     manifest, so one is vendored here, regenerated after stripping
#     devDependencies/scripts (postPatch reapplies the same strip, so
#     the lock keeps matching the package.json that `npm ci` sees):
#       npm install --package-lock-only --omit=dev --ignore-scripts
#   * --ignore-scripts keeps install hooks (and any network fetches
#     they would do) out of the sandboxed build.
#   * To update: bump version, refresh `hash`,
#       nix store prefetch-file --json \
#         https://registry.npmjs.org/supergateway/-/supergateway-<ver>.tgz
#     regenerate package-lock.json as described above, then set
#     npmDepsHash from the build failure message.

let
  version = "4.2.4";
in
buildNpmPackage {
  pname = "supergateway";
  inherit version nodejs;

  src = fetchurl {
    url = "https://registry.npmjs.org/supergateway/-/supergateway-${version}.tgz";
    hash = "sha256-xyOK33kCZbBWHqGH35B9g2G/oatTp9qLkIWJViGLlGQ=";
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

  npmDepsHash = "sha256-TGD66++apAE3L+wWUFDQi2MPptB4u5qFCPok8FAbnAA=";

  npmFlags = [
    "--ignore-scripts"
    "--no-audit"
    "--no-fund"
  ];

  dontNpmBuild = true;

  meta = {
    description = "MCP stdio to SSE / streamable-HTTP gateway";
    homepage = "https://github.com/supercorp-ai/supergateway";
    license = lib.licenses.mit;
    mainProgram = "supergateway";
    platforms = lib.platforms.all;
  };
}
