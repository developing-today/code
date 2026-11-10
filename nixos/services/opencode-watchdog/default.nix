# opencode-watchdog — dark-port watchdog for the standing OpenCode
# server (developing-today/code#7780).
#
# The failure this guards (2026-11-08, laptop `amd`): opencode.service's
# main process exited, systemd's Restart=on-failure restarted it once,
# and the replacement exited with status 0 after 731 ms without ever
# listening — a clean exit, so systemd made no further attempt. Port
# 13420 stayed dark and service-prod.json stayed absent for ~15
# minutes until a human-path manual start; nothing alerted.
#
# Why not Restart=always in the unit itself: the fork module
# (developing-today-forks/opencode, nix/modules) chose on-failure
# deliberately — the service lifecycle hands a running server over to
# a successor during upgrades and the old process then exits cleanly;
# a restart-on-success policy would immediately spawn a second server
# racing the successor for the port. This watchdog preserves that
# semantics and closes only the stuck-dark hole:
#
#   serving(port) OR active(unit)          -> healthy, forget darkness
#   dark for less than graceSeconds        -> wait (restart/handover
#                                             windows are legitimately
#                                             dark for a few seconds)
#   dark continuously >= graceSeconds      -> alert (journal, and this
#                                             unit exits 1 so it shows
#                                             as failed until the next
#                                             healthy run — the same
#                                             alert surface the seating
#                                             guard relies on) and
#                                             `systemctl --user start`
#                                             the unit
#
# The probe is the oracle: "serving" means a TCP connect to
# 127.0.0.1:<port> succeeds. A handover successor serving the port
# makes the watchdog healthy regardless of the unit's state, so the
# watchdog can never race a handover.
#
# Test hooks (used by the module's verification, harmless in
# production): OPENCODE_WATCHDOG_UNIT / OPENCODE_WATCHDOG_PORT /
# OPENCODE_WATCHDOG_GRACE_SECONDS / OPENCODE_WATCHDOG_STATE_DIR
# override the baked values.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.opencode-watchdog;
  watchdog = pkgs.writeShellApplication {
    name = "opencode-watchdog";
    runtimeInputs = [ pkgs.systemd ];
    text = ''
      unit="''${OPENCODE_WATCHDOG_UNIT:-opencode.service}"
      port="''${OPENCODE_WATCHDOG_PORT:-${toString cfg.port}}"
      grace="''${OPENCODE_WATCHDOG_GRACE_SECONDS:-${toString cfg.graceSeconds}}"
      state_dir="''${OPENCODE_WATCHDOG_STATE_DIR:-''${XDG_STATE_HOME:-$HOME/.local/state}/opencode-watchdog}"
      stamp="$state_dir/first-dark-epoch"

      serving() {
        (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null
      }

      if systemctl --user is-active --quiet "$unit" || serving; then
        rm -f "$stamp"
        exit 0
      fi

      # Dark: unit not active and nothing serving the port.
      mkdir -p "$state_dir"
      now="$(date +%s)"
      first=""
      if [ -f "$stamp" ]; then
        first="$(cat "$stamp")"
      fi
      case "$first" in
        "" | *[!0-9]*)
          printf '%s\n' "$now" >"$stamp"
          echo "opencode-watchdog: $unit inactive and port $port dark; grace window (${toString cfg.graceSeconds}s default) started" >&2
          exit 0
          ;;
      esac

      dark_for=$((now - first))
      if [ "$dark_for" -lt "$grace" ]; then
        exit 0
      fi

      echo "opencode-watchdog: ALERT: $unit has been inactive with port $port dark for ''${dark_for}s (>= ''${grace}s grace) — starting $unit (code#7780)" >&2
      rm -f "$stamp"
      systemctl --user start "$unit" || true
      exit 1
    '';
  };
in
{
  options.services.opencode-watchdog = {
    enable = lib.mkEnableOption "the OpenCode dark-port watchdog (per-minute assertion that opencode.service is active or something serves its port, with alert + restart after a grace window, code#7780)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose user unit opencode.service is watched.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 13420;
      description = "Port the standing OpenCode server must serve on 127.0.0.1.";
    };

    graceSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 120;
      description = "How long the unit may stay inactive with a dark port before the watchdog alerts and restarts it. Covers systemd restart windows and lifecycle handovers, which are dark only for seconds.";
    };

    checkIntervalSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 60;
      description = "How often the watchdog timer runs.";
    };
  };

  config = lib.mkIf cfg.enable {
    home-manager.users.${cfg.user} = {
      systemd.user.services.opencode-watchdog = {
        Unit.Description = "OpenCode watchdog — opencode.service active or port ${toString cfg.port} serving, else alert + restart after ${toString cfg.graceSeconds}s dark (code#7780)";
        Service = {
          Type = "oneshot";
          TimeoutStartSec = 60;
          ExecStart = "${watchdog}/bin/opencode-watchdog";
        };
      };

      systemd.user.timers.opencode-watchdog = {
        Unit.Description = "Run the OpenCode watchdog every ${toString cfg.checkIntervalSeconds}s (code#7780)";
        Timer = {
          OnBootSec = "2min";
          OnUnitActiveSec = "${toString cfg.checkIntervalSeconds}s";
          Unit = "opencode-watchdog.service";
        };
        Install.WantedBy = [ "timers.target" ];
      };
    };
  };
}
