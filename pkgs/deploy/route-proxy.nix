# The OpenCode route proxy as a flake package - issue #5768 (M1).
#
# The proxy script lived loose in bin/ and ran from a hand-written
# systemd unit; the opencode#1153 retirement (e8b4cdc754) deleted
# both, and #5610 recorded that the retirement was premature: the
# deployed OpenCode config still routes opencode-route/* through
# :18750. The script and its test are restored byte-identical from
# e8b4cdc754^ (the exact source the #5610 stopgap ran) so the deploy
# registry has a real package to name. The restored script and test
# are the starting point; the source now adopts fd 3 for systemd
# socket activation (#1822). Its local imports are carried into the
# package alongside it, since the script resolves them relative to
# itself.
{
  runCommand,
  makeWrapper,
  nodejs,
}:
runCommand "opencode-route-proxy-0.1.0"
  {
    nativeBuildInputs = [ makeWrapper ];
    meta = {
      description = "OpenCode route proxy (OpenAI-compatible routing door, :18750)";
      mainProgram = "opencode-route-proxy";
    };
  }
  ''
    mkdir -p $out/bin $out/libexec/opencode-route-proxy
    cp ${../../bin/opencode-route-proxy.mjs} $out/libexec/opencode-route-proxy/opencode-route-proxy.mjs
    cp ${../../bin/proxy-port.mjs} $out/libexec/opencode-route-proxy/proxy-port.mjs
    cp ${../../bin/home-dir.mjs} $out/libexec/opencode-route-proxy/home-dir.mjs
    cp ${../../bin/request-body.mjs} $out/libexec/opencode-route-proxy/request-body.mjs
    makeWrapper ${nodejs}/bin/node $out/bin/opencode-route-proxy \
      --add-flags $out/libexec/opencode-route-proxy/opencode-route-proxy.mjs
  ''
