{ config, lib, ... }:
{
  imports = [ (lib.from-root "nixos/abstract/tailscale-autoconnect") ];
  services.tailscaleAutoconnect = {
    enable = true;
    authkeyFile = config.sops.secrets.tailscale_key.path;
    loginServer = "https://login.tailscale.com";
    # default login server is controlplane, unsure why we are changing it.
    #exitNode = "some-node-id";
    #exitNodeAllowLanAccess = true;
    acceptRoutes = true;
    # MagicDNS on. This is only safe with systemd-resolved (enabled below): with
    # plain dhcpcd+openresolv tailscaled owned /etc/resolv.conf and answered SERVFAIL
    # for everything whenever its scraped upstream list went empty (hourly Wi-Fi
    # reconnects). With resolved, tailscale registers split-DNS on tailscale0 only and
    # the DHCP nameservers stay on wlp1s0, independent of tailscale.
    acceptDns = true;
  };
  # dhcpcd keeps working: NixOS points `resolvconf` at systemd's shim, which feeds
  # resolved per-interface. dnsmasq (dhcp-nat) uses bind-dynamic on bridge addresses,
  # so it does not clash with the resolved stub on 127.0.0.53.
  services.resolved.enable = true;
  sops.secrets.tailscale_key = {
    # TODO: distinguish between persistent and ephemeral tailscale keys (ephemeral remove from tailnet on shutdown)
    sopsFile = lib.from-root "secrets/sops/common/tailscale.yaml";
  };
}
