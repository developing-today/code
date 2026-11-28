# Sandbox link Cloudflare Tunnel (code#10356). Drew's transport
# ruling, 2026-11-15: "stop using tailscale. point the sandbox at
# cloudflare point laptop at cloudflare tunnel." The sandbox <->
# laptop fleet control path (laptop-agent) rides this tunnel as its
# primary transport; the Tailscale session proxy remains the client's
# automatic fallback, not the primary.
#
# A third, separate remotely-managed tunnel named "sandbox-link"
# (722f72ba-b27a-42a8-b06a-9b72ed2f8e8c), distinct from the pm stack's
# "amd-6-pm-stack" and the lab's "lab-cluster" tunnels, so its
# ingress, credentials, and connector lifecycle never entangle with
# theirs. The connector pattern is lab-tunnel's, verbatim in shape:
# home-manager user service, cloudflared from this flake's nixpkgs,
# token from an operator-rendered file, --protocol http2 (the QUIC
# default kept dropping on this host's network path — #5647).
#
# Ingress is managed in Cloudflare (config source: cloudflare), not
# here, exactly like the other tunnels:
#   sandbox-link.security.cab -> http://100.99.113.103:4799
#     (the laptop-agent daemon, which binds the tailnet IP only; the
#     connector dials it locally. The hostname is behind Cloudflare
#     Access with a service-token policy — the sandbox client sends
#     the Access headers plus the daemon's own X-Agent-Token.)
#   relay.security.cab        -> http://localhost:3340
#     (the stock iroh relay from the code#10391 probes; a policy
#     choice, kept on this tunnel's config — do not drop the row
#     when editing ingress.)
#   (catch-all)               -> http_status:404
# DNS: CNAME sandbox-link.security.cab ->
# 722f72ba-...cfargotunnel.com (proxied) in the security.cab zone.
#
# Runtime state + the secret stay on the machine, never in this repo:
# ~/.cloudflared-sandbox-link/tunnel.token (mode 600) carries the raw
# tunnel token, rendered by the operator from BWS
# (cloudflare.tunnel.sandbox-link-token) and read by cloudflared via
# --token-file (#8933: the token never enters the connector's argv).
#
# Until the switch that first evaluates this module, an interim user
# unit of the same name in ~/.local/share/systemd/user/ runs the
# connector; this module's unit shadows it by systemd unit search
# order the moment it lands, and the interim file can then be
# removed.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.sandbox-link-tunnel;
  cloudflared = "${pkgs.cloudflared}/bin/cloudflared";

  # The port cloudflared serves its metrics + /ready endpoint on
  # (loopback only; pm-tunnel uses 20242, lab-tunnel 20243). The
  # watchdog below reads /ready from it, mirroring pm-tunnel's #6992
  # recovery path (#8933).
  tunnelMetricsPort = 20244;

  tunnelWatchdog = pkgs.writeShellScript "sandbox-link-tunnel-watchdog" (
    lib.replaceStrings
      [ "@curl@" "@systemctl@" "@readyUrl@" ]
      [
        "${pkgs.curl}/bin/curl"
        "/run/current-system/sw/bin/systemctl"
        "http://127.0.0.1:${toString tunnelMetricsPort}/ready"
      ]
      (builtins.readFile ./sandbox-link-tunnel-watchdog.sh)
  );
  home = "/home/${cfg.user}";
  stateDir = "${home}/.cloudflared-sandbox-link";
in
{
  options.services.sandbox-link-tunnel = {
    enable = lib.mkEnableOption "the sandbox link Cloudflare Tunnel connector as a NixOS-managed user service";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the sandbox link tunnel services.";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services = {
        sandbox-link-tunnel = {
          Unit = {
            Description = "Cloudflare Tunnel connector for the sandbox link — sandbox-link.security.cab ingress to the laptop-agent daemon (NixOS)";
            After = [ "network-online.target" ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            # tunnel.token is operator-rendered from BWS
            # (cloudflare.tunnel.sandbox-link-token) as a raw-token
            # file; cloudflared reads it via --token-file so the
            # token stays out of the process argv (#8933).
            # --protocol http2 (#5647): the default QUIC transport
            # kept dropping on this host's network path.
            ExecStart = "${cloudflared} tunnel --no-autoupdate --protocol http2 --metrics 127.0.0.1:${toString tunnelMetricsPort} run --token-file ${stateDir}/tunnel.token";
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        sandbox-link-tunnel-watchdog = {
          Unit = {
            Description = "sandbox-link-tunnel watchdog — restart sandbox-link-tunnel when its edge connections stay down (#8933, mirroring pm-tunnel #6992)";
            After = [ "sandbox-link-tunnel.service" ];
          };
          Service = {
            Type = "oneshot";
            ExecStart = "${tunnelWatchdog}";
          };
        };
      };

      systemd.user.timers = {
        sandbox-link-tunnel-watchdog = {
          Unit.Description = "Check sandbox-link-tunnel edge readiness every 2 minutes (#8933)";
          Timer = {
            OnBootSec = "2min";
            OnUnitActiveSec = "2min";
            Unit = "sandbox-link-tunnel-watchdog.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };
      };
    };
  };
}
