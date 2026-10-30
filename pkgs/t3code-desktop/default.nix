{
  lib,
  appimageTools,
  fetchurl,
}:

# T3 Code desktop -- the Electron GUI for T3 Code.
#   https://t3.codes  /  github:pingdotgg/t3code
#
# This is the client half of T3 Code: it connects to a T3 Code server (the
# `t3` CLI/server in pkgs/latest-cli, run headless by
# systemd.user.services.t3code) and provides the desktop UI for its threads,
# providers and orchestrator. The server tarball does not contain it; upstream
# publishes it as a separate release asset (.deb / .AppImage).
#
# Packaging notes:
#   * The AppImage is wrapped with appimageTools.wrapType2, which handles the
#     FHS environment the Electron binary expects; the .deb would need the
#     same treatment by hand. This replaces the nixpkgs `t3code` desktop,
#     which was still on the 0.0.45 (V1) line.
#   * The version MUST match the server in pkgs/latest-cli: 0.1.2 is
#     Orchestrator V2, and a V1 app refuses a V2 server and vice versa.
#     Bump the two together, and note the mobile app must likewise be
#     T3 Code 2.0 for a V2 server.
#   * The AppImage's binary is named `t3code`; pname is "t3code-desktop" so
#     the wrapped executable is $out/bin/t3code-desktop, matching the
#     mainProgram the fleet has always used for this app.
#   * Upstream is MIT (changed from the unfree marking the server entry
#     previously carried in error).
#
# To update: bump version with pkgs/latest-cli's t3, then
#   nix store prefetch-file --json <url>

let
  pname = "t3code-desktop";
  version = "0.1.2";

  src = fetchurl {
    url = "https://github.com/pingdotgg/t3code/releases/download/v${version}/T3-Code-${version}-x86_64.AppImage";
    hash = "sha256-zwHi0Wfv+dAcTBAE0xssDaniUizJQmLkFkTRyxXf21w=";
  };

  appimageContents = appimageTools.extractType2 { inherit pname version src; };
in
appimageTools.wrapType2 {
  inherit pname version src;

  extraInstallCommands = ''
    install -Dm444 ${appimageContents}/t3code.desktop -t "$out/share/applications"
    # The upstream entry launches via AppRun; point it at the wrapped binary.
    sed -i 's|^Exec=AppRun|Exec=t3code-desktop|' "$out/share/applications/t3code.desktop"
    cp -r ${appimageContents}/usr/share/icons "$out/share/"
    install -Dm444 ${appimageContents}/t3code.png "$out/share/pixmaps/t3code.png"
  '';

  meta = {
    description = "T3 Code desktop app -- GUI client for the T3 Code server";
    homepage = "https://t3.codes";
    license = lib.licenses.mit;
    mainProgram = "t3code-desktop";
    platforms = [ "x86_64-linux" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
