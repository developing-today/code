# The Roc compiler at the nightly this project pins. The tests and the roc-world
# platform were generated with this tag (tests/cli_integration.rs,
# src/world_compile.rs). The tarballs and hashes are the ones roc-overlay records
# in its sources.json for the same tag, so this is the same compiler, fetched here
# so that pkgs/id does not depend on the root flake's roc input.
{
  lib,
  stdenv,
  fetchurl,
  makeWrapper,
  coreutils,
}:

let
  # Shared with the example build scripts so the executable guard and Nix
  # package cannot silently pin different Roc nightlies.
  tag = lib.removeSuffix "\n" (builtins.readFile ./roc-version);
  tagSuffix = lib.removePrefix "nightly-" tag;

  sources = {
    x86_64-linux = {
      asset = "roc_nightly-linux_x86_64-${tagSuffix}.tar.gz";
      hash = "sha256-iTyG0toKTDkMvb1CWM5F1obl7BW0Lu64Ef0axEU3cU8=";
    };
    aarch64-linux = {
      asset = "roc_nightly-linux_arm64-${tagSuffix}.tar.gz";
      hash = "sha256-9FMtKQyDkg0j8ZHaRlCxmqgwJ0PtNcq06a3IO+rwwds=";
    };
  };

  source =
    sources.${stdenv.hostPlatform.system}
      or (throw "no Roc ${tag} binary for ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "roc";
  version = tag;

  src = fetchurl {
    url = "https://github.com/roc-lang/nightlies/releases/download/${tag}/${source.asset}";
    inherit (source) hash;
  };

  # A prebuilt binary: there is nothing to configure or compile.
  dontConfigure = true;
  dontBuild = true;
  dontStrip = true;

  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall

    install -Dm755 roc "$out/bin/roc"
    install -Dm644 LICENSE "$out/share/licenses/roc/LICENSE"
    install -Dm644 legal_details "$out/share/doc/roc/legal_details"

    runHook postInstall
  '';

  # roc runs cc and coreutils when it links a program, so they go on its PATH.
  postFixup = ''
    wrapProgram "$out/bin/roc" --prefix PATH : ${
      lib.makeBinPath [
        coreutils
        stdenv.cc
      ]
    }
  '';

  passthru = { inherit tag; };

  meta = {
    description = "Official Roc new-compiler nightly ${tag}";
    homepage = "https://github.com/roc-lang/nightlies";
    changelog = "https://github.com/roc-lang/nightlies/releases/tag/${tag}";
    license = lib.licenses.upl;
    mainProgram = "roc";
    platforms = builtins.attrNames sources;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
