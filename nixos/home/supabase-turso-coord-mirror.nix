# supabase-turso-coord-mirror — the daily Supabase -> Turso coord
# mirror (developing-today/code#1723, item 3 remainder).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.supabase-turso-coord-mirror.enable = true.
#
# The mirror tooling moved to the laptop with the Tier-2 execution
# migration (code#1723) but was never scheduled: no timer and no
# cron ran it, and its state.json watermark sat at 2026-10-28 while
# the duty silently did not happen. Declared here per Drew's rule
# that all laptop services are made by the code flake (code#1723),
# following the cold-store / supabase-backup pattern in
# laptop-backups.nix: the script stays at
# ~/tools/supabase-mirror/sync_agent_to_turso.py, the path its
# author maintains, and credentials come from Bitwarden by name at
# runtime (the script's own bws fetches), never from the store.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.supabase-turso-coord-mirror;
  home = config.home.homeDirectory;
  servicePath = lib.makeBinPath [
    pkgs.coreutils
    pkgs.curl
    pkgs.python3
    pkgs.systemd
  ];
in
{
  options.services.supabase-turso-coord-mirror = {
    enable = lib.mkEnableOption "the daily Supabase -> Turso coord mirror (code#1723)";

    onCalendar = lib.mkOption {
      type = lib.types.str;
      default = "04:24";
      description = "Daily run time (local) for the coord mirror.";
    };

    script = lib.mkOption {
      type = lib.types.str;
      default = "${home}/tools/supabase-mirror/sync_agent_to_turso.py";
      description = "Path of the coord-mirror sync script to run.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.supabase-turso-coord-mirror = {
      Unit = {
        Description = "Supabase -> Turso coord mirror (daily sync_agent_to_turso) (code#1723)";
        After = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        ExecStart = cfg.script;
        TimeoutStartSec = "30m";
        Environment = "PATH=${servicePath}:${home}/tools/bin:/run/current-system/sw/bin";
      };
    };

    systemd.user.timers.supabase-turso-coord-mirror = {
      Unit.Description = "Run the Supabase -> Turso coord mirror daily (code#1723)";
      Timer = {
        OnCalendar = cfg.onCalendar;
        Persistent = true;
        RandomizedDelaySec = "15m";
        Unit = "supabase-turso-coord-mirror.service";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
