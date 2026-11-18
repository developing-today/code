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
    # No inherited PATH: under sudo the caller's PATH would otherwise
    # decide which nix/git/nixos-rebuild run as root.
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
      # arguments. The sudo rule also forbids them; this is the second wall.
      if (($# != 0)); then
        echo "usage: sudo agent-rebuild-switch   (takes no arguments)" >&2
        exit 2
      fi
      if ((EUID != 0)); then
        echo "agent-rebuild-switch: run it with sudo" >&2
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
  # Passwordless rebuilds for agents on this laptop, scoped to one
  # command: `sudo agent-rebuild-switch` (no arguments) switches the
  # system to the amd config on main. Nothing else becomes passwordless:
  # ./rebuild, nixos-rebuild with its own flags, and every other sudo
  # still ask for the password.
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
}
