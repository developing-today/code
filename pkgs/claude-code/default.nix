{
  lib,
  stdenvNoCC,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  versionCheckHook,
  git,
  openssh,
  ripgrep,
}:

# Claude Code CLI, pinned ahead of nixpkgs.
#
# nixpkgs (both the pinned channel and unstable) carries 2.1.234 while npm is on
# 2.1.289. This packages the newer one.
#
# DISTRIBUTION SHAPE -- this is why it is not just a buildNpmPackage bump:
#   `@anthropic-ai/claude-code` 2.x has ZERO dependencies and ships a 500-byte
#   shell *stub* at bin/claude.exe whose entire job is to print
#   "Error: claude native binary not installed." The real program is a
#   platform-native ELF delivered as one of eight optionalDependencies
#   (@anthropic-ai/claude-code-{linux,darwin,win32}-{x64,arm64}[-musl]),
#   fetched by an install.cjs postinstall. That postinstall cannot run in a
#   sandboxed build, so the platform package is pinned directly and the wrapper
#   package is skipped entirely.
#
# The payload is a dynamically linked ELF, hence autoPatchelfHook.
#
# Runtime deps on PATH mirror what the nixpkgs derivation provides: git and
# openssh for repo work, ripgrep for search (Claude Code shells out to `rg`).
#
# To update: bump version, then
#   nix store prefetch-file --json \
#     https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-<ver>.tgz
# Check https://registry.npmjs.org/@anthropic-ai/claude-code for the latest
# version; the platform packages track it exactly.

let
  version = "2.1.289";

  throwSystem = throw "claude-code: unsupported system ${stdenvNoCC.hostPlatform.system}";

  sources = {
    x86_64-linux = {
      url = "https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-${version}.tgz";
      hash = "sha256-UKbrQzQi/rz6e3rB54unlc33nx/ljwF/sdmuuUkbM30=";
    };
  };
in
stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "claude-code";
  inherit version;

  src = fetchurl (sources.${stdenvNoCC.hostPlatform.system} or throwSystem);

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
  ];

  # Upstream ships it stripped; re-stripping gains nothing.
  dontStrip = true;

  installPhase = ''
    runHook preInstall

    install -Dm755 claude "$out/bin/.claude-unwrapped"
    install -Dm644 LICENSE.md "$out/share/doc/claude-code/LICENSE.md"

    makeWrapper "$out/bin/.claude-unwrapped" "$out/bin/claude" \
      --prefix PATH : ${
        lib.makeBinPath [
          git
          openssh
          ripgrep
        ]
      }

    runHook postInstall
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgramArg = "--version";
  doInstallCheck = true;

  meta = {
    description = "Agentic coding tool that lives in your terminal";
    homepage = "https://github.com/anthropics/claude-code";
    downloadPage = "https://www.npmjs.com/package/@anthropic-ai/claude-code";
    license = lib.licenses.unfree; # Anthropic Commercial Terms of Service
    mainProgram = "claude";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
})
