{
  lib,
  stdenvNoCC,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  nodejs,
  versionCheckHook,
}:

# ready-for-agent — the PM stack's dispatch-layer Harness UI + operator
# CLI (pm-stack Phase 2), packaged from the pinned upstream npm release
# (code#12909; the packaging half of the #12768 deploy registration).
#
# Until this derivation existed, the laptop installed the package
# imperatively: pm-stack/ready-for-agent/install.sh runs
# `npm install -g --prefix ~/.local ready-for-agent@1.1.0`, and the
# NixOS pm-apps module's user unit (#1752) runs
# ~/.local/bin/ready-for-agent. This package gives the deploy registry
# a derivation to point a definition at; switching the pm-apps unit's
# ExecStart to the store binary is the bundled-cutover question on
# #12768/#1752, not this slice.
#
# DISTRIBUTION SHAPE -- why two tarballs: `ready-for-agent` is a thin
# JS launcher (bin/ready-for-agent.js, node >= 20) that resolves the
# platform package `ready-for-agent-linux-x64` through node's module
# resolution and spawns its bin/ready-for-agent, a Bun-compiled
# standalone ELF. npm installs the platform package as an
# optionalDependency nested in the launcher's node_modules; that exact
# layout is reproduced here from the two pinned tarballs, so the
# launcher runs unmodified. The payload is a dynamically linked ELF,
# hence autoPatchelfHook. Same shape as pkgs/claude-code.
#
# To update: bump version, then
#   nix store prefetch-file --json \
#     https://registry.npmjs.org/ready-for-agent/-/ready-for-agent-<ver>.tgz
#   nix store prefetch-file --json \
#     https://registry.npmjs.org/ready-for-agent-linux-x64/-/ready-for-agent-linux-x64-<ver>.tgz
# (the platform packages track the launcher version exactly), and bump
# PINNED_VERSION in pm-stack/ready-for-agent/install.sh in the same
# pass, until the cutover retires the imperative install.

let
  version = "1.1.0";

  launcher = fetchurl {
    url = "https://registry.npmjs.org/ready-for-agent/-/ready-for-agent-${version}.tgz";
    hash = "sha256-OGo+u1+0dwggc7mt6xrynPJzpZs9J3zOT7lrbpOKvEA=";
  };

  platformPkg = fetchurl {
    url = "https://registry.npmjs.org/ready-for-agent-linux-x64/-/ready-for-agent-linux-x64-${version}.tgz";
    hash = "sha256-YrhxxGHTWht7A9O3XJemwqfdGw0VyZQ2ISGsIJU6DtU=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "ready-for-agent";
  inherit version;

  dontUnpack = true;

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
  ];

  # Upstream ships the Bun payload stripped; re-stripping gains nothing.
  dontStrip = true;

  installPhase = ''
    runHook preInstall

    # The launcher package, verbatim (npm tarballs root at package/).
    mkdir -p "$out/lib/ready-for-agent"
    tar -xzf ${launcher} -C "$out/lib/ready-for-agent" --strip-components=1

    # The platform package where the launcher's module resolution
    # looks for it: nested node_modules, exactly as npm's
    # optionalDependency install lays it down.
    mkdir -p "$out/lib/ready-for-agent/node_modules/ready-for-agent-linux-x64"
    tar -xzf ${platformPkg} \
      -C "$out/lib/ready-for-agent/node_modules/ready-for-agent-linux-x64" \
      --strip-components=1
    chmod +x "$out/lib/ready-for-agent/node_modules/ready-for-agent-linux-x64/bin/ready-for-agent"

    install -Dm644 "$out/lib/ready-for-agent/README.md" \
      "$out/share/doc/ready-for-agent/README.md"

    makeWrapper ${lib.getExe nodejs} "$out/bin/ready-for-agent" \
      --add-flags "$out/lib/ready-for-agent/bin/ready-for-agent.js"

    runHook postInstall
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgramArg = "--version";
  doInstallCheck = true;

  meta = {
    description = "Ready for Agent: local Harness UI + backend and operator CLI";
    homepage = "https://github.com/berenddeboer/ready-for-agent";
    downloadPage = "https://www.npmjs.com/package/ready-for-agent";
    license = lib.licenses.mit;
    mainProgram = "ready-for-agent";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
