# worktree-reaper — hourly retirement of dead fleet worktrees on
# the laptop under Drew's 6-hour quiet rule
# (developing-today/code#3698).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.worktree-reaper.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720).
# Until now the unit was a plain file in ~/.config/systemd/user,
# installed by infra/worktree-reaper/systemd/install-units.sh from
# the pre-merge bootstrap worktree ~/git/code-wt-reaper, and its
# ExecStart pointed there. The fold's one intentional unit change:
# ExecStart now points at the canonical primary checkout
# (~/git/code/infra/worktree-reaper/...), retiring the bootstrap
# install per the worktree-reaper README. Every other field is
# replicated exactly. All reaper state (config overlay, run logs,
# the DISABLED kill switch) lives in ~/.local/state/worktree-reaper/
# and is unaffected by where the unit comes from; the script reads
# the config.json shipped beside it in infra/worktree-reaper and
# merges the state-dir overlay on top, as before.
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.worktree-reaper;
  home = config.home.homeDirectory;
in
{
  options.services.worktree-reaper = {
    enable = lib.mkEnableOption "the worktree reaper (hourly dead-worktree retirement, code#3698)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.worktree-reaper = {
      Unit = {
        Description = "worktree-reaper: retire dead fleet worktrees (6-hour quiet rule)";
        After = [ "network-online.target" ];
        Wants = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        ExecStart = "/usr/bin/env python3 ${home}/git/code/infra/worktree-reaper/bin/worktree_reaper.py";
        WorkingDirectory = "%h";
        TimeoutStartSec = "6h";
        Nice = 10;
      };
    };

    systemd.user.timers.worktree-reaper = {
      Unit.Description = "worktree-reaper (hourly)";
      Timer = {
        OnCalendar = "hourly";
        Persistent = true;
        RandomizedDelaySec = "10m";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
