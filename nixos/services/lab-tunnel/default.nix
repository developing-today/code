# Lab cluster Cloudflare Tunnel (code#6455, under the cluster program
# #1817 / network fabric #1084; egress context #5030). Drew approved
# 2026-11-06: "we've used tunnels here before. go ahead. Use CF
# versions and/or nix flake to handle deploy / service."
#
# A second, separate remotely-managed tunnel named "lab-cluster"
# (9302c849-4a4c-41eb-a1bb-5b6bac60ac6b), distinct from the pm stack's
# "amd-6-pm-stack" tunnel in nixos/services/pm-stack — the lab cluster
# gets its own tunnel so its ingress, credentials, and connector
# lifecycle never entangle with the PM stack's. The connector pattern
# is pm-tunnel's, verbatim in shape: home-manager user service,
# cloudflared from this flake's nixpkgs (CF's published build), token
# from an operator-rendered EnvironmentFile, --protocol http2 (the
# QUIC default kept dropping on this host's network path — #5647).
#
# Ingress is managed in Cloudflare (config source: cloudflare), not
# here, exactly like pm-tunnel:
#   lab.security.cab -> http://localhost:8790  (lab-edge-status, below)
#   (catch-all)      -> http_status:404
# DNS: CNAME lab.security.cab -> 9302c849-...cfargotunnel.com (proxied)
# in the security.cab zone. Future lab services (Cozystack dashboard,
# the BMC/KVM broker from the ideas log) join by adding an ingress row
# in Cloudflare + a service here; real lab UIs get Cloudflare Access
# in front when they land — the seed status page below is
# non-sensitive and public, like kaneo/dispatch on the pm tunnel.
#
# The lab hardware is not stood up yet (#1086 waits on the physical
# bring-up under #6066), so the first connector runs on amd — the
# always-on host that already runs the pm-stack connector. The
# redundant second connector (same tunnel, second host — Cloudflare
# load-balances connectors on one tunnel automatically) lands with
# the first lab node and is tracked on #6455.
#
# Runtime state + the secret stay on the machine, never in this repo:
# ~/lab-tunnel/tunnel.token (mode 600) carries the raw tunnel token,
# rendered by the operator from BWS (cloudflare.tunnel.lab-cluster-token)
# and read by cloudflared via --token-file (#8933: the token never
# enters the connector's argv) — the same arrangement as
# ~/pm-stack/tunnel.token for pm-tunnel.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.lab-tunnel;
  cloudflared = "${pkgs.cloudflared}/bin/cloudflared";
  python3 = "${pkgs.python3}/bin/python3";

  # The port cloudflared serves its metrics + /ready endpoint on
  # (loopback only; pm-tunnel uses 20242). The watchdog below reads
  # /ready from it, mirroring pm-tunnel's #6992 recovery path (#8933).
  tunnelMetricsPort = 20243;

  tunnelWatchdog = pkgs.writeShellScript "lab-tunnel-watchdog" (
    lib.replaceStrings
      [ "@curl@" "@systemctl@" "@readyUrl@" ]
      [
        "${pkgs.curl}/bin/curl"
        "/run/current-system/sw/bin/systemctl"
        "http://127.0.0.1:${toString tunnelMetricsPort}/ready"
      ]
      (builtins.readFile ./lab-tunnel-watchdog.sh)
  );
  home = "/home/${cfg.user}";
  stateDir = "${home}/lab-tunnel";
  statusDir = pkgs.writeTextDir "index.html" ''
    <!doctype html>
    <title>lab cluster edge</title>
    <h1>lab cluster edge</h1>
    <p>lab-edge-status:ok</p>
    <p>Tunnel <code>lab-cluster</code> (9302c849-4a4c-41eb-a1bb-5b6bac60ac6b)
    is live. Seed endpoint only — Cozystack and the BMC broker join
    this tunnel as the lab hardware lands (code#6455).</p>
  '';
in
{
  options.services.lab-tunnel = {
    enable = lib.mkEnableOption "the lab cluster Cloudflare Tunnel connector + edge status service as NixOS-managed user services";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the lab tunnel services.";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services = {
        lab-edge-status = {
          Unit = {
            Description = "Lab cluster edge status page — seed service behind lab.security.cab (NixOS)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            ExecStart = "${python3} -m http.server 8790 --bind 127.0.0.1 --directory ${statusDir}";
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        lab-tunnel = {
          Unit = {
            Description = "Cloudflare Tunnel connector for the lab cluster — lab.security.cab ingress (NixOS)";
            After = [ "network-online.target" ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            # tunnel.token is operator-rendered from BWS
            # (cloudflare.tunnel.lab-cluster-token) as a raw-token
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

        lab-tunnel-watchdog = {
          Unit = {
            Description = "lab-tunnel watchdog — restart lab-tunnel when its edge connections stay down (#8933, mirroring pm-tunnel #6992)";
            After = [ "lab-tunnel.service" ];
          };
          Service = {
            Type = "oneshot";
            ExecStart = "${tunnelWatchdog}";
          };
        };
      };

      systemd.user.timers = {
        lab-tunnel-watchdog = {
          Unit.Description = "Check lab-tunnel edge readiness every 2 minutes (#8933)";
          Timer = {
            OnBootSec = "2min";
            OnUnitActiveSec = "2min";
            Unit = "lab-tunnel-watchdog.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };
      };
    };
  };
}
