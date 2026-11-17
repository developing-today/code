# dashboard-backup — the daily dashboard/orchestration data
# backup, laptop leg (developing-today/code#1459). Script:
# ./dashboard-backup.sh, vendored in this flake and packaged below
# (2026-11-03, mirroring laptop-backups.nix's pm-dashboard-backup);
# it previously lived at ~/tools/dashboard-backup.sh, maintained by
# its author there — the vendored copy is now the source of record
# and carries the 2026-11-03 dataset (the relocated aggregator
# runtime files: events.json, views.json, coverage-snapshot.json,
# aa-last-good.json, charge-inputs/).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.dashboard-backup.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until then the units were plain files in ~/.config/systemd/user.
#
# NOT the flake's pm-dashboard-backup (nixos/home/laptop-backups.nix,
# code#1516): that is a separate, newer job. This older job is folded
# in as-is; merging the two backups is a separate decision.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.dashboard-backup;
  dashboardPkg = pkgs.writeShellScriptBin "dashboard-backup" (
    builtins.readFile ./dashboard-backup.sh
  );
  servicePath = lib.makeBinPath [
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnutar
    pkgs.zstd
    pkgs.python3
    pkgs.systemd
  ];
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
        ExecStart = "${dashboardPkg}/bin/dashboard-backup";
        Environment = "PATH=${servicePath}";
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
