{
  lib,
  appimageTools,
  fetchurl,
}:

# Cockpit Tools -- account manager for AI IDEs.
#   https://github.com/jlcodes99/cockpit-tools
#
# This is the "Antigravity Cockpit" thing: a universal account-management tool
# covering Antigravity, Codex, GitHub Copilot, Windsurf, Kiro, Cursor, Gemini
# CLI and others -- switching between accounts and watching per-account quota.
# 18.6k stars, actively pushed.
#
# Related projects by the same author, not packaged here because they are editor
# extensions rather than standalone apps:
#   jlcodes99/vscode-antigravity-cockpit (4.8k stars) -- VS Code extension for
#   monitoring Antigravity. Install from the VS Code marketplace if wanted.
#
# Packaging notes:
#   * Not in nixpkgs. Upstream ships AppImage / rpm / deb; AppImage is the
#     cleanest to wrap, and appimageTools handles the FHS and desktop-entry
#     extraction, so no autoPatchelfHook juggling is needed.
#   * LICENSE: the repository declares none (GitHub reports no SPDX id), so no
#     rights are formally granted. Marked unfree accordingly. Installed at the
#     user's explicit request.
#   * The app is an account/credential manager, so it will want to read and
#     write the credential stores of whatever IDEs it manages.
#
# To update: bump version, then
#   nix store prefetch-file --json <url>

let
  pname = "cockpit-tools";
  version = "1.3.65";

  src = fetchurl {
    url = "https://github.com/jlcodes99/cockpit-tools/releases/download/v${version}/Cockpit.Tools_${version}_amd64.AppImage";
    hash = "sha256-D9DcTaQnhC5vEBZ90iAiRnZoNCfBSwJxjd7uBPg3RU8=";
  };

  appimageContents = appimageTools.extractType2 { inherit pname version src; };
in
appimageTools.wrapType2 {
  inherit pname version src;

  extraInstallCommands = ''
    install -Dm444 ${appimageContents}/*.desktop -t "$out/share/applications" || true
    if [ -d ${appimageContents}/usr/share/icons ]; then
      cp -r ${appimageContents}/usr/share/icons "$out/share/" || true
    fi
    for png in ${appimageContents}/*.png; do
      [ -e "$png" ] || continue
      install -Dm444 "$png" "$out/share/pixmaps/$(basename "$png")"
    done
  '';

  meta = {
    description = "Account manager for AI IDEs (Antigravity, Codex, Copilot, Cursor, Gemini CLI)";
    homepage = "https://github.com/jlcodes99/cockpit-tools";
    # Upstream declares no licence.
    license = lib.licenses.unfree;
    mainProgram = "cockpit-tools";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
