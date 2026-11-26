# stable-entrypoints — stable deploy entrypoints for the OpenChamber
# and mcpx systemd user units (developing-today/code#14187).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.stable-entrypoints.enable = true.
#
# THE CONTRACT
#
# The OpenChamber and mcpx user units exec fixed paths under the
# state home, never a generation-specific store path:
#
#   openchamber.service  %h/.local/state/openchamber/start
#   mcpx.service         %h/.local/state/mcpx/bin/mcpx daemon --config <rendered config>
#
# (%h is systemd's spelling of the user's home; the state home is
# ${XDG_STATE_HOME:-$HOME/.local/state}, the same directory in any
# default environment.) Each stable path is a symlink to the current
# generation's real program — for OpenChamber the module's start
# wrapper (services.openchamber.startWrapper, launch semantics
# unchanged), for mcpx the very package the mcpx module would have
# exec'd directly (services.mcpx.package), started with the very
# rendered config the module renders (services.mcpx.configFile), so
# the daemon still receives its real config path.
#
# Home Manager activation owns first-time creation and validation
# of the links (./stable-entrypoints.sh): every switch repoints a
# link whose target changed, leaves an unchanged link untouched,
# and fails the switch outright when a target is missing or not
# executable. Because the unit files no longer change when only the
# service build changes, such a switch does not bounce the service;
# mcpx keeps its X-RestartIfChanged=false handover on top, and this
# module gives openchamber the same no-restart-on-switch setting.
#
# A deploy app (the option-(c) consumer, coordinated in
# dezren39/darwin-nix#1393) may atomically repoint a stable link,
# then restart and health-check the service. It must not write or
# replace the generated unit files — they stay Home Manager-owned.
#
# Scope: the laptop user's units (home-manager.users.user), the
# ones the deploy apps drive. The mcpx module's own default
# (pkgs/mcpx/modules) is unchanged for other consumers.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.stable-entrypoints;
  oc = config.services.openchamber;
  mcpxSvc = config.services.mcpx;

  # The (link, target) pairs the activation maintains, as flat
  # arguments for stable-entrypoints.sh: link paths relative to the
  # state home, targets the current generation's real programs.
  entrypointArgs =
    lib.optionals oc.enable [
      "openchamber/start"
      "${oc.startWrapper}/bin/openchamber-start"
    ]
    ++ lib.optionals mcpxSvc.enable [
      "mcpx/bin/mcpx"
      "${mcpxSvc.package}/bin/mcpx"
    ];
in
{
  options.services.stable-entrypoints = {
    enable = lib.mkEnableOption "stable deploy entrypoints for the OpenChamber and mcpx user units (code#14187)";
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      (lib.mkIf oc.enable {
        systemd.user.services.openchamber = {
          # A switch whose only change is a new OpenChamber build no
          # longer changes this unit at all (the ExecStart below is
          # constant); keep the mcpx/opencode no-bounce rule for any
          # other unit change too — redeploys go through the stable
          # link and an explicit restart by the deploy app.
          Unit."X-RestartIfChanged" = false;
          Service.ExecStart = lib.mkForce "%h/.local/state/openchamber/start";
        };
      })

      (lib.mkIf mcpxSvc.enable {
        systemd.user.services.mcpx.Service = {
          ExecStart = lib.mkForce "%h/.local/state/mcpx/bin/mcpx daemon --config ${mcpxSvc.configFile}";
          # Reload through the same stable path, so the unit names
          # no generation-specific executable at all. (The consumer
          # refinement in home/common/default.nix sets a store-path
          # ExecReload; this mkForce is the migration of that line.)
          ExecReload = lib.mkForce "%h/.local/state/mcpx/bin/mcpx reload";
        };
      })

      (lib.mkIf (entrypointArgs != [ ]) {
        # Between reloadSystemd (the new unit files are in place)
        # and mcpxDeploy (which may start or hand over the daemon
        # through the unit's start command, so the stable link must
        # already point at the new build when it runs).
        home.activation.stableEntrypoints = lib.hm.dag.entryBetween [ "mcpxDeploy" ] [ "reloadSystemd" ] ''
          run ${pkgs.bash}/bin/bash ${./stable-entrypoints.sh} \
            "$HOME/.local/state" ${lib.escapeShellArgs entrypointArgs}
        '';
      })
    ]
  );
}
