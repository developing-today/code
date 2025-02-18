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
  };
  sops.secrets.tailscale_key = {
    # TODO: distinguish between persistent and ephemeral tailscale keys (ephemeral remove from tailnet on shutdown)
    sopsFile = lib.from-root "secrets/sops/common/tailscale.yaml";
  };
}
