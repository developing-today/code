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
      pkgs.curl
      pkgs.util-linux
      pkgs.systemd
    ];
    text = ''
      # Argument contract (AGENTS.md, #3352): this command takes no
      # arguments. The sudo rule forbids them and the unit passes none.
      if (($# != 0)); then
        echo "usage: agent-rebuild-switch   (takes no arguments; or start agent-rebuild-switch.service)" >&2
        exit 2
      fi
      if ((EUID != 0)); then
        echo "agent-rebuild-switch: run it with sudo, or start agent-rebuild-switch.service" >&2
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
        if [[ ! $token =~ ^[A-Za-z0-9_]+$ ]]; then
          echo "agent-rebuild-switch: ignoring ${tokenFile} (not a bare GitHub token)" >&2
        # The header goes in on stdin so the token never shows in ps.
        elif [[ "$(printf 'Authorization: Bearer %s\n' "$token" |
          curl -s -o /dev/null -w '%{http_code}' --max-time 15 -H @- \
            https://api.github.com/rate_limit)" == 401 ]]; then
          # An expired or revoked token makes every GitHub fetch fail with
          # 401, even for public inputs; fetching without one still works.
          echo "agent-rebuild-switch: GitHub rejects the token in ${tokenFile} (401); continuing without it" >&2
          logger -t agent-rebuild-switch "token in ${tokenFile} rejected by GitHub (401); continuing without it"
        else
          export NIX_CONFIG="access-tokens = github.com=$token"
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
  # Passwordless rebuilds for agents on this laptop, two ways in, both
  # running the same no-argument wrapper (switch to the amd config on
  # main) and nothing else:
  #   - `sudo agent-rebuild-switch` (sudo rule below), and
  #   - `systemctl start --no-block agent-rebuild-switch.service`, which a
  #     polkit rule lets `user` start (only start) without sudo, so sudo
  #     can stay blocked for agents in Desktop Commander.
  # Follow a service run with `journalctl -u agent-rebuild-switch -f`.
  environment.systemPackages = [ agentRebuildSwitch ];

  security.sudo.extraRules = [
    {
      users = [ "user" ];
      commands = [
        {
          # `""` after the path is sudoers for "no arguments allowed".
          command = ''${agentRebuildSwitch}/bin/agent-rebuild-switch ""'';
          options = [ "NOPASSWD" ];
        }
      ];
    }
  ];

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
