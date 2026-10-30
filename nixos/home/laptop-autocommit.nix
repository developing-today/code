# laptop-autocommit — every-minute tracked-file sweeps for the
# laptop's personal repos, plus the daily untracked-file review
# (Drew, 2026-10-28; developing-today/code#1723). Mirrors the
# sandbox discipline: hatch-autocommit (tracked files only,
# committed + pushed promptly) and the daily untracked review
# (new files classified; oversized ones belong to the backup
# scripts, never to git).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.laptop-autocommit.enable = true.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.laptop-autocommit;
  home = config.home.homeDirectory;
  sweepPkg = pkgs.writeShellScriptBin "laptop-autocommit" (
    builtins.readFile ./laptop-autocommit.sh
  );
  reviewPkg = pkgs.writeShellScriptBin "laptop-untracked-review" (
    builtins.readFile ./laptop-untracked-review.sh
  );
  servicePath = lib.makeBinPath [
    pkgs.git
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
  ];
  repos = {
    openclaw = "${home}/.openclaw";
    hatch = "${home}/hatch";
    hatch-personal = "${home}/hatch-personal";
  };
  sweepService = name: path: {
    "laptop-autocommit-${name}" = {
      Unit.Description = "Autocommit tracked files in ${path} (code#1723)";
      Service = {
        Type = "oneshot";
        ExecStart = "${sweepPkg}/bin/laptop-autocommit ${path}";
        Environment = "PATH=${servicePath}:/run/current-system/sw/bin";
      };
    };
  };
  sweepTimer = name: _: {
    "laptop-autocommit-${name}" = {
      Unit.Description = "Autocommit ${name} every minute";
      Timer.OnCalendar = "*-*-* *:*:00";
      Install.WantedBy = [ "timers.target" ];
    };
  };
in
{
  options.services.laptop-autocommit = {
    enable = lib.mkEnableOption "laptop autocommit sweeps + daily untracked review";

    reviewOnCalendar = lib.mkOption {
      type = lib.types.str;
      default = "07:37";
      description = "Daily run time (local) for the untracked review.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services = lib.mkMerge [
      (lib.mkMerge (lib.mapAttrsToList sweepService repos))
      {
        laptop-untracked-review = {
          Unit.Description = "Daily untracked-file review for the laptop personal repos (code#1723)";
          Service = {
            Type = "oneshot";
            ExecStart = "${reviewPkg}/bin/laptop-untracked-review";
            Environment = "PATH=${servicePath}:/run/current-system/sw/bin";
            TimeoutStartSec = "10m";
          };
        };
      }
    ];

    systemd.user.timers = lib.mkMerge [
      (lib.mkMerge (lib.mapAttrsToList sweepTimer repos))
      {
        laptop-untracked-review = {
          Unit.Description = "Run the untracked review daily";
          Timer = {
            OnCalendar = cfg.reviewOnCalendar;
            Persistent = true;
            RandomizedDelaySec = "10m";
          };
          Install.WantedBy = [ "timers.target" ];
        };
      }
    ];
  };
}
