# meta-quota-export — the Meta (dev.meta.ai) quota exporter
# (developing-today/code#1301 Phase 1), run every 15 minutes.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.meta-quota-export.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the units were plain files in ~/.config/systemd/user.
# Semantics are replicated exactly; only provenance changes. The
# script itself stays at ~/atlas/meta-quota-export/, next to the
# data it exports, and runs under the system node.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.meta-quota-export;
in
{
  options.services.meta-quota-export = {
    enable = lib.mkEnableOption "the Meta quota exporter (code#1301)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.meta-quota-export = {
      Unit = {
        Description = "Meta (dev.meta.ai) quota exporter — code#1301 Phase 1";
        After = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        TimeoutStartSec = 120;
        ExecStart = "/run/current-system/sw/bin/node %h/atlas/meta-quota-export/meta-quota-export.mjs --cdp-port 19222 --out-dir %h/atlas/meta-quota-export";
      };
    };

    systemd.user.timers.meta-quota-export = {
      Unit.Description = "Run the Meta quota exporter every 15 minutes — code#1301 Phase 1";
      Timer = {
        OnCalendar = "*:0/15";
        RandomizedDelaySec = 20;
        Persistent = true;
        Unit = "meta-quota-export.service";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
