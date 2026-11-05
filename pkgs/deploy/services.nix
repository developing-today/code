# Service definitions for the deploy registry — issue #5766, slice 1.
#
# One option set per service, in the shape of the deployment-architecture
# design (doc/2026-11-04_deployment-architecture-services-layer.md §3):
# the repo that owns the code declares the service; renderers (systemd,
# launchd, Cloudflare, container — later slices) consume the option set.
# This file is the code flake's own contribution to the registry; the
# `deployTargets` flake output built from it is where the forks'
# definitions get merged as they grow deploy outputs of their own.
#
# `class` names the activation class of design §4 that renders the
# service (systemd / launchd / cloudflare / sandbox / container).
{ fleet, mcpx }:
{
  queue-broker = {
    package = fleet;
    entryPoint = "bin/fleet";
    args = [
      "serve"
      "queue-broker"
    ];
    environment = {
      PORT = "4801"; # PORT_QUEUE_BROKER, pkgs/fleet/crates/fleet-config
    };
    # Names only — values are resolved by the secrets broker at
    # activation time (later slice); a definition never carries a value.
    secrets = [ "supabase.muse-dev2.service-role-key" ];
    user = "user";
    ports = [ 4801 ];
    restart = "on-failure";
    healthCheck = "http://100.99.113.103:4801/health";
    targets = [ "laptop" ];
    class = "systemd";
  };

  secrets-broker = {
    package = fleet;
    entryPoint = "bin/fleet";
    args = [
      "serve"
      "secrets-broker"
    ];
    environment = {
      PORT = "4800"; # PORT_SECRETS_BROKER, pkgs/fleet/crates/fleet-config
    };
    secrets = [ ]; # the secrets broker is the resolver; it needs none by name
    user = "user";
    ports = [ 4800 ];
    restart = "on-failure";
    healthCheck = "http://100.99.113.103:4800/health";
    targets = [ "laptop" ];
    class = "systemd";
  };

  session-proxy = {
    package = fleet;
    entryPoint = "bin/fleet";
    args = [
      "serve"
      "proxy"
    ];
    environment = { };
    secrets = [ ];
    user = "user";
    ports = [ 3130 ]; # DEFAULT_EGRESS_PROXY_PORT, fleet-config
    restart = "on-failure";
    healthCheck = null;
    targets = [ "sandbox" ];
    class = "sandbox";
  };

  mcpx = {
    package = mcpx;
    entryPoint = "bin/mcpx";
    args = [ ];
    environment = { };
    secrets = [ ];
    user = "user";
    ports = [ ];
    restart = "on-failure";
    healthCheck = null;
    targets = [
      "laptop"
      "darwin"
    ];
    class = "systemd";
  };
}
