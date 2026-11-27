# Archer Slack collector (code#2137 S2): the Socket Mode listener as
# a NixOS-managed user service on the laptop.
#
# Why the laptop (design record doc/fleet/archer-slack/
# socket-mode-collector.md, decision 1): the collector is a
# supervised daemon; the sandbox has no systemd user bus at all and
# its one live run (2026-10-29) died with a sandbox recycle ~54
# minutes in, so the 3-day parallel run the cutover waits on can only
# be hosted here, via this flake — the issue's own placement rule.
#
# The unit runs the packaged collector (pkgs/fleet/archer-slack/
# collector — `nix build .#archer-slack-collector` is the package
# check). Its launcher fetches the app-level token by name
# (slack.archer-app-token) via the fleet secrets route at start; the
# token never enters this repo, the unit text, or the journal.
#
# The store: the collector appends to the puller store at the same
# path the archer-slack-review job reads on its own seat
# (~/workspace/archer-slack/puller/state/archer-slack.db, here the
# laptop-local copy the parallel run writes). The review job and the
# watermark contract are untouched by this slice; the wake gate is
# S3, the fidelity comparison (which reads both stores) is S4.
#
# Reactions: the collector stores reaction_added events, but the
# Slack app manifest does not subscribe to them and the bot lacks
# reactions:read — capture is code-ready but event-starved until the
# workspace owner amends the manifest and reinstalls the app (named
# owner action in the design record; not a code gap).
#
# Failure shape: transient Slack drops reconnect inside the collector
# with bounded backoff; the process exits only on fatal auth errors
# (78) or a crash, which Restart=on-failure surfaces loudly in the
# journal — a revoked token therefore restart-loops at RestartSec
# pace instead of failing silently, and the S4 fidelity report reads
# the restart count from this unit's journal.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.archer-slack-collector;
  alignmentCfg = config.services.alignment-handler;
  home = user: "/home/${user}";
  collector = pkgs.callPackage ../../../pkgs/fleet/archer-slack/collector/package.nix { };
  alignmentHandler = pkgs.callPackage ../../../pm-stack/alignment-handler/package.nix { };
in
{
  options.services.archer-slack-collector = {
    enable = lib.mkEnableOption "the Archer Slack Socket Mode collector as a NixOS-managed user service (code#2137)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the collector.";
    };

    channels = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "C0CBYQR65LL" ];
      description = "Watched Slack channel IDs. The PoC scope is #alignment only (Drew, 2026-10-28).";
    };

    dbPath = lib.mkOption {
      type = lib.types.str;
      default = "${home cfg.user}/workspace/archer-slack/puller/state/archer-slack.db";
      description = "The puller store the collector appends collected_events to — the same store path the archer-slack-review job reads on its seat.";
    };
  };

  options.services.alignment-handler = {
    enable = lib.mkEnableOption "the staged-first #alignment request handler (code#6651)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home-manager session runs the alignment handler.";
    };

    dbPath = lib.mkOption {
      type = lib.types.str;
      default = "${home alignmentCfg.user}/workspace/archer-slack/puller/state/archer-slack.db";
      description = "The same SQLite event store written by the Archer Socket Mode collector.";
    };

    botUserId = lib.mkOption {
      type = lib.types.str;
      default = "U0CBTJGQQSZ";
      description = "Archer's Slack bot user ID used to detect owner mentions.";
    };
  };

  config = lib.mkMerge [
    {
      assertions = [
        {
          assertion = !alignmentCfg.enable || cfg.enable;
          message = "services.alignment-handler requires services.archer-slack-collector to provide the #alignment event store";
        }
      ];
    }
    (lib.mkIf cfg.enable {
      home-manager.users.${cfg.user} = {
        systemd.user.services.archer-slack-collector = {
          Unit = {
            Description = "Archer Slack collector — Socket Mode listener for #alignment; parallel run beside the watermark puller (NixOS; packaged, pkgs/fleet/archer-slack/collector; code#2137)";
            After = [ "network-online.target" ];
            Wants = [ "network-online.target" ];
          };
          Service = {
            Type = "simple";
            Environment = [
              "ARCHER_SLACK_DB=${cfg.dbPath}"
              "ARCHER_WATCH_CHANNELS=${lib.concatStringsSep "," cfg.channels}"
              # The launcher resolves `fleet` (token fetch) and the
              # coreutils it needs from this PATH.
              "PATH=${home cfg.user}/.local/bin:/run/current-system/sw/bin:/usr/bin:/bin"
            ];
            ExecStart = "${collector}/bin/archer-slack-collector";
            Restart = "on-failure";
            RestartSec = 30;
          };
          Install.WantedBy = [ "default.target" ];
        };
      };
    })
    (lib.mkIf alignmentCfg.enable {
      home-manager.users.${alignmentCfg.user}.systemd.user.services.alignment-handler = {
        Unit = {
          Description = "Fleet #alignment request handler (code#6651)";
          After = [
            "network-online.target"
            "archer-slack-collector.service"
          ];
          Wants = [
            "network-online.target"
            "archer-slack-collector.service"
          ];
        };
        Service = {
          Type = "simple";
          Environment = [
            "ARCHER_SLACK_DB=${alignmentCfg.dbPath}"
            "ALIGNMENT_HANDLER_BOT_USER_ID=${alignmentCfg.botUserId}"
            "PATH=${home alignmentCfg.user}/.local/bin:/run/current-system/sw/bin:/usr/bin:/bin"
          ];
          ExecStart = "${alignmentHandler}/bin/alignment-handler";
          Restart = "on-failure";
          RestartSec = 15;
        };
        Install.WantedBy = [ "default.target" ];
      };
    })
  ];
}
