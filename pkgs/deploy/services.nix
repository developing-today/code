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
  laptopAgent,
  mcpx,
  routeProxy,
  quotaRecorder,
  idMailDeploy,
  pkarrDeploy,
  developingTodayBackupDeploy,
  coldStoreRehydratorDeploy,
  labOpsHubDeploy,
  cfContainerHelloDeploy,
  queueTransportAlertDeploy,
  messageBridgeDeploy,
  agentInboxDeploy,
  webhookGatewayDeploy,
  webhookGatewaySinkDeploy,
  pmAggregator,
  pmProjector,
  pmTunnel,
  readyForAgent,
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
      # Live since the Turso cutover (#6741): matches the
      # 30-turso-queue.conf drop-in the running unit carries.
      QUEUE_BROKER_BACKEND = "turso";
    };
    # Names only — values are resolved by the secrets broker at
    # activation time (later slice); a definition never carries a value.
    secrets = [
      "turso.fleet-queue-rw-token" # the Turso backend token, by name
    ];
    user = "user";
    ports = [ 4801 ];
    restart = "on-failure";
    healthCheck = "http://100.99.113.103:4801/healthz";
    # Release attestation (release-workflow design §3 part 2,
    # #13306): the fleet app's own test workflow sits outside the
    # main-gate watched set, so the release verb requires it as
    # per-app evidence.
    evidenceWorkflow = "fleet-rust-tests";
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
    # Release attestation (release-workflow design §3 part 2,
    # #13306): the fleet app's own test workflow sits outside the
    # main-gate watched set, so the release verb requires it as
    # per-app evidence.
    evidenceWorkflow = "fleet-rust-tests";
    targets = [ "laptop" ];
    class = "systemd";
  };

  laptop-agent = {
    package = laptopAgent;
    entryPoint = "bin/laptop-agent";
    args = [ ];
    environment = {
      LAPTOP_AGENT_HOST = "100.99.113.103";
      LAPTOP_AGENT_PORT = "4799";
    };
    # The daemon token is a local file (~/.config/laptop-agent/token)
    # the daemon reads itself at startup; no broker-resolved secrets.
    secrets = [ ];
    user = "user";
    ports = [ 4799 ];
    # The hand-written unit this replaces ran Restart=always: the
    # 2026-11-16 daemon-death episode (code#2879) showed on-failure
    # does not restart a clean (SIGTERM/success) exit.
    restart = "always";
    # No unauthenticated health endpoint exists: /health requires the
    # daemon token (401 without it), which the deploy probe cannot
    # present, so healthCheck stays null (the session-proxy/mcpx
    # shape) and health is verified through the authenticated client
    # (pkgs/fleet/laptop-agent.py, `laptop-agent.py health`) after
    # activation - issue #5769. That script is the client only; the
    # daemon deployed here is the laptopAgent package above
    # (pkgs/laptop-agent, the reconciled source of #10131).
    healthCheck = null;
    targets = [ "laptop" ];
    class = "systemd";
  };

  # Sandbox quota recorder - issue #5770, migration M3 of the
  # deployment architecture (design §4.4/§7): the first sandbox-class
  # target. The recorder had no installer of record — its units lived
  # as templates in the sandbox's ~/tools and were re-copied by hand
  # after every recycle. As a target, its generation lives in the
  # sandbox's home profile and the generic restorer (deploy.py slice
  # 4 / restore.py) links its rendered units into /etc/systemd/system
  # on every boot. The cadence is the incumbent template's: a
  # 5-minute oneshot under a persistent timer — expressed with the
  # optional `timer` field slice 4 adds to the §3 shape (the shape
  # had no way to name a timer-driven service). The data path is
  # untouched (file-first, Turso landing per #5690; #5703's
  # boundary): the recorder resolves its DB token by name at flush
  # time, so the name is declared here and no value is carried.
  quota-recorder = {
    package = quotaRecorder;
    entryPoint = "bin/quota-recorder";
    args = [ ];
    environment = {
      # The incumbent unit's only environment: the recorder resolves
      # its output paths from HOME (the sandbox user's home, not
      # root's), exactly as the hand-copied unit ran it.
      HOME = "/home/hatch";
    };
    secrets = [ "turso.fleet-platform-rw-token" ];
    user = "root"; # sandbox system units run as root; there is no user bus
    ports = [ ];
    restart = "no"; # a oneshot under its timer, not a daemon
    healthCheck = null; # no HTTP surface; activation verifies the timer is active
    targets = [ "sandbox" ];
    class = "sandbox";
    timer = {
      onCalendar = "*:0/5";
      persistent = true;
      randomizedDelaySec = 20;
    };
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

  # Issue #5767, phase 2: the remaining in-repo Workers from the
  # issue's target list, registered behind the deploy CLI. Each
  # deploy app runs its Worker's flake package (bundle / Worker tree
  # with lockfile-pinned deps — no deploy-time npm install) and
  # gathers its Cloudflare credentials impurely at deploy time, so
  # the definitions name no secrets of their own (the phase-1 shape).
  cold-store-rehydrator = {
    package = coldStoreRehydratorDeploy;
    entryPoint = "bin/cold-store-rehydrator-deploy";
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

  lab-ops-hub = {
    package = labOpsHubDeploy;
    entryPoint = "bin/lab-ops-hub-deploy";
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

  cf-container-hello = {
    package = cfContainerHelloDeploy;
    entryPoint = "bin/cf-container-hello-deploy";
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

  # code#6369: probes the broker's public route every 10 minutes from a
  # Cloudflare cron and files/closes a GitHub incident on an outage.
  queue-transport-alert = {
    package = queueTransportAlertDeploy;
    entryPoint = "bin/queue-transport-alert-deploy";
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

  # Remote MCP connector for Claude that sends and receives messages,
  # with a history page; one SQLite Durable Object holds the messages.
  message-bridge = {
    package = messageBridgeDeploy;
    entryPoint = "bin/message-bridge-deploy";
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

  # Issue #5767, external-Worker slice: agent-inbox, webhook-gateway,
  # and webhook-gateway-sink live in dezren39/hatch-personal (source
  # verdict comment 6974159268), not this repo. Each deploy app stages
  # its worker dir from the pinned hatch-personal rev at deploy time
  # (sparse checkout) and runs wrangler deploy there; both workers are
  # dependency-free, so there is no build and no deploy-time npm
  # install. Credentials are gathered impurely at deploy time, so the
  # definitions name no secrets (the phase-1 shape). The pinned rev is
  # repeated in `environment` so `deploy list` shows the source pin
  # next to the target. Deploy the sink before the gateway on a fresh
  # account (the gateway binds to the SINK service).
  agent-inbox = {
    package = agentInboxDeploy;
    entryPoint = "bin/agent-inbox-deploy";
    args = [ ];
    environment = {
      HATCH_PERSONAL_REPO = "dezren39/hatch-personal";
      HATCH_PERSONAL_REV = "ae79c332a78a7ba39b6dacd0f30228384d58dfdc";
      HATCH_WORKER_PATH = "workspace/agent-inbox/worker";
    };
    secrets = [ ]; # resolved by the deploy app itself at deploy time
    user = "user";
    ports = [ ];
    restart = "no";
    healthCheck = null;
    targets = [ "cloudflare" ];
    class = "cloudflare";
  };

  webhook-gateway = {
    package = webhookGatewayDeploy;
    entryPoint = "bin/webhook-gateway-deploy";
    args = [ ];
    environment = {
      HATCH_PERSONAL_REPO = "dezren39/hatch-personal";
      HATCH_PERSONAL_REV = "ae79c332a78a7ba39b6dacd0f30228384d58dfdc";
      HATCH_WORKER_PATH = "workspace/webhook-gateway";
    };
    secrets = [ ]; # resolved by the deploy app itself at deploy time
    user = "user";
    ports = [ ];
    restart = "no";
    healthCheck = null;
    targets = [ "cloudflare" ];
    class = "cloudflare";
  };

  webhook-gateway-sink = {
    package = webhookGatewaySinkDeploy;
    entryPoint = "bin/webhook-gateway-sink-deploy";
    args = [ ];
    environment = {
      HATCH_PERSONAL_REPO = "dezren39/hatch-personal";
      HATCH_PERSONAL_REV = "ae79c332a78a7ba39b6dacd0f30228384d58dfdc";
      HATCH_WORKER_PATH = "workspace/webhook-gateway/sink";
    };
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

  # PM stack - issue #12768: the aggregator, projector, and tunnel
  # join the registry. Their units of record today live in the NixOS
  # pm-stack module (nixos/services/pm-stack), which callPackages the
  # same packages and carries the services' full runtime environment:
  # the state .env files under ~/.local/state/pm-aggregator and
  # ~/.local/state/pm-projector (mode 600, rendered from the pm-stack
  # SOPS file; credentials carried BY NAME there) and the tunnel token
  # file rendered by the pm-kaneo deploy flow. These definitions are
  # registration only - the activation cutover is the bundled slice
  # Drew ruled on 2026-11-22, and it must port the module's full
  # Environment/EnvironmentFile set before any of these targets is
  # activated through the deploy CLI.
  pm-aggregator = {
    package = pmAggregator;
    entryPoint = "bin/pm-aggregator";
    args = [ ];
    environment = {
      PM_AGG_PORT = "4870"; # services.pm-stack.aggregatorPort default
      PM_PROJECTOR_URL = "http://127.0.0.1:7801";
      # The package bundles kaneo-projects.json; the module pins the
      # reader at the bundled copy the same way.
      PM_KANEO_PROJECTS_FILE = "${pmAggregator}/libexec/pm-aggregator/kaneo-projects.json";
    };
    # Credentials arrive via the state .env (the unit's
    # EnvironmentFile) and the BWS token file the app reads itself -
    # no broker-resolved secret names, the laptop-agent shape.
    secrets = [ ];
    user = "user";
    ports = [
      4870 # dashboard HTTP
      4871 # freshness /publish IPC, loopback
    ];
    restart = "on-failure";
    healthCheck = "http://127.0.0.1:4870/health";
    targets = [ "laptop" ];
    class = "systemd";
  };

  pm-projector = {
    package = pmProjector;
    entryPoint = "bin/pm-projector";
    args = [ ];
    environment = {
      PORT = "7801"; # services.pm-stack.projectorPort default
    };
    # DATABASE_URL and PM_PROJECTOR_API_TOKEN arrive via the state
    # .env (the unit's EnvironmentFile), not broker-resolved names.
    secrets = [ ];
    user = "user";
    ports = [ 7801 ];
    restart = "on-failure";
    healthCheck = "http://127.0.0.1:7801/health";
    targets = [ "laptop" ];
    class = "systemd";
  };

  pm-tunnel = {
    package = pmTunnel; # pkgs.cloudflared, the package the pm-stack module runs
    entryPoint = "bin/cloudflared";
    args = [
      "tunnel"
      "--no-autoupdate"
      # --protocol http2 (#5647): QUIC kept dropping on this host's
      # network path; HTTP/2 runs over TCP.
      "--protocol"
      "http2"
      "--metrics"
      "127.0.0.1:20242"
      "run"
      # The token is read from the file by cloudflared itself
      # (#8933: it never enters the process argv). The file is
      # rendered by the pm-kaneo deploy flow (pm-stack/deploy.sh)
      # into the deployment dir, the module's stackDir.
      "--token-file"
      "/home/user/pm-stack/tunnel.token"
    ];
    environment = { };
    secrets = [ ]; # the tunnel token arrives via --token-file, above
    user = "user";
    ports = [ 20242 ]; # cloudflared metrics + /ready, loopback
    restart = "on-failure";
    # cloudflared's own /ready: 200 = at least one live edge
    # connection (the pm-tunnel watchdog reads the same endpoint).
    healthCheck = "http://127.0.0.1:20242/ready";
    targets = [ "laptop" ];
    class = "systemd";
  };

  # ready-for-agent (#12909): the Harness UI + operator CLI. Its
  # deployment of record is still the NixOS pm-apps module under
  # #1752, whose unit runs the imperative ~/.local install
  # (pm-stack/ready-for-agent/install.sh) — this definition is
  # registration only, like the other pm-stack targets above, until
  # the bundled cutover (#12768) switches the unit's ExecStart to
  # this derivation.
  ready-for-agent = {
    package = readyForAgent;
    entryPoint = "bin/ready-for-agent";
    args = [
      "start"
      "--no-open"
    ];
    environment = { };
    secrets = [ ]; # the harness reads its own on-disk config; no broker-resolved names
    user = "user";
    ports = [ 6056 ]; # Harness UI + GraphQL, loopback
    restart = "on-failure";
    healthCheck = "http://127.0.0.1:6056/";
    targets = [ "laptop" ];
    class = "systemd";
  };
}
