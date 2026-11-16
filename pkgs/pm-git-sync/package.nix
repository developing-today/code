# pm-git-sync — keep the primary code clone on a clean, current main,
# packaged from this repo's pm-stack/scripts/pm-git-sync.sh (code#8171).
#
# The script still operates ON the checkout (that is its whole job);
# it just runs from the store like everything else in the stack. Its
# tool dependencies are pinned on the wrapper PATH; PM_GIT_SYNC_REPO
# still overrides the target clone.
{
  lib,
  stdenv,
  makeWrapper,
  git,
  coreutils,
  gnused,
  gnugrep,
}:
stdenv.mkDerivation {
  pname = "pm-git-sync";
  version = "0.2.0";

  src = lib.fileset.toSource {
    root = ../../pm-stack/scripts;
    fileset = ../../pm-stack/scripts/pm-git-sync.sh;
  };

  nativeBuildInputs = [ makeWrapper ];

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 pm-git-sync.sh $out/bin/pm-git-sync
    wrapProgram $out/bin/pm-git-sync \
      --prefix PATH : ${
        lib.makeBinPath [
          git
          coreutils
          gnused
          gnugrep
        ]
      }
    runHook postInstall
  '';

  meta = {
    description = "Keep the primary code clone on a clean, current main (PM stack)";
    mainProgram = "pm-git-sync";
    platforms = lib.platforms.linux;
  };
}
