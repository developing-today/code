# PM stack (Drew, 2026-10-27: "make nixos switch deploy all these
# components like everything else. enable projector and all the other
# features."; Drew, 2026-10-30: "make pm stack build run with nixos
# switch" — pm-dispatch, pm-tunnel, and the Kaneo stack join the module
# below, so one switch deploys the whole pm stack.)
#
# The project-management components for this host, declared as
# home-manager user services so `nixos-rebuild switch` deploys them like
# everything else on this machine — replacing the hand-written units
# that used to live in ~/.config/systemd/user (pm-aggregator,
# pm-projector, pm-git-sync, quota-gatherer, and — until the 2026-10-30
# extension — pm-dispatch and pm-tunnel; the pre-NixOS files are kept
# beside this history in git, and home-manager renames any same-named
# hand file to *.backup on the first switch that lands this module).
# The fleet-feed collector joins 2026-11-01 (code#3363): it moved from
# the sandbox to this host on 2026-10-31 as hand-written units and is
# folded in below like the rest.
# The Kaneo docker stack had no hand unit at all: it only ran when
# someone ran pm-stack/deploy.sh by hand, and stayed down otherwise.
#
# Code source (2026-11-10, code#8171 — Drew: "each thing should be
# built from code into a nix app"): the projector, aggregator,
# dispatch, and git-sync are Nix packages built from this repo
# (pkgs/pm-projector, pkgs/pm-aggregator, pkgs/pm-dispatch,
# pkgs/pm-git-sync) and the units below run their store wrappers.
# Code updates deploy by merge -> nixos-rebuild switch, like
# everything else on this machine. The checkout-run model this
# replaces (units `bun run`ning straight out of ~/git/code, pm-git-sync
# restarting services when a subtree pull landed, dependency steps
# improvised at deploy time) is retired: it is what left pm-projector
# crash-looping on a missing node_modules when a restart landed
# without an install (#8171). pm-git-sync remains, but only as clone
# hygiene — it keeps the primary checkout on a clean, current main
# for the tooling that still works in the tree, and for the one unit
# that stays checkout-coupled by design (fleet-feed-collector, below).
#
# Runtime state + secrets stay on the machine, never in this repo:
# ~/.local/state/pm-aggregator/.env and ~/.local/state/pm-projector/.env
# (mode 600, rendered from the pm-stack SOPS file; they carry the
# PM_AGG_*_FILE paths, Kaneo/Grist credentials, and the GitHub App
# credentials BY NAME, exactly as the hand units consumed them).
# pm-dispatch's env files are external the same way:
# ~/pm-stack/dispatch/.env (rendered by pm-stack/dispatch/install.sh
# from the same SOPS file — it carries the GitHub App material and the
# fleet Supabase config: supabase_url + supabase_service_role_key for
# muse-dev2) and ~/secrets/dispatch.env (operator-only secrets that
# are not in SOPS: DISPATCH_API_TOKEN). install.sh remains the env
# renderer and the fallback installer; this module owns the unit
# lifecycle.
#
# The Kaneo stack is the exception to the checkout model: pm-kaneo
# runs deploy.sh from the ~/pm-stack deployment dir, because the
# rendered .env, the compose project, and the pm-stack_postgres_data
# volume live there. deploy.sh also renders ~/pm-stack/tunnel.token,
# which pm-tunnel consumes via cloudflared's --token-file flag
# (#8933: the token never enters the tunnel's ExecStart argv; the
# older tunnel.env rendering is kept for rollback only). (The tunnel's ingress is managed in
# Cloudflare: kaneo.security.cab -> http://localhost:5173 and
# dispatch.security.cab -> http://localhost:4879.)
#
# The projector is ENABLED: the aggregator's environment pins
# PM_PROJECTOR_URL to the local projector service (the env file said
# "off" from the install.sh era, when no projector had ever been
# deployed; aggregator/install.sh no longer renders it off either).
# A 15-minute reconcile timer keeps the projection fresh from the
# GitHub API (the projector's webhook intake needs a public route,
# which this stack deliberately does not have; reconcile is its
# designed self-heal path).
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.pm-stack;
  bun = "${pkgs.bun}/bin/bun";
  cloudflared = "${pkgs.cloudflared}/bin/cloudflared";
  docker = "/run/current-system/sw/bin/docker";
  home = "/home/${cfg.user}";
  repo = "${home}/git/code";
  aggState = "${home}/.local/state/pm-aggregator";
  projState = "${home}/.local/state/pm-projector";
  stackDir = "${home}/pm-stack";

  pmProjector = pkgs.callPackage ../../../pkgs/pm-projector/package.nix { };
  pmAggregator = pkgs.callPackage ../../../pkgs/pm-aggregator/package.nix { };
  pmDispatch = pkgs.callPackage ../../../pkgs/pm-dispatch/package.nix { };
  pmGitSync = pkgs.callPackage ../../../pkgs/pm-git-sync/package.nix { };

  projectorReconcile = pkgs.writeShellScript "pm-projector-reconcile" ''
    # PM_PROJECTOR_API_TOKEN arrives via the unit's EnvironmentFile.
    exec ${pkgs.curl}/bin/curl -fsS -X POST \
      -H "Authorization: Bearer $PM_PROJECTOR_API_TOKEN" \
      http://127.0.0.1:${toString cfg.projectorPort}/reconcile
  '';

  # The port cloudflared serves its metrics + /ready endpoint on
  # (loopback only). The watchdog below reads /ready from it (#6992).
  tunnelMetricsPort = 20242;

  waitForTailnet = pkgs.writeShellScript "pm-wait-for-tailnet" ''
    # pm-dispatch binds the tailnet address (DISPATCH_HOSTS); at boot
    # that address does not exist until tailscaled assigns it, and the
    # server dies EADDRNOTAVAIL (#6992). Wait for tailscale0 to carry
    # an IPv4 address instead of crash-looping on Restart=. On timeout
    # start anyway: the loopback bind (what the tunnel uses) does not
    # need the tailnet, and Restart=on-failure stays the backstop.
    for i in $(seq 1 90); do
      if ${pkgs.iproute2}/bin/ip -4 addr show tailscale0 2>/dev/null | grep -q 'inet '; then
        exit 0
      fi
      sleep 2
    done
    echo "pm-wait-for-tailnet: tailscale0 has no IPv4 address after 180s; starting anyway" >&2
  '';

  waitForDns = pkgs.writeShellScript "pm-wait-for-dns" ''
    # cloudflared discovers the edge through DNS at start; when the
    # local stub resolver is not ready yet it cannot register and the
    # edge answers 502 for every ingress (#6992). Wait until the edge
    # hostname resolves. On timeout start anyway (cloudflared keeps
    # retrying internally; the watchdog below covers the rest).
    for i in $(seq 1 90); do
      if ${pkgs.glibc.getent}/bin/getent hosts region1.v2.argotunnel.com >/dev/null 2>&1; then
        exit 0
      fi
      sleep 2
    done
    echo "pm-wait-for-dns: region1.v2.argotunnel.com unresolvable after 180s; starting anyway" >&2
  '';

  tunnelWatchdog = pkgs.writeShellScript "pm-tunnel-watchdog" ''
    # pm-tunnel can stay process-alive with no live edge connections:
    # exactly the state in which the edge answered 502 for days while
    # pm-dispatch was healthy (#6992). cloudflared's own /ready
    # endpoint is the truth (200 = at least one live edge connection).
    # Three consecutive failures (the timer runs every 2 minutes)
    # restart pm-tunnel; any success resets the counter.
    stateDir="$HOME/.local/state/pm-tunnel-watchdog"
    mkdir -p "$stateDir"
    countFile="$stateDir/consecutive-failures"
    count=0
    if [ -f "$countFile" ]; then
      count=$(cat "$countFile")
    fi
    if ${pkgs.curl}/bin/curl -fsS -m 10 "http://127.0.0.1:${toString tunnelMetricsPort}/ready" >/dev/null 2>&1; then
      echo 0 > "$countFile"
      exit 0
    fi
    count=$((count + 1))
    echo "$count" > "$countFile"
    echo "pm-tunnel-watchdog: /ready failed ($count consecutive)" >&2
    if [ "$count" -ge 3 ]; then
      echo 0 > "$countFile"
      exec /run/current-system/sw/bin/systemctl --user restart pm-tunnel.service
    fi
  '';
