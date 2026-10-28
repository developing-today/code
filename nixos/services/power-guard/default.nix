{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.power-guard;
in
{
  # power-guard: gate UPower's critical-power hybrid-sleep behind a
  # sustained-reading confirmation (code#1687).
  #
  # On 2026-10-28 upowerd hybrid-slept this machine 06:12 -> 08:53 on a
  # battery reading that could not have been real (99% on AC minutes
  # after resume; no low-battery warnings logged). UPower acts on a
  # single sample and has no confirmation setting. The guard polls the
  # battery, holds a block inhibitor on sleep while an unplug looks
  # critical, and only lets a *sustained* critical state (300s, or 60s
  # at a true 2%) put the machine to hybrid-sleep — issuing the sleep
  # itself. On AC it can never fire. UPower's own detection and
  # thresholds stay untouched.
  options.services.power-guard = {
    enable = lib.mkEnableOption "power-guard battery confirmation service";
  };

  config = lib.mkIf cfg.enable {
    systemd.services.power-guard = {
      description = "Confirm critical battery readings before UPower can hybrid-sleep the machine";
      wantedBy = [ "multi-user.target" ];
      after = [ "upower.service" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python3 ${./power-guard.py}";
        Restart = "always";
        RestartSec = "10s";
      };
    };
  };
}
