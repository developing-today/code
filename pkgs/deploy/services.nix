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
{
  fleet,
  mcpx,
  routeProxy,
  idMailDeploy,
  pkarrDeploy,
  developingTodayBackupDeploy,
}:
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
    healthCheck = "http://100.99.113.103:4801/healthz";
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

  # Cloudflare Workers — issue #5767, phase 1: the three Workers that
  # already deploy through flake apps (`nix run .#<name>-deploy`, see
  # lib/default.nix apps) are absorbed as registry targets. For a
  # cloudflare target the definition's package/entryPoint names the
  # existing deploy app — the one-shot command a later activation slice
  # runs to deploy the Worker — not the Worker bundle itself (the
  # bundle-as-flake-package conversion is a later #5767 phase). The
  # apps' behavior is unchanged: each resolves its Cloudflare
  # credentials impurely at deploy time (sops cloudflare_account_id,
  # Bitwarden via bws, inside its own deploy script), so the registry
  # definition names no secrets of its own. Workers listen on the
  # Cloudflare edge, so there are no local ports and no local health
  # endpoint; `restart = "no"` because a deploy app is a one-shot
  # command, not a daemon.
  id-mail = {
    package = idMailDeploy;
    entryPoint = "bin/id-mail-deploy";
    args = [ ];
    environment = { };
    secrets = [ ]; # resolved by the deploy app itself at deploy time
    user = "user";
    ports = [ ];
    restart = "no";
    healthCheck = null;
    targets = [ "cloudflare" ];
    class = "cloudflare";
  };

  pkarr = {
    package = pkarrDeploy;
    entryPoint = "bin/pkarr-deploy";
    args = [ ];
    environment = { };
    secrets = [ ]; # resolved by the deploy app itself at deploy time
    user = "user";
    ports = [ ];
    restart = "no";
    healthCheck = null;
    targets = [ "cloudflare" ];
    class = "cloudflare";
  };

  developing-today-backup = {
    package = developingTodayBackupDeploy;
    entryPoint = "bin/developing-today-backup-deploy";
    args = [ ];
    environment = { };
    secrets = [ ]; # resolved by the deploy app itself at deploy time
    user = "user";
    ports = [ ];
    restart = "no";
    healthCheck = null;
    targets = [ "cloudflare" ];
    class = "cloudflare";
  };

  # Route proxy - issue #5768, migration M1 of the deployment
  # architecture (design doc/2026-11-04_deployment-architecture-services-layer.md, §7): the
  # hand-written opencode-route-proxy.service unit is already gone
  # and :18750 serves nothing, while the deployed OpenCode config
  # still routes opencode-route/* through it (#5610). This registry
  # entry is the services-layer target definition; the package is the
  # restored proxy script (see route-proxy.nix). Activation - the
  # rendered unit, generations, and rollback in M1's acceptance -
  # waits on the systemd renderer/generation slice, which has not
  # landed; no hand-written unit replaces this entry in the meantime.
  # Whether the proxy itself is retired stays #5610's decision; this
  # entry only moves its deployment mechanism.
  route-proxy = {
    package = routeProxy;
    entryPoint = "bin/opencode-route-proxy";
    args = [ ];
    environment = {
      OPENCODE_ROUTE_PROXY_PORT = "18750";
      OPENCODE_ROUTE_PROXY_HOST = "127.0.0.1";
    };
    secrets = [ ]; # credentials are read by the proxy itself from the OpenCode credential store at runtime
    user = "user";
    ports = [ 18750 ];
    restart = "on-failure";
    # A real request through :18750 (the proxy's own health route),
    # not a process or port check.
    healthCheck = "http://127.0.0.1:18750/health";
    targets = [ "laptop" ];
    class = "systemd";
  };
}
