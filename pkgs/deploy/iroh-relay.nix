# Self-hosted iroh relay binary + tunnel-mode config - issue #14719.
#
# nixpkgs' iroh-relay takes its config at a path, and the generic
# systemd renderer only runs the package's entryPoint with fixed args,
# so the tunnel-mode config from the relay kit
# (pkgs/id/tests/fixtures/relay, pinned by relay_ops.rs) rides inside
# this derivation at the kit unit's config path. No secrets, no
# hostnames: tunnel.toml carries only loopback bind addresses.
{
  runCommand,
  makeWrapper,
  irohRelayPkg,
}:
runCommand "iroh-relay-tunnel-0.1.0"
  {
    nativeBuildInputs = [ makeWrapper ];
    meta = {
      description = "Self-hosted iroh relay, Cloudflare Tunnel mode (fleet transport pilot)";
      mainProgram = "iroh-relay";
    };
  }
  ''
    mkdir -p $out/bin $out/etc/iroh-relay
    cp ${../id/tests/fixtures/relay/tunnel.toml} $out/etc/iroh-relay/relay.toml
    makeWrapper ${irohRelayPkg}/bin/iroh-relay $out/bin/iroh-relay \
      --add-flags --config-path \
      --add-flags "$out/etc/iroh-relay/relay.toml"
  ''
