# laptop-scratch-sweep — daily reaping of opencode test/build
# scratch and mcpx binary dirs on the laptop (darwin-nix#373).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.laptop-scratch-sweep.enable = true.
#
# ~/.local/share/opencode/tmp accrues ~7,500 scratch entries a day
# (core-test dirs ~2.4 GB each, cargo targets) and nothing aged
# them out until the 2026-11-11 disk catch removed ~80 GB by hand;
# /tmp/mcpx-bin-* accumulates the same way in miniature. The sweep
# makes the catch's rules standing: opencode tmp entries older
# than 72h and mcpx-bin dirs older than 24h, once a day. The rules
# themselves live in ./laptop-scratch-sweep.sh; see its header for
# the safety bounds (fixed roots, depth-1, age-only).
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.laptop-scratch-sweep;
  sweepPkg = pkgs.writeShellScriptBin "laptop-scratch-sweep" (
    builtins.readFile ./laptop-scratch-sweep.sh
  );
  servicePath = lib.makeBinPath [
    pkgs.coreutils
    pkgs.findutils
  ];
in
{
  options.services.laptop-scratch-sweep = {
    enable = lib.mkEnableOption "the laptop scratch sweep (daily opencode-tmp / mcpx-bin reaping, darwin-nix#373)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.laptop-scratch-sweep = {
      Unit.Description = "laptop-scratch-sweep: reap opencode tmp (>72h) and mcpx-bin (>24h) scratch (darwin-nix#373)";
      Service = {
        Type = "oneshot";
        ExecStart = "${sweepPkg}/bin/laptop-scratch-sweep";
        Environment = "PATH=${servicePath}:/run/current-system/sw/bin";
        # The first runs can face days of backlog (thousands of
        # multi-GB entries); deletion is I/O-bound, so allow 2h.
        TimeoutStartSec = "2h";
        Nice = 10;
      };
    };

    systemd.user.timers.laptop-scratch-sweep = {
      Unit.Description = "laptop-scratch-sweep (daily)";
      Timer = {
        OnCalendar = "*-*-* 05:35:00";
        Persistent = true;
        RandomizedDelaySec = "15m";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
