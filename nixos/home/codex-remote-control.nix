# codex-remote-control — Codex's app-server daemon with remote control
# enabled, declared in the flake (Drew, 2026-10-28; dezren39/nix#1028,
# developing-today/code#1685).
#
# Home-manager module: import it from the host config that manages the
# laptop user's services (the NixOS config in developing-today/code does)
# and set `services.codex-remote-control.enable = true`.
#
# Relocated 2026-10-28 from dezren39/nix (Drew's Darwin flake) into
# this flake, per Drew's rule that all laptop services are made by
# the code flake (code#1723).
#
# What it runs: Codex CLI's experimental remote control
# (`codex remote-control start|stop|pair`). `start` bootstraps Codex's
# managed app-server daemon (its own auto-updating package under
# ~/.codex/packages/app-server-daemon, currently 0.164.0 against the
# system's CLI 0.160.0) as a detached pid-backend process serving the
# control socket ~/.codex/app-server-control/app-server-control.sock
# with remote control enabled, so the machine shows up as a remote
# Codex environment (pair with `codex remote-control pair`). The
# daemon's own state (settings.json remoteControlEnabled, daemon.pid)
# lives under ~/.codex/app-server-daemon and stays Codex-managed.
#
# Why a oneshot: `remote-control start` daemonizes and exits, and
# `remote-control stop` is its matching teardown — the vendor CLI owns
# the daemon lifecycle (pid backend, auto-update, pid-update loop), so
# the unit wraps exactly those two commands instead of re-declaring
# the daemon as a Type=simple process systemd would mis-track. The
# unit reads active while the daemon is up; check the real surface
# with `codex app-server daemon version`.
#
# Before this module the daemon was started ad-hoc only: it ran
# 2026-10-23..26 and died with the session, and nothing restarted it.
#
# Watchdog (#1845): the oneshot unit above still cannot notice when
# the daemon dies mid-session (crash, OOM, failed auto-update) — it
# reads active forever. A user timer therefore runs a companion
# check service every few minutes: it probes the daemon through the
# CLI liveness surface (`codex app-server daemon version`), logs
# the verdict to the journal, and runs `remote-control start` when
# the daemon is down. The check lives in
# codex-remote-control-watchdog.sh.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.codex-remote-control;
  watchdogPkg = pkgs.writeShellScriptBin "codex-remote-control-watchdog" (
    builtins.readFile ./codex-remote-control-watchdog.sh
  );
in
{
  options.services.codex-remote-control = {
    enable = lib.mkEnableOption "the Codex app-server daemon with remote control enabled";

    watchdogInterval = lib.mkOption {
      type = lib.types.str;
      default = "5m";
      description = "How often the watchdog timer probes daemon liveness (a systemd time span, used as the timer OnUnitActiveSec).";
    };

    codexPath = lib.mkOption {
      type = lib.types.str;
      default = "/run/current-system/sw/bin/codex";
      description = "Path of the codex CLI whose remote-control commands drive the daemon. The laptop installs codex into the system profile (developing-today/code, latestCli.codex), so the default is its /run/current-system path, stable across switches.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.codex-remote-control = {
      Unit = {
        Description = "Codex app-server daemon with remote control enabled (nix#1028, code#1685)";
        After = [ "network-online.target" ];
        Wants = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${cfg.codexPath} remote-control start";
        ExecStop = "${cfg.codexPath} remote-control stop";
      };
      Install.WantedBy = [ "default.target" ];
    };

    systemd.user.services.codex-remote-control-watchdog = {
      Unit = {
        Description = "Codex app-server daemon liveness check - restart it if it died (code#1845)";
        After = [ "network-online.target" ];
        Wants = [ "network-online.target" ];
      };
      Service = {
        Type = "oneshot";
        ExecStart = "${watchdogPkg}/bin/codex-remote-control-watchdog ${cfg.codexPath}";
      };
    };

    systemd.user.timers.codex-remote-control-watchdog = {
      Unit.Description = "Probe the Codex app-server daemon liveness periodically (code#1845)";
      Timer = {
        OnBootSec = "2m";
        OnUnitActiveSec = cfg.watchdogInterval;
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
