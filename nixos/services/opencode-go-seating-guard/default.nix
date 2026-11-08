# opencode-go-seating-guard — hourly seat check for the OpenCode Go
# leaves (developing-today/code#5611).
#
# The failure this guards (2026-11-03/04): provisioning the popesdrew Go
# instance stored its OAuth credential under the bare integration id
# `opencode` instead of `opencode-go.popesdrew`. The route selector matches
# credentials by the EXACT leaf provider id, so every attempt failed with
# "no credential for provider opencode-go.popesdrew": 23 failed attempts,
# the account served 0% for ~16h, and nothing noticed. Provisioning ended at
# "rows created" and the weighted pool config was correct as deployed — the
# gap was that nothing asserted the leaf could actually serve.
#
# The check itself lives in the repo's bin/ directory
# (bin/check-opencode-go-seating.mjs, read from the primary checkout at
# latest origin/main — the keep-primaries-clean model pm-stack uses) and
# asserts, for every `opencode-go.*` leaf the config seats in a pool:
#
#   1. credential  A credential row under the EXACT leaf integration id with
#                  active = 1 (never the bare `opencode` console-login key).
#   2. attempt     A successful `route_attempt` for the leaf after the
#                  active config's latest modification (a conservative
#                  seating-time proxy) and within the window. Surface
#                  substitution recorded on #5611: the
#                  standalone route proxy whose ledger the issue named was
#                  retired under opencode#1153; the successor surface is the
#                  `route_attempt` table in the OpenCode history store, which
#                  the in-fork route selector writes and the usage aggregator
#                  reads.
#
# Read-only: one sqlite read of the store plus one config parse. Prints
# provider ids, credential ids/labels and attempt counts — never token
# values. Exit 0 healthy, 1 a seated leaf is not serving, 2 check error
# (config or store unreadable). There is no separate alert surface beyond
# the failed unit + journal, the same surface openai-cred-watchdog relies on,
# so a violation fails this unit naming the offending leaf.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.opencode-go-seating-guard;
  home = "/home/${cfg.user}";
  repo = "${home}/git/code";
  guard = pkgs.writeShellApplication {
    name = "opencode-go-seating-guard";
    runtimeInputs = [ pkgs.nodejs_24 ];
    text = ''
      # The guard reads the tracked config and the live store; both paths are
      # defaults of the script itself, overridable by the module for tests.
      exec node ${repo}/bin/check-opencode-go-seating.mjs \
        "${cfg.configPath}" \
        --db "${cfg.storePath}" \
        --window-hours ${toString cfg.windowHours} \
        --stage ${cfg.stage}
    '';
  };
in
{
  options.services.opencode-go-seating-guard = {
    enable = lib.mkEnableOption "the OpenCode Go seating guard (hourly assertion that every seated opencode-go.* leaf has an exact-id credential and has served, code#5611)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose home holds the OpenCode credential store and the code checkout.";
    };

    configPath = lib.mkOption {
      type = lib.types.str;
      default = "${home}/.local/share/opencode/opencode.jsonc";
      description = "Active OpenCode JSONC config whose seated opencode-go.* leaves are checked.";
    };

    storePath = lib.mkOption {
      type = lib.types.str;
      default = "${home}/.local/share/opencode/opencode.db";
      description = "OpenCode history store holding the credential and route_attempt tables.";
    };

    windowHours = lib.mkOption {
      type = lib.types.ints.positive;
      default = 24;
      description = "How far back a successful route_attempt counts as the leaf serving.";
    };

    stage = lib.mkOption {
      type = lib.types.enum [
        "post-seat"
        "periodic"
      ];
      default = "periodic";
      description = "Assertion stage. 'pre-seat' is a registration-time mode (credential only) and is not a timer stage.";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services.opencode-go-seating-guard = {
        Unit.Description = "OpenCode Go seating guard — every seated opencode-go.* leaf has an exact-id active credential and has served (code#5611)";
        Service = {
          Type = "oneshot";
          TimeoutStartSec = 120;
          ExecStart = "${guard}/bin/opencode-go-seating-guard";
        };
      };

      systemd.user.timers.opencode-go-seating-guard = {
        Unit.Description = "Run the OpenCode Go seating guard hourly (code#5611)";
        Timer = {
          OnCalendar = "hourly";
          RandomizedDelaySec = 20;
          Persistent = true;
          Unit = "opencode-go-seating-guard.service";
        };
        Install.WantedBy = [ "timers.target" ];
      };
    };
  };
}
