{
  lib,
  stdenvNoCC,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  libsecret,
  versionCheckHook,
}:

# Jules Tools -- the CLI for Google's Jules async coding agent.
#   https://jules.google
#   Docs: https://jules.google/docs/cli/reference.md
#   SDK:  https://github.com/google-labs-code/jules-sdk
#
# Packaging notes:
#   * Not in nixpkgs (verified: pkgs/by-name/ju/jules and .../jules-cli both
#     absent) and no third-party flake exists.
#   * `npm install -g @google/jules` does NOT work on NixOS. That npm package is
#     a four-file stub whose postinstall downloads this tarball; the payload is a
#     dynamically linked Go binary wanting /lib64/ld-linux-x86-64.so.2, which
#     does not exist here. We skip npm entirely and pin the artifact.
#   * Packaged the same way as nixpkgs' antigravity-cli (also a Google
#     storage.googleapis.com tarball + autoPatchelfHook), so the shape here is
#     deliberately parallel to that derivation.
#   * The bundled dependency list (from licenses/ in the tarball) includes
#     github.com/zalando/go-keyring, so `jules login` wants a DBus Secret
#     Service -- gnome-keyring, KWallet or similar. libsecret is wired in for
#     that. Without a running secret service, login will fail even though the
#     binary runs.
#   * It also bundles github.com/minio/selfupdate. Self-update cannot work
#     against a read-only /nix/store; expect it to fail loudly rather than
#     silently corrupt anything. Update by bumping this file.
#   * Auth alternatives that avoid the keyring entirely: the REST API takes an
#     API key in the `x-goog-api-key` header (JULES_API_KEY by convention),
#     minted at https://jules.google.com/settings -- max 3 keys per account.
#   * Free tier is 15 tasks/day, 3 concurrent. Paid tiers currently only
#     purchasable on individual @gmail.com accounts, not Workspace.
#   * npm publishes this with `"license": null`; upstream ships no LICENSE for
#     the tool itself (only vendored dependency licences), so it is marked
#     unfree-redistributable rather than guessed at.
#
# To update: bump version, then
#   nix store prefetch-file --json \
#     "https://storage.googleapis.com/jules-cli/v<ver>/jules_external_v<ver>_linux_amd64.tar.gz"

let
  version = "0.1.42";

  throwSystem = throw "jules: unsupported system ${stdenvNoCC.hostPlatform.system}";

  sources = {
    x86_64-linux = {
      url = "https://storage.googleapis.com/jules-cli/v${version}/jules_external_v${version}_linux_amd64.tar.gz";
      hash = "sha256-c869LI+Jubsk703MuM15Q8y2npmzfeJnwvV5Mjen0QM=";
    };
  };
in
stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "jules";
  inherit version;

  strictDeps = true;

  src = fetchurl (sources.${stdenvNoCC.hostPlatform.system} or throwSystem);

  # Tarball has no top-level directory: jules, run.cjs, README.md, licenses/.
  sourceRoot = ".";

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
  ];
  buildInputs = [ libsecret ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall

    install -Dm755 jules "$out/bin/jules"
    install -Dm644 README.md "$out/share/doc/jules/README.md"
    cp -r licenses "$out/share/doc/jules/licenses"

    runHook postInstall
  '';

  postInstall = ''
    wrapProgram "$out/bin/jules" \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libsecret ]}
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgramArg = "version";
  doInstallCheck = true;

  meta = {
    description = "CLI for Jules, Google's asynchronous coding agent";
    homepage = "https://jules.google";
    downloadPage = "https://www.npmjs.com/package/@google/jules";
    # Upstream ships no licence for the tool itself and npm reports none.
    license = lib.licenses.unfree;
    mainProgram = "jules";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
})
