# meta-quota-export — the Meta (dev.meta.ai) console-scrape quota
# exporter (developing-today/code#1301 Phase 1), run every 15 minutes
# as a quota-gatherer tick. It talks to the laptop Helium browser over
# CDP (:19222) and writes ~/atlas/meta-quota-export/{latest.json,
# history.jsonl}; the data dir stays operator-owned, exactly like the
# Cloudflare exporter's ~/atlas/cloudflare-export.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.meta-quota-export.enable = true.
#
# Folded into the flake 2026-11-09 (developing-today/code#2775): until
# now the units and the script were plain files copied by
# pm-stack/meta-exporter/install-laptop.sh. Semantics are replicated
# exactly; only provenance changes. The installer stays for machines
# without this module (the sandbox is out of scope), and its README
# points here for the amd laptop.
#
# History: the exporter was retired 2026-11-03 (code#5040, quota asked
# from OpenCode instead) and re-homed 2026-11-05 (code#5487, PR #6333)
# with the helium-cdp readiness probe and bounded recovery. The script
# installed at ~/atlas predates those fixes, so the next activation
# replaces it with the flake copy below — a bug fix, not a behaviour
# change in the units.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.meta-quota-export;
in
{
  options.services.meta-quota-export = {
    enable = lib.mkEnableOption "the Meta quota exporter (code#1301, re-homed by code#5487)";
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
        ExecStart = "${pkgs.nodejs}/bin/node %h/atlas/meta-quota-export/meta-quota-export.mjs --cdp-port 19222 --out-dir %h/atlas/meta-quota-export";
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

    # Provision the script at the exact path ExecStart references,
    # from the flake copy beside pm-stack/meta-exporter. The first
    # activation renames the installer's copy to *.backup (same
    # pattern as oc-password-sync.nix).
    home.file."atlas/meta-quota-export/meta-quota-export.mjs" = {
      source = ../../pm-stack/meta-exporter/meta-quota-export.mjs;
      executable = true;
    };
  };
}
