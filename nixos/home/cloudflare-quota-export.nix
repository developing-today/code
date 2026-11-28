# cloudflare-quota-export — the Cloudflare quota exporter
# (developing-today/code#1048), run every 15 minutes as a
# quota-gatherer tick (developing-today/code#1151).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.cloudflare-quota-export.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the units were plain files in ~/.config/systemd/user.
# Semantics are replicated exactly; only provenance changes. The
# script itself stays at ~/atlas/cloudflare-export/, next to the
# data it exports.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.cloudflare-quota-export;
in
{
  options.services.cloudflare-quota-export = {
    enable = lib.mkEnableOption "the Cloudflare quota exporter (code#1048)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.cloudflare-quota-export = {
      Unit = {
        Description = "Cloudflare quota exporter — code#1048";
        After = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        TimeoutStartSec = 120;
        ExecStart = "%h/atlas/cloudflare-export/cloudflare-quota-export.sh --out-dir %h/atlas/cloudflare-export";
      };
    };

    systemd.user.timers.cloudflare-quota-export = {
      Unit.Description = "Run the Cloudflare quota exporter every 15 minutes — code#1048 (gatherer tick, #1151)";
      Timer = {
        OnCalendar = "*:0/15";
        RandomizedDelaySec = 20;
        Persistent = true;
        Unit = "cloudflare-quota-export.service";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
