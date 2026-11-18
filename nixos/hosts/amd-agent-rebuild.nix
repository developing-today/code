{
  config,
  pkgs,
  ...
}:
let
  # The only source this command will ever switch the laptop to: the
  # amd configuration as merged on main. It deliberately does NOT build
  # from the local checkout (~/git/code): that tree is writable by every
  # agent running as `user`, so a passwordless switch from it would let
  # any of them become root. Building from main means only reviewed,
  # merged changes can reach the running system through this path.
  flakeRef = "github:developing-today/code/main#amd";

  # Same token file ./rebuild (lib/auth.sh) already uses. Read as root
  # inside the command, validated as a bare token so a stray line in it
  # cannot inject other nix.conf settings into a root nix invocation.
  tokenFile = "/home/user/auth";

  agentRebuildSwitch = pkgs.writeShellApplication {
    name = "agent-rebuild-switch";
    # No inherited PATH: the caller's environment must never decide
    # which nix/git/nixos-rebuild run as root.
    inheritPath = false;
    runtimeInputs = [
      config.system.build.nixos-rebuild
      config.nix.package
      pkgs.git
      pkgs.coreutils
      pkgs.util-linux
      pkgs.systemd
    ];
    text = ''
      # Argument contract (AGENTS.md, #3352): this command takes no
      # arguments; the systemd unit passes none.
      if (($# != 0)); then
        echo "usage: agent-rebuild-switch   (takes no arguments; start agent-rebuild-switch.service)" >&2
        exit 2
      fi
      if ((EUID != 0)); then
        echo "agent-rebuild-switch: must run as root (start agent-rebuild-switch.service)" >&2
        exit 1
      fi

      # One rebuild at a time.
      exec 9>/run/agent-rebuild-switch.lock
      if ! flock -n 9; then
        echo "agent-rebuild-switch: another rebuild is already running" >&2
        exit 1
      fi

      # lib/auth.sh raises this for large evaluations too.
      ulimit -n "$(ulimit -Hn)" || true

      if [[ -f ${tokenFile} && ! -L ${tokenFile} ]]; then
        token="$(head -n 1 ${tokenFile} | tr -d '\r\n')"
        if [[ $token =~ ^[A-Za-z0-9_]+$ ]]; then
          export NIX_CONFIG="access-tokens = github.com=$token"
        else
          echo "agent-rebuild-switch: ignoring ${tokenFile} (not a bare GitHub token)" >&2
        fi
        unset token
      fi

      logger -t agent-rebuild-switch "switch to ${flakeRef} started by ''${SUDO_USER:-unknown}"
      # Nix resolves an installable that contains a slash against the
      # cwd, following a symlink there into the store. Run from root-owned
      # / so no caller can plant an entry that stands in for the flake
      # reference (the unit also sets WorkingDirectory=/).
      cd /
      if nixos-rebuild switch --refresh --accept-flake-config --flake ${flakeRef}; then
        logger -t agent-rebuild-switch "switch to ${flakeRef} succeeded"
      else
        rc=$?
        logger -t agent-rebuild-switch "switch to ${flakeRef} failed (exit $rc)"
        exit "$rc"
      fi
    '';
  };
in
{
  # Passwordless rebuilds for agents on this laptop, without sudo.
  # `systemctl start --no-block agent-rebuild-switch.service` runs the
  # wrapper above as root; it switches the system to the amd config on
  # main and takes no input. A polkit rule lets `user` start (only start)
  # this one unit, so sudo can stay blocked for agents entirely.
  # Follow a run with `journalctl -u agent-rebuild-switch -f`.
  systemd.services.agent-rebuild-switch = {
    description = "Switch amd to the config on main (agent-triggered)";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${agentRebuildSwitch}/bin/agent-rebuild-switch";
      WorkingDirectory = "/";
      Environment = [ "HOME=/root" ];
    };
    # The switch this unit runs must not stop or restart the unit itself.
    restartIfChanged = false;
    stopIfChanged = false;
  };

  security.polkit.extraConfig = ''
    polkit.addRule(function (action, subject) {
      if (
        action.id == "org.freedesktop.systemd1.manage-units" &&
        action.lookup("unit") == "agent-rebuild-switch.service" &&
        action.lookup("verb") == "start" &&
        subject.user == "user"
      ) {
        return polkit.Result.YES;
      }
    });
  '';
}
