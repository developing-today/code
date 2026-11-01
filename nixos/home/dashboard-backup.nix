# dashboard-backup — the daily dashboard/orchestration data
# backup, laptop leg (developing-today/code#1459). Script:
# ~/tools/dashboard-backup.sh, maintained by its author there.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.dashboard-backup.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the units were plain files in ~/.config/systemd/user.
# Semantics are replicated exactly; only provenance changes.
#
# NOT the flake's pm-dashboard-backup (nixos/home/laptop-backups.nix,
# code#1516): that is a separate, newer job. This older job is folded
# in as-is; merging the two backups is a separate decision.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.dashboard-backup;
in
{
  options.services.dashboard-backup = {
    enable = lib.mkEnableOption "the daily dashboard backup, laptop leg (code#1459)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.dashboard-backup = {
      Unit.Description = "Daily dashboard/orchestration data backup (laptop leg, code#1459)";
      Service = {
        Type = "oneshot";
        ExecStart = "%h/tools/dashboard-backup.sh";
      };
    };

    systemd.user.timers.dashboard-backup = {
      Unit.Description = "Run dashboard backup daily ~04:30 local (code#1459)";
      Timer = {
        OnCalendar = "*-*-* 04:30:00";
        Persistent = true;
        Unit = "dashboard-backup.service";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
