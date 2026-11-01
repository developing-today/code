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
# Code source: the services run the aggregator/projector/dispatch
# straight out of the primary code checkout (~/git/code,
# pm-stack/aggregator, pm-stack/projector, pm-stack/dispatch) at latest
# origin/main — the keep-primaries-clean model. pm-git-sync (below)
# keeps that checkout on a clean, current main and restarts a service
# when its subtree changes, so code updates land by git pull while unit
# definitions, environment, and lifecycle land by switch. (mcpx-style
# store-pinned packages were the alternative; the PM stack has no
# package.nix and iterates daily, and the checkout model is what the
# fleet standardized on 2026-10-27.)
#
# Runtime state + secrets stay on the machine, never in this repo:
# ~/.local/state/pm-aggregator/.env and ~/.local/state/pm-projector/.env
# (mode 600, rendered from the pm-stack SOPS file; they carry the
# PM_AGG_*_FILE paths, Kaneo/Grist credentials, and the GitHub App
# credentials BY NAME, exactly as the hand units consumed them).
# pm-dispatch's env files are external the same way:
# ~/pm-stack/dispatch/.env (rendered by pm-stack/dispatch/install.sh
# from the same SOPS file) and ~/secrets/dispatch.env (operator secrets
# that are not in SOPS). install.sh remains the env renderer and the
# fallback installer; this module owns the unit lifecycle.
#
# The Kaneo stack is the exception to the checkout model: pm-kaneo
# runs deploy.sh from the ~/pm-stack deployment dir, because the
# rendered .env, the compose project, and the pm-stack_postgres_data
# volume live there. deploy.sh also renders ~/pm-stack/tunnel.env,
# which pm-tunnel consumes — systemd expands TUNNEL_TOKEN from that
# EnvironmentFile in the tunnel's ExecStart, exactly as the
# hand-written unit relied on. (The tunnel's ingress is managed in
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

  projectorReconcile = pkgs.writeShellScript "pm-projector-reconcile" ''
    # PM_PROJECTOR_API_TOKEN arrives via the unit's EnvironmentFile.
    exec ${pkgs.curl}/bin/curl -fsS -X POST \
      -H "Authorization: Bearer $PM_PROJECTOR_API_TOKEN" \
      http://127.0.0.1:${toString cfg.projectorPort}/reconcile
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
            Description = "PM projector + link registry — GitHub projection for the aggregator (NixOS; runs from ~/git/code checkout)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            WorkingDirectory = "${repo}/pm-stack/projector";
            ExecStart = "${bun} run src/index.ts";
            EnvironmentFile = "${projState}/.env";
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-aggregator = {
          Unit = {
            Description = "PM aggregator — board / roadmap / quota / needs-you (NixOS; runs from ~/git/code checkout)";
            After = [ "pm-projector.service" ];
            Wants = [ "pm-projector.service" ];
          };
          Service = {
            Type = "simple";
            WorkingDirectory = "${repo}/pm-stack/aggregator";
            ExecStart = "${bun} run src/server.ts";
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
              "PM_AGG_COVERAGE_SNAPSHOT_FILE=${aggState}/coverage-snapshot.json"
            ];
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
        };

        pm-dispatch = {
          Unit = {
            Description = "pm-dispatch — authenticated intake: launch/track OpenCode sessions + queue intake (NixOS; runs from ~/git/code checkout)";
            After = [ "network-online.target" ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            WorkingDirectory = "${repo}/pm-stack/dispatch";
            ExecStart = "${bun} run src/server.ts";
            # The env files stay outside the module, exactly like the
            # aggregator's: install.sh renders the sops-derived file,
            # and the operator file is laptop-only. The "-" keeps the
            # hand unit's semantics — a missing operator file degrades
            # the service instead of crash-looping it.
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
            Description = "pm-git-sync — keep ~/git/code on clean main; redeploy PM services on subtree changes";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            ExecStart = "${repo}/pm-stack/scripts/pm-git-sync.sh";
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
            Description = "Quota gatherer — provider quotas every ~15 minutes (NixOS; runs from ~/git/code checkout, #1151)";
            After = [ "network-online.target" ];
          };
          Service = {
            Type = "oneshot";
            TimeoutStartSec = 300;
            WorkingDirectory = "${repo}/pm-stack/aggregator";
            Environment = [
              "QUOTA_CURRENT_PATH=${aggState}/quota-current.json"
              "QUOTA_HEARTBEAT_PATH=${aggState}/quota-gatherer-heartbeat.json"
              "QUOTA_GATHERER_STATE_PATH=${aggState}/quota-gatherer-state.json"
              "QUOTA_SNAPSHOTS_DIR=${aggState}/snapshots"
            ];
            ExecStart = "${bun} run ${repo}/pm-stack/aggregator/scripts/gather-quota.ts";
          };
        };

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
              "SUPABASE_PROJECT_REF=keihfchslgjetkmnmzra"
              "BWS_BIN=/run/current-system/sw/bin/bws"
              "BWS_ACCESS_TOKEN_FILE=${home}/.config/bws/access-token"
              "FLEET_FEED_OUT=${aggState}/fleet-feed.json"
              "QUOTA_VIEW_PATH=${home}/hatch-personal/workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md"
              "QUOTA_REGISTRY_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md"
              "JULES_STATE_PATH=${home}/hatch-personal/workspace/goals/keep-the-four-repos-healthy/hidden_files/jules-review-state.json"
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
            # tunnel.env is rendered by the pm-kaneo deploy flow
            # (deploy.sh); systemd expands TUNNEL_TOKEN from this
            # EnvironmentFile in ExecStart, exactly as the hand-written
            # unit relied on.
            EnvironmentFile = "${stackDir}/tunnel.env";
            ExecStart = "${cloudflared} tunnel --no-autoupdate run --token \${TUNNEL_TOKEN}";
            Restart = "on-failure";
            RestartSec = 5;
          };
          Install.WantedBy = [ "default.target" ];
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
      };
    };
  };
}
