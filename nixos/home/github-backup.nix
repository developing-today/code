# github-backup — the GitHub estate mirror system's laptop timers
# (developing-today/code#3215): drain the Worker webhook queue every
# 5 minutes, fetch all mirrors daily, reconcile weekly.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.github-backup.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now these units were plain files in ~/.config/systemd/user
# installed by infra/github-backup's installer. Semantics are
# replicated exactly; only provenance changes. The scripts stay in
# this repo and run from the primary checkout (~/git/code), per the
# fleet-feed-collector precedent. The drain wrapper
# (infra/github-backup/systemd/drain-wrapper.sh) sources the Worker
# URL + bearer from Bitwarden by name at runtime, so the units need
# no Environment of their own.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.github-backup;
  home = config.home.homeDirectory;
  infra = "${home}/git/code/infra/github-backup";
  netService = description: execStart: {
    Unit = {
      Description = description;
      After = [ "network-online.target" ];
      Wants = [ "network-online.target" ];
    };
    Service = {
      Type = "oneshot";
      ExecStart = execStart;
      WorkingDirectory = "%h/backups/github";
    };
  };
in
{
  options.services.github-backup = {
    enable = lib.mkEnableOption "the github-backup mirror timers (drain, fetch, reconcile)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services = {
      github-backup-drain = netService "github-backup: drain webhook queue from the Worker and fetch" "${infra}/systemd/drain-wrapper.sh";
      github-backup-fetch = netService "github-backup: daily fetch of all mirrors" "/usr/bin/env python3 ${infra}/bin/fetch_mirrors.py";
      github-backup-reconcile = netService "github-backup: weekly reconciliation sweep" "/usr/bin/env python3 ${infra}/bin/reconcile.py";
    };

    systemd.user.timers = {
      github-backup-drain = {
        Unit.Description = "github-backup queue drain (every 5 min)";
        Timer = {
          OnBootSec = "3min";
          OnUnitActiveSec = "5min";
          Persistent = true;
        };
        Install.WantedBy = [ "timers.target" ];
      };
      github-backup-fetch = {
        Unit.Description = "github-backup fetch (daily)";
        Timer = {
          OnCalendar = "daily";
          Persistent = true;
          RandomizedDelaySec = "30m";
        };
        Install.WantedBy = [ "timers.target" ];
      };
      github-backup-reconcile = {
        Unit.Description = "github-backup reconcile (weekly)";
        Timer = {
          OnCalendar = "weekly";
          Persistent = true;
          RandomizedDelaySec = "2h";
        };
        Install.WantedBy = [ "timers.target" ];
      };
    };
  };
}
