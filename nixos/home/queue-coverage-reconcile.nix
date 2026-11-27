# Queue coverage reconciliation (#7016): hourly issue/queue coverage
# check for the four repo pilot queues. The first-run observe/import
# report and operating contract live in scripts/fleet/queue-reconcile/.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.queue-coverage-reconcile;
  home = config.home.homeDirectory;
  checkCoverage = ../../scripts/fleet/queue-reconcile/check_coverage.py;
  runner = pkgs.writeShellApplication {
    name = "queue-coverage-reconcile";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.python3
      pkgs.gh
    ];
    text = ''
      # Command line: no arguments.
      if [[ $# -ne 0 ]]; then
        printf 'Unknown argument: %q\n' "$1" >&2
        printf 'Usage: %s\n' "$0" >&2
        exit 2
      fi

      mode_file="${home}/.config/queue-reconcile/mode"
      mode=observe
      if [[ -f "$mode_file" ]]; then
        mode="$(cat "$mode_file")"
      fi
      case "$mode" in
        observe) flag=--observe ;;
        import) flag=--import ;;
        *)
          printf 'queue-coverage-reconcile: invalid mode %q\n' "$mode" >&2
          exit 2
          ;;
      esac

      exec ${pkgs.python3}/bin/python3 ${checkCoverage} \
        "$flag" --fleet "${home}/tools/fleet"
    '';
  };
in
{
  options.services.queue-coverage-reconcile.enable = lib.mkEnableOption (
    "the hourly queue coverage reconciliation"
  );

  config = lib.mkIf cfg.enable {
    systemd.user.services.queue-coverage-reconcile = {
      Unit = {
        Description = "Queue coverage reconciliation: open issues vs live pilot-queue tasks (#7016)";
        After = [
          "network-online.target"
          "queue-broker.service"
        ];
        Wants = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        ExecStart = "${runner}/bin/queue-coverage-reconcile";
        WorkingDirectory = home;
      };
    };

    systemd.user.timers.queue-coverage-reconcile = {
      Unit.Description = "Queue coverage reconciliation, hourly (#7016)";
      Timer = {
        OnCalendar = "*-*-* *:41:00";
        Persistent = true;
        RandomizedDelaySec = "5m";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
