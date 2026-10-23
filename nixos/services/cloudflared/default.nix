{ config, lib, ... }:
{
  # Opt-in profile, intentionally NOT referenced from any host's `profiles`
  # list yet (see nixos/hosts/default.nix) -- it is inert until added there.
  #
  # Cloudflare Tunnel requires an account-side resource (a tunnel UUID plus a
  # credentials.json minted for it) that only your own browser-authenticated
  # `cloudflared` CLI can create. Nothing here can fabricate that. One-time
  # setup, once `cloudflared` is on PATH (already added to
  # home/common/default.nix -- run a rebuild first):
  #
  #   1. cloudflared tunnel login
  #        -> opens a browser, writes ~/.cloudflared/cert.pem
  #   2. cloudflared tunnel create <name>
  #        -> prints a tunnel UUID, writes ~/.cloudflared/<UUID>.json
  #   3. sops secrets/sops/common/cloudflared.yaml
  #        -> (creates the file on first run) add a key:
  #           cloudflared_tunnel_credentials: '<paste the full contents of
  #           ~/.cloudflared/<UUID>.json here, as a one-line JSON string>'
  #   4. Below: replace REPLACE_WITH_TUNNEL_UUID with the UUID from step 2,
  #      and fill in `ingress` with the hostname(s)/local service(s) this
  #      tunnel should proxy to.
  #   5. Add "services/cloudflared" to the target host's `profiles` list in
  #      nixos/hosts/default.nix.
  #
  # See nixos/tailscale-autoconnect/default.nix for the same sops-secret
  # pattern used elsewhere in this repo.
  sops.secrets.cloudflared_tunnel_credentials = {
    sopsFile = lib.from-root "secrets/sops/common/cloudflared.yaml";
  };

  services.cloudflared = {
    enable = true;
    tunnels."REPLACE_WITH_TUNNEL_UUID" = {
      credentialsFile = config.sops.secrets.cloudflared_tunnel_credentials.path;
      default = "http_status:404";
      ingress = {
        # "example.yourdomain.com" = "http://localhost:8080";
      };
    };
  };
}
