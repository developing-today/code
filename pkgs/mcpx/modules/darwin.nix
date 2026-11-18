# SPDX-License-Identifier: MIT OR Apache-2.0
#
# mcpx module — nix-darwin (code#4801).
#
# Canonical home: developing-today/code, pkgs/mcpx/modules. The
# launchd rendering of the daemon, ported from the hand-rolled agent
# dezren39/nix's darwin configuration ran (configuration.nix,
# launchd.user.agents.mcpx). Same service semantics as the NixOS and
# home-manager modules in this directory: start at login
# (RunAtLoad), keep running (KeepAlive), a 20 s stop window
# (ExitTimeOut — mcpx's stop drains HTTP and stops its server
# processes, all bounded by limits.execDrainTimeout, so launchd's
# 5 s default cut every restart off mid-shutdown), $HOME as the
# working directory (mcpx keys a daemon by the config files it finds
# searching up from where it starts, and a CLI run from $HOME finds
# the same set), and the rendered config named with `--config`.
#
# The program is a fixed path, /usr/local/libexec/mcpx/mcpx, NOT a
# store path — deliberately. macOS lists a background item under its
# executable's file name and tracks it by path: a store path changes
# on every mcpx build, giving the agent a new name and executable
# each time (dezren39/nix counted 9 path changes in three weeks).
# The fixed file is a copy of the wrapper below, refreshed by
# extraActivation before launchd is set up; since the plist no
# longer changes when mcpx does, postActivation kickstarts the agent
# whenever the copy changed, including in an earlier activation that
# failed before reaching postActivation (a marker file carries it).
#
# The wrapper stops whatever already serves this config's socket
# before starting: launchd never runs two copies of one job, so a
# running daemon was started by something else (the CLI's autostart,
# the OpenCode fork's mcpx plugin — developing-today/opencode#134).
# Left running, it holds the socket, this daemon exits "a daemon is
# already listening", and KeepAlive respawns it forever.
#
# Two things the dezren39/nix original had that this canonical
# module deliberately does not: its service-events start hook (a
# local observability embellishment of that repo, which can layer it
# back by overriding the agent) and its hand-written user config —
# the module starts the daemon on the rendered `configFile` and
# points clients at it with MCPX_CONFIG, exactly like the NixOS
# module, so every platform computes the same daemon socket.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.mcpx;
  home = config.users.users.${config.system.primaryUser}.home;
  agentPath = "/usr/local/libexec/mcpx/mcpx";
  agent = pkgs.writeShellScript "mcpx" ''
    ${cfg.package}/bin/mcpx --config ${cfg.configFile} stop || true
    exec ${cfg.package}/bin/mcpx daemon --config ${cfg.configFile}
  '';
in
{
  imports = [ ./common.nix ];

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.environmentFile == null;
        message = "services.mcpx.environmentFile has no launchd equivalent; on darwin, hand the daemon credentials through files its servers point at instead.";
      }
    ];

    environment.systemPackages = [ cfg.package ];

    # Point every other mcpx client at the same config the daemon
    # runs: MCPX_CONFIG names exactly one file and bypasses
    # discovery, so a CLI or MCP client in a user session computes
    # the same daemon socket.
    # String interpolation, not the bare value: nix-darwin's
    # launchd.user.envVariables is typed string with no derivation
    # coercion, unlike the sessionVariables options on the other
    # platforms, which accept cfg.configFile directly.
    launchd.user.envVariables.MCPX_CONFIG = "${cfg.configFile}";

    launchd.user.agents.mcpx = {
      serviceConfig = {
        ProgramArguments = [ agentPath ];
        EnvironmentVariables = {
          HOME = home;
        }
        // cfg.environment;
        KeepAlive = true;
        RunAtLoad = true;
        ExitTimeOut = 20;
        WorkingDirectory = home;
        # Not /tmp: that is emptied at boot, taking the evidence of
        # what went wrong before it.
        StandardErrorPath = "${home}/Library/Logs/mcpx.log";
        StandardOutPath = "${home}/Library/Logs/mcpx.log";
      };
    };

    # Refresh the fixed-path copy before launchd is set up, and
    # remember whether it changed so postActivation can kickstart
    # the agent (its plist no longer changes when mcpx does).
    # Root-owned and not user-writable, like the store copy it
    # mirrors.
    #
    # "Changed" is a file, not a shell variable: a variable lived only
    # for one activation, so when anything between extraActivation and
    # postActivation failed (a brew bundle error, a helper exit), the
    # copy was already current on the next switch, cmp found nothing to
    # do, and the daemon kept running the old binary and the old config
    # indefinitely (dezren39/darwin-nix, 2026-11-17: a daemon on a config
    # three switches stale, every client "no daemon reachable"). The
    # marker stays until a postActivation actually restarts the agent.
    system.activationScripts.extraActivation.text = lib.mkAfter ''
      # mcpx-agent-install-begin
      if ! /usr/bin/cmp -s ${agent} ${agentPath}; then
        echo "installing ${agentPath}" >&2
        /bin/mkdir -p "$(/usr/bin/dirname ${agentPath})"
        /usr/bin/install -o root -g wheel -m 0755 ${agent} ${agentPath}.tmp
        /bin/mv -f ${agentPath}.tmp ${agentPath}
        /usr/bin/touch ${agentPath}.restart-pending
      fi
      # mcpx-agent-install-end
    '';

    system.activationScripts.postActivation.text = lib.mkAfter ''
      # mcpx-agent-restart-begin
      if [ -e ${agentPath}.restart-pending ]; then
        if /bin/launchctl kickstart -k "gui/$(/usr/bin/id -u ${config.system.primaryUser})/org.nixos.mcpx"; then
          /bin/rm -f ${agentPath}.restart-pending
        else
          echo "mcpx: could not restart org.nixos.mcpx; will retry on the next activation" >&2
        fi
      fi
      # mcpx-agent-restart-end
    '';
  };
}
