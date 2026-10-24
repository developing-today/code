{
  lib,
  stdenvNoCC,
  fetchurl,
}:

# Muse Code, Meta's coding agent for the terminal and CI.
#   https://dev.meta.ai/docs/muse-code
#   SDK (MIT, open): https://github.com/meta-models/muse-code-sdk
#
# Packaging notes:
#   * Not in nixpkgs. The `muse` attribute there is muse-sequencer, an unrelated
#     MIDI/audio DAW. Upstream ships no flake and the agent binary itself is
#     closed source -- only the MSP SDK is public.
#   * The documented install is `curl -fsSL https://dev.meta.ai/install.sh | sh`,
#     which is unusable declaratively. That script does nothing but fetch a
#     launcher from api.meta.ai and exec it with MUSE_LAUNCHER_INSTALL=1; the
#     launcher is a self-updating shim that resolves the real artifact at run
#     time. We bypass both layers and pin the artifact directly:
#
#       channel manifest : https://api.meta.ai/muse-code/channels/muse-stable
#         -> { version, manifest_url }
#       release manifest : lookaside.facebook.com/.../file=manifest.json
#         -> artifacts.<platform> = { url, checksum (sha256), size }
#
#     Upstream publishes a sha256 per artifact in that manifest, and the value
#     below was verified to match the downloaded bytes, so the pin is upstream's
#     own integrity claim rather than something we computed unilaterally.
#   * To update: refetch the two manifests above and copy version + checksum.
#     Note the channel endpoint rejects browser-like User-Agent strings with
#     HTTP 400 (curl's default UA and absent UA both work) -- see
#     ai-crawler-site-access-table.md. Nix's fetchurl is unaffected.
#   * The artifact is a fully static, stripped ELF, so there is no interpreter
#     or RPATH to fix: no autoPatchelfHook, no wrapper, no extra runtime deps.
#     `file` reports "statically linked" and `--version` runs as-is.
#   * Self-update is left alone deliberately. The launcher is what normally
#     updates Muse, and we never install it; the bare binary has no writable
#     install dir to update into from /nix/store.
#   * Runtime requires a Meta account sign-in (or META_API_KEY for CI) and a
#     paid plan. Nothing to do at build time, but it means there is no sensible
#     installCheck beyond `--version`, which works offline.

let
  version = "1.4.2-R4684.1";

  # artifacts.<platform> from the release manifest, keyed by Nix system.
  artifacts = {
    "x86_64-linux" = {
      file = "muse-x86-linux";
      hash = "sha256-37MJbJH0dnxNmABkYIALe6kGoLGkCCgKkmqNwZoa9k8=";
    };
    "aarch64-linux" = {
      file = "muse-aarch64-linux";
      hash = "sha256-+ml0wjMHoNXbkTZ1SeQVBaXntV3NZsZy6y3YnBoSWtY=";
    };
    "x86_64-darwin" = {
      file = "muse-x86-macos";
      hash = "sha256-g7IijkDFisN5gJWTbbfteHtW/I/QCBIJaXuMVP1mzoY=";
    };
    "aarch64-darwin" = {
      file = "muse-aarch64-macos";
      hash = "sha256-6Zh+9CZ6ZI3BkxwoNqI/bxb4tEFzaLEmqBC+ziq9MI0=";
    };
  };

  inherit (stdenvNoCC.hostPlatform) system;

  artifact =
    artifacts.${system}
      or (throw "muse-code: no upstream artifact for system ${system}");
in

stdenvNoCC.mkDerivation {
  pname = "muse-code";
  inherit version;

  src = fetchurl {
    # Query-string URL, so name= must be given explicitly.
    name = "muse-code-${version}-${artifact.file}";
    url = "https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=${version}&file=${artifact.file}";
    inherit (artifact) hash;
  };

  dontUnpack = true;
  dontBuild = true;
  # Already stripped upstream, and stripping a static binary risks breaking it.
  dontStrip = true;

  installPhase = ''
    runHook preInstall

    install -Dm755 "$src" "$out/bin/muse"

    runHook postInstall
  '';

  doInstallCheck = true;

  installCheckPhase = ''
    runHook preInstallCheck

    export HOME="$PWD/home"
    mkdir -p "$HOME"
    "$out/bin/muse" --version

    runHook postInstallCheck
  '';

  meta = {
    description = "Meta's coding agent for the terminal and CI, built for Muse Spark";
    homepage = "https://dev.meta.ai/docs/muse-code";
    downloadPage = "https://dev.meta.ai/docs/muse-code";
    # Proprietary binary; requires a Meta account and a paid plan to use.
    license = lib.licenses.unfree;
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    mainProgram = "muse";
    platforms = lib.attrNames artifacts;
  };
}
