# PM stack (Drew, 2026-10-27: "make nixos switch deploy all these
# components like everything else. enable projector and all the other
# features.")
#
# The project-management dashboard components for this host, declared as
# home-manager user services so `nixos-rebuild switch` deploys them like
# everything else on this machine — replacing the hand-written units that
# used to live in ~/.config/systemd/user (pm-aggregator, pm-projector,
# pm-git-sync, quota-gatherer; the pre-NixOS files are kept beside this
# history in git, and home-manager renames any same-named hand file to
# *.backup on the first switch that lands this module).
#
# Code source: the services run the aggregator/projector straight out of
# the primary code checkout (~/git/code, pm-stack/aggregator and
# pm-stack/projector) at latest origin/main — the keep-primaries-clean
# model. pm-git-sync (below) keeps that checkout on a clean, current main
# and restarts a service when its subtree changes, so code updates land
# by git pull while unit definitions, environment, and lifecycle land by
# switch. (mcpx-style store-pinned packages were the alternative; the PM
# stack has no package.nix and iterates daily, and the checkout model is
# what the fleet standardized on 2026-10-27.)
#
# Runtime state + secrets stay on the machine, never in this repo:
# ~/.local/state/pm-aggregator/.env and ~/.local/state/pm-projector/.env
# (mode 600, rendered from the pm-stack SOPS file; they carry the
# PM_AGG_*_FILE paths, Kaneo/Grist credentials, and the GitHub App
# credentials BY NAME, exactly as the hand units consumed them).
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
  home = "/home/${cfg.user}";
  repo = "${home}/git/code";
  aggState = "${home}/.local/state/pm-aggregator";
  projState = "${home}/.local/state/pm-projector";

  projectorReconcile = pkgs.writeShellScript "pm-projector-reconcile" ''
    # PM_PROJECTOR_API_TOKEN arrives via the unit's EnvironmentFile.
    exec ${pkgs.curl}/bin/curl -fsS -X POST \
      -H "Authorization: Bearer $PM_PROJECTOR_API_TOKEN" \
      http://127.0.0.1:${toString cfg.projectorPort}/reconcile
  '';
in
{
  options.services.pm-stack = {
    enable = lib.mkEnableOption "the PM dashboard stack (aggregator + projector) as NixOS-managed user services";

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
            # render it off (see the header note).
            Environment = "PM_PROJECTOR_URL=http://127.0.0.1:${toString cfg.projectorPort}";
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
      };
    };
  };
}
