{
  lib,
  stdenvNoCC,
  fetchurl,
  makeWrapper,
  nodejs_24,
}:

# Pi, Earendil's minimal coding agent harness.
#   https://pi.dev  /  github:earendil-works/pi (packages/coding-agent)
#
# Packaging notes:
#   * Not in nixpkgs, and upstream ships no Nix flake (unlike OMP).
#   * The published npm tarball is already built: `prepublishOnly` runs the
#     bundler, so dist/bundle/ contains ~7MB of self-contained chunks whose only
#     bare imports are Node builtins. That means no npm dependency resolution
#     (buildNpmPackage / npmDepsHash) is needed at all — we just drop the tree in
#     place and wrap it with node.
#   * The handful of non-builtin specifiers left in the bundle
#     (bufferutil, utf-8-validate, supports-color, @aws-sdk/signature-v4-crt)
#     are optional accelerators referenced defensively; Pi runs without them.
#   * nodejs_24 is used because the bundle imports `node:sqlite`, which is only
#     available from Node 22.5 onwards.
#   * Upstream's own install docs pass `--ignore-scripts`, so there are no
#     postinstall steps to replicate.
stdenvNoCC.mkDerivation rec {
  pname = "pi-coding-agent";
  version = "0.84.3";

  src = fetchurl {
    url = "https://registry.npmjs.org/@earendil-works/pi-coding-agent/-/pi-coding-agent-${version}.tgz";
    hash = "sha256-0H3EF/eKFNrDdqh4tlVrUZYfEY95dx7jdTM9xRNWvHU=";
  };

  nativeBuildInputs = [ makeWrapper ];

  sourceRoot = "package";

  dontBuild = true;

  installPhase = ''
    runHook preInstall

    libDir="$out/lib/pi-coding-agent"
    mkdir -p "$libDir" "$out/bin"

    # dist/ holds the prebuilt bundle; docs/ and examples/ are loaded at runtime
    # for /help and the extension examples.
    cp -r dist docs examples package.json README.md "$libDir/"

    makeWrapper ${lib.getExe nodejs_24} "$out/bin/pi" \
      --add-flags "$libDir/dist/bundle/cli.js"

    runHook postInstall
  '';

  doInstallCheck = true;

  installCheckPhase = ''
    runHook preInstallCheck

    export HOME="$PWD/home"
    mkdir -p "$HOME"
    "$out/bin/pi" --version

    runHook postInstallCheck
  '';

  meta = {
    description = "Minimal, aggressively extensible coding agent harness";
    homepage = "https://pi.dev";
    downloadPage = "https://github.com/earendil-works/pi/tree/main/packages/coding-agent";
    license = lib.licenses.mit;
    sourceProvenance = with lib.sourceTypes; [ binaryBytecode ];
    mainProgram = "pi";
    platforms = lib.platforms.unix;
  };
}