in
{
  options.services.pm-stack = {
    enable = lib.mkEnableOption "the PM stack (aggregator, projector, dispatch, tunnel, Kaneo, fleet-feed collector) as NixOS-managed user services";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the PM stack services.";
    };

    projectorPort = lib.mkOption {
      type = lib.types.port;
      default = 7801;
      description = "Port the pm-projector listens on (loopback only).";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services = {
        pm-projector = {
          Unit = {
            Description = "PM projector + link registry — GitHub projection for the aggregator (NixOS; packaged, pkgs/pm-projector)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            # The DB is pinned by DATABASE_URL in the env file; the
            # state dir as cwd also keeps the code's cwd-relative
            # fallback (file:data/projector.db) out of any checkout.
            WorkingDirectory = projState;
            ExecStart = "${pmProjector}/bin/pm-projector";
            EnvironmentFile = "${projState}/.env";
            Environment = [ "PM_AGG_FRESHNESS_URL=http://127.0.0.1:4871/publish" ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-aggregator = {
          Unit = {
            Description = "PM aggregator — board / roadmap / quota / needs-you (NixOS; packaged, pkgs/pm-aggregator)";
            After = [ "pm-projector.service" ];
            Wants = [ "pm-projector.service" ];
          };
          Service = {
            Type = "simple";
            # Every state file is env-pinned below or in the .env
            # file; the state dir as cwd makes any unpinned cwd
            # default in the config land in state too, never in a
            # checkout or the (read-only) store.
            WorkingDirectory = aggState;
            ExecStart = "${pmAggregator}/bin/pm-aggregator";
            EnvironmentFile = "${aggState}/.env";
            # The projector is a deployed service of this stack — never
            # render it off (see the header note). The coverage snapshot
            # is runtime state: keep it in the state dir, not the
            # checkout — written into the working tree it dirtied the
            # primary clone and failed pm-git-sync on every cycle
            # (2026-11-01; the aggregator's config already supports
            # PM_AGG_COVERAGE_SNAPSHOT_FILE for exactly this).
            Environment = [
              "PM_PROJECTOR_URL=http://127.0.0.1:${toString cfg.projectorPort}"
              # The Kaneo projects registry used to be reached through
              # the checkout working directory (../kaneo-projects.json
              # from pm-stack/aggregator); the package bundles the
              # file, and this pins the reader at the bundled copy.
              "PM_KANEO_PROJECTS_FILE=${pmAggregator}/libexec/pm-aggregator/kaneo-projects.json"
              "PM_AGG_COVERAGE_SNAPSHOT_FILE=${aggState}/coverage-snapshot.json"
              # Roadmap annotations (#1284) are runtime state too: the
              # first annotation would otherwise create
              # roadmap-annotations.json inside this checkout and block
              # pm-git-sync the same way (Codex P1 on PR #4054).
              "PM_AGG_ROADMAP_ANNOTATIONS_FILE=${aggState}/roadmap-annotations.json"
              # The event log and saved views are runtime state too
              # (2026-11-03, code#4906): events.json was still written
              # into the checkout, where a repo-keeper snapshot of it
              # onto main blocked every fast-forward of the primary.
              # Declared here like the two paths above; the same two
              # vars were also added to the state .env so the deployed
              # (pre-switch) unit resolves them identically.
              "PM_AGG_EVENTS_FILE=${aggState}/events.json"
              "PM_AGG_VIEWS_FILE=${aggState}/views.json"
              # Freshness notices are a separate bounded store; the
              # publisher listener binds only to loopback on this port.
              "PM_AGG_FRESHNESS_FILE=${aggState}/freshness.json"
              "PM_AGG_FRESHNESS_PORT=4871"
              # The activity-ingestion store (#3321) is runtime state
              # too: left at its cwd default its first persisted
              # delivery created activity-ingestion.json inside this
              # checkout (Codex P1 on PR #5141, same failure class as
              # code#4906 above).
              "PM_AGG_ACTIVITY_FILE=${aggState}/activity-ingestion.json"
              # The digest archive (#1278) is runtime state too: one
              # JSON per day, rendered verbatim by /digest?date=<date>.
              "PM_AGG_DIGESTS_DIR=${aggState}/digests"
            ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-dispatch = {
          Unit = {
            Description = "pm-dispatch — authenticated intake: launch/track OpenCode sessions + queue intake (NixOS; packaged, pkgs/pm-dispatch)";
            After = [
              "network-online.target"
              "tailscaled.service"
            ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            # The app has no cwd-relative access of its own (its
            # paths come from the env files below); the deployment
            # dir stays as cwd because that is where its rendered
            # .env lives and where an operator expects to find it.
            WorkingDirectory = "${stackDir}/dispatch";
            # Wait for the tailnet bind address to exist (#6992); the
            # 180s wait needs room beyond the 90s default start limit.
            TimeoutStartSec = 300;
            ExecStartPre = "${waitForTailnet}";
            ExecStart = "${pmDispatch}/bin/pm-dispatch";
            # The env files stay outside the module, exactly like the
            # aggregator's: install.sh renders the sops-derived file
            # (GitHub App + Supabase config), and the operator file is
            # laptop-only (DISPATCH_API_TOKEN). The "-" keeps the hand
            # unit's semantics — a missing operator file degrades the
            # service instead of crash-looping it. systemd applies the
            # files in order, so the operator file wins on a collision:
            # it must never carry SUPABASE_* lines (install.sh warns).
            EnvironmentFile = [
              "${stackDir}/dispatch/.env"
              "-${home}/secrets/dispatch.env"
            ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-git-sync = {
          Unit = {
            Description = "pm-git-sync — keep ~/git/code on clean main (PM services deploy by merge -> switch, #8171)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            ExecStart = "${pmGitSync}/bin/pm-git-sync";
          };
        };

        pm-projector-reconcile = {
          Unit = {
            Description = "PM projector reconcile — poll GitHub so the projection self-heals beyond webhook delivery";
            After = [ "pm-projector.service" ];
          };
          Service = {
            Type = "oneshot";
            EnvironmentFile = "${projState}/.env";
            ExecStart = "${projectorReconcile}";
          };
        };

        quota-gatherer = {
          Unit = {
            Description = "Quota gatherer — provider quotas every ~15 minutes (NixOS; packaged pm-aggregator, #1151)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            TimeoutStartSec = 300;
            WorkingDirectory = aggState;
            # ANTIGRAVITY_OAUTH_CLIENT_SECRET for the quota subcommand's Antigravity token refresh (code#8086);
            # the file is materialized by the opencode unit's ExecStartPre (home/common).
            EnvironmentFile = "-%h/.local/state/opencode/mistral-api-key.env";
            Environment = [
              "QUOTA_CURRENT_PATH=${aggState}/quota-current.json"
              "QUOTA_HEARTBEAT_PATH=${aggState}/quota-gatherer-heartbeat.json"
              "QUOTA_GATHERER_STATE_PATH=${aggState}/quota-gatherer-state.json"
              "QUOTA_SNAPSHOTS_DIR=${aggState}/snapshots"
            ];
            ExecStart = "${pmAggregator}/bin/pm-aggregator-gather-quota";
          };
        };

        pm-digest-daily = {
          Unit = {
            Description = "PM daily digest — generate + archive + deliver the day's digest (NixOS; packaged pm-aggregator, #1278)";
            After = [
              "network-online.target"
              "pm-aggregator.service"
            ];
          };
          Service = {
            Type = "oneshot";
            TimeoutStartSec = 300;
            WorkingDirectory = aggState;
            EnvironmentFile = "${aggState}/.env";
            Environment = [
              "PM_AGG_SOURCE=live"
              "PM_AGG_DIGESTS_DIR=${aggState}/digests"
            ];
            ExecStart = "${pmAggregator}/bin/pm-aggregator-generate-digest --kind daily";
          };
        };

        pm-digest-weekly = {
          Unit = {
            Description = "PM weekly digest — generate + archive + deliver the week's roll-up (NixOS; packaged pm-aggregator, #1278)";
            After = [
              "network-online.target"
              "pm-aggregator.service"
            ];
          };
          Service = {
            Type = "oneshot";
            TimeoutStartSec = 300;
            WorkingDirectory = aggState;
            EnvironmentFile = "${aggState}/.env";
            Environment = [
              "PM_AGG_SOURCE=live"
              "PM_AGG_DIGESTS_DIR=${aggState}/digests"
            ];
            ExecStart = "${pmAggregator}/bin/pm-aggregator-generate-digest --kind weekly";
          };
        };

        # fleet-feed-collector is the one unit still running from
        # the checkout, deliberately (code#8171 follow-up): its script
        # derives repo-root paths from its own file location — it
        # reads the checkout's .opencode/opencode.jsonc and inspects
        # the checkout itself for drift (collect-fleet-feed.ts:
        # AA_OPENCODE_CONFIG_PATH default, computeCheckoutDrift) — so
        # a store copy would inspect the wrong tree. Packaging it
        # first needs a code change to take the checkout root from
        # the environment.
        fleet-feed-collector = {
          Unit = {
            Description = "Fleet feed collector for the PM aggregator — collect the fleet feed every ~5 minutes (NixOS; runs from ~/git/code checkout, #3363)";
            After = [ "network-online.target" ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            WorkingDirectory = "${repo}/pm-stack/aggregator";
            Environment = [
              "BWS_BIN=/run/current-system/sw/bin/bws"
              "BWS_ACCESS_TOKEN_FILE=${home}/.config/bws/access-token"
              "FLEET_FEED_OUT=${aggState}/fleet-feed.json"
              "PM_AGG_FRESHNESS_URL=http://127.0.0.1:4871/publish"
              "QUOTA_VIEW_PATH=${home}/hatch-personal/workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md"
              "QUOTA_REGISTRY_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md"
              "ORCHESTRATION_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/orchestration.json"
              "DNS_REGISTER_PATH=${home}/hatch-personal/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/dns-domain-register.md"
              "CHARGE_CANDIDATES_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-candidates.json"
              "CHARGES_SURFACED_STATE_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/charges-surfaced-state.json"
              "CLAUDE_LAUNCH_RECORDS_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/claude-launch-records.jsonl"
              "USAGE_SNAPSHOT_OUTBOX_PATH=${aggState}/usage-snapshot-outbox.jsonl"
              "SANDBOX_OPENCODE_URL=http://127.0.0.1:13420"
              "OPENCODE_SERVER_URL=http://127.0.0.1:13420"
              "OPENCODE_SESSION_SOURCE=opencode-laptop"
              "OPENCODE_SERVER_PASSWORD_FILE=${aggState}/opencode-server-password"
            ];
            # Extracts the prod OpenCode server password into the bare
            # file the collector reads; the script itself stays
            # operator-managed in ~/.local/bin (mirrors oc-password-sync).
            ExecStartPre = "${home}/.local/bin/fleet-feed-oc-password.sh";
            ExecStart = "${bun} run scripts/collect-fleet-feed.ts --out ${aggState}/fleet-feed.json";
          };
        };

        pm-tunnel = {
          Unit = {
            Description = "Cloudflare Tunnel connector for the pm stack — kaneo + dispatch ingress (NixOS)";
            After = [
              "network-online.target"
              "docker.service"
            ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            WorkingDirectory = "${stackDir}";
            # tunnel.token is rendered by the pm-kaneo deploy flow
            # (deploy.sh) from the same SOPS secret as the old
            # tunnel.env; cloudflared reads it via --token-file so
            # the token stays out of the process argv (#8933).
            # Wait until the edge hostname resolves (#6992).
            TimeoutStartSec = 300;
            ExecStartPre = "${waitForDns}";
            # --protocol http2 (#5647): the default QUIC transport kept
            # dropping on this host's network path - cloudflared logged
            # "failed to accept QUIC stream: timeout: no recent network
            # activity" and re-registered (2026-11-05 11:36Z), and while
            # its connections were down Cloudflare's edge answered
            # pm-dispatch callers with 502 ("error code: 502") even
            # though pm-dispatch itself was healthy. HTTP/2 runs over
            # TCP and does not depend on the UDP path QUIC needs.
            ExecStart = "${cloudflared} tunnel --no-autoupdate --protocol http2 --metrics 127.0.0.1:${toString tunnelMetricsPort} run --token-file ${stackDir}/tunnel.token";
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-tunnel-watchdog = {
          Unit = {
            Description = "pm-tunnel watchdog — restart pm-tunnel when its edge connections stay down (#6992)";
            After = [ "pm-tunnel.service" ];
          };
          Service = {
            Type = "oneshot";
            ExecStart = "${tunnelWatchdog}";
          };
        };

        # The Kaneo stack is a oneshot, not a daemon: deploy.sh renders
        # the env, pulls the images, and (re)creates the containers,
        # which then run under docker's own restart policy. A user unit
        # cannot order itself against the system docker.service, so if
        # dockerd is not ready when this first runs, the pm-kaneo timer
        # below retries it until it has succeeded once (a succeeded
        # oneshot stays active, so later timer elapses are no-ops) —
        # the same self-heal shape as the projector reconcile.
        pm-kaneo = {
          Unit = {
            Description = "Kaneo stack — render env + docker compose up via pm-stack/deploy.sh (NixOS)";
            After = [
              "network-online.target"
              "docker.service"
            ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            RemainAfterExit = true;
            # The first activation pulls both images and then waits
            # out deploy.sh's health gate; give it room.
            TimeoutStartSec = 1200;
            WorkingDirectory = "${stackDir}";
            ExecStart = "${stackDir}/deploy.sh";
            # No -v: stopping the stack never deletes the postgres
            # data volume.
            ExecStop = "${docker} compose down";
          };
          Install.WantedBy = [ "default.target" ];
        };
      };

      systemd.user.timers = {
        pm-git-sync = {
          Unit.Description = "Run pm-git-sync every 30 seconds";
          Timer = {
            OnBootSec = "30s";
            OnUnitActiveSec = "30s";
            AccuracySec = "5s";
            Unit = "pm-git-sync.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        pm-projector-reconcile = {
          Unit.Description = "Reconcile the PM projector against GitHub every 15 minutes";
          Timer = {
            OnCalendar = "*:0/15";
            RandomizedDelaySec = 20;
            Persistent = true;
            Unit = "pm-projector-reconcile.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        quota-gatherer = {
          Unit.Description = "Run the quota gatherer every 15 minutes (pm-stack, #1151)";
          Timer = {
            OnCalendar = "*:0/15";
            RandomizedDelaySec = 20;
            Persistent = true;
            Unit = "quota-gatherer.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        pm-digest-daily = {
          Unit.Description = "Run the PM daily digest every morning at 07:00 local (pm-stack, #1278)";
          Timer = {
            OnCalendar = "*-*-* 07:00:00";
            RandomizedDelaySec = 60;
            Persistent = true;
            Unit = "pm-digest-daily.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        pm-digest-weekly = {
          Unit.Description = "Run the PM weekly digest on Mondays at 07:05 local (pm-stack, #1278)";
          Timer = {
            OnCalendar = "Mon *-*-* 07:05:00";
            RandomizedDelaySec = 60;
            Persistent = true;
            Unit = "pm-digest-weekly.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        fleet-feed-collector = {
          Unit.Description = "Collect the fleet feed for the laptop aggregator every ~5 minutes";
          Timer = {
            OnBootSec = "2min";
            OnUnitActiveSec = "5min";
            Unit = "fleet-feed-collector.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        pm-kaneo = {
          Unit.Description = "Retry pm-kaneo until the Kaneo stack is up (docker-ordering self-heal)";
          Timer = {
            OnBootSec = "1min";
            OnUnitActiveSec = "30min";
            Unit = "pm-kaneo.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };

        pm-tunnel-watchdog = {
          Unit.Description = "Check pm-tunnel edge readiness every 2 minutes (#6992)";
          Timer = {
            OnBootSec = "2min";
            OnUnitActiveSec = "2min";
            Unit = "pm-tunnel-watchdog.service";
          };
          Install.WantedBy = [ "timers.target" ];
        };
      };
    };
  };
}
