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

      # Log this run to /var/log/agent-rebuild/<UTC timestamp>.log (dir
      # 0750 and file 0640, root:wheel, so `user` can read it without
      # sudo) with latest.log pointing at it. Output still goes to the
      # terminal or, for the unit, the journal. Created after the lock so
      # two runs never race on it; a logging failure never stops a rebuild.
      log_dir=/var/log/agent-rebuild
      log_file=
      if mkdir -p "$log_dir" && [[ -d $log_dir && ! -L $log_dir ]] &&
        chown root:wheel "$log_dir" && chmod 0750 "$log_dir"; then
        # noclobber: never reuse an existing path; a second run in the
        # same second gets a pid suffix.
        stamp="$(date -u +%Y%m%dT%H%M%SZ)"
        candidate="$log_dir/$stamp.log"
        if ! (umask 0137 && set -o noclobber && : >"$candidate") 2>/dev/null; then
          candidate="$log_dir/$stamp-$$.log"
          (umask 0137 && set -o noclobber && : >"$candidate") 2>/dev/null || candidate=
        fi
        if [[ -n $candidate ]] && chgrp wheel "$candidate"; then
          log_file=$candidate
          ln -sfn "$(basename "$log_file")" "$log_dir/latest.log" || true
          printf '# agent-rebuild-switch log: started %s by %s; format: plain text, the messages of this command and nixos-rebuild stdout+stderr\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "''${SUDO_USER:-unit}" >>"$log_file" 2>/dev/null ||
            log_file= # a log that can't be written is skipped, never a reason to stop the switch
          # Keep the newest 30 (names sort by time).
          old_logs=("$log_dir"/[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z*.log)
          if ((''${#old_logs[@]} > 30)); then
            for old in "''${old_logs[@]:0:''${#old_logs[@]}-30}"; do
              if [[ -f $old && ! -L $old && $old != "$log_file" ]]; then
                rm -f -- "$old"
              fi
            done
          fi
        fi
      fi
      # A message to stderr (the terminal or the journal) and to the log.
      say() {
        printf '%s\n' "$*" >&2
        if [[ -n $log_file ]]; then
          printf '%s\n' "$*" >>"$log_file" || true
        fi
      }

      # lib/auth.sh raises this for large evaluations too.
      ulimit -n "$(ulimit -Hn)" || true

      if [[ -f ${tokenFile} && ! -L ${tokenFile} ]]; then
        token="$(head -n 1 ${tokenFile} | tr -d '\r\n')"
        if [[ ! $token =~ ^[A-Za-z0-9_]+$ ]]; then
          say "agent-rebuild-switch: ignoring ${tokenFile} (not a bare GitHub token)"
        # The header goes in on stdin so the token never shows in ps.
        elif [[ "$(printf 'Authorization: Bearer %s\n' "$token" |
          curl -s -o /dev/null -w '%{http_code}' --max-time 15 -H @- \
            https://api.github.com/rate_limit)" == 401 ]]; then
          # An expired or revoked token makes every GitHub fetch fail with
          # 401, even for public inputs; fetching without one still works.
          say "agent-rebuild-switch: GitHub rejects the token in ${tokenFile} (401); continuing without it"
          logger -t agent-rebuild-switch "token in ${tokenFile} rejected by GitHub (401); continuing without it"
        else
          export NIX_CONFIG="access-tokens = github.com=$token"
        fi
        unset token
      fi

      # The hosts evaluate unfree packages from inputs whose nixpkgs
      # config comes from the environment (Affinity via affinity-nix's
      # legacyPackages, antigravity-ide, and others); /etc/nix/nix.conf
      # sets pure-eval = false so the variable is honoured. ./rebuild
      # evaluates as `user`, whose session sets NIXPKGS_ALLOW_UNFREE=1
      # (nixos/environment), and CI's host eval sets it too. This command
      # evaluates as root from sudo or a systemd unit, where it is unset,
      # so evaluation refused those packages. Set it here for parity.
      export NIXPKGS_ALLOW_UNFREE=1

      logger -t agent-rebuild-switch "switch to ${flakeRef} started by ''${SUDO_USER:-unknown}; log: ''${log_file:-none}"
      # Nix resolves an installable that contains a slash against the
      # cwd, following a symlink there into the store. Run from root-owned
      # / so no caller can plant an entry that stands in for the flake
      # reference (the unit also sets WorkingDirectory=/).
      cd /
      set +e
      if [[ -n $log_file ]]; then
        # stdout+stderr into the log and on to stdout (terminal/journal).
        # The tee never decides the status; nixos-rebuild's does.
        nixos-rebuild switch --refresh --accept-flake-config --flake ${flakeRef} 2>&1 |
          { tee -a "$log_file" || true; }
        rc=''${PIPESTATUS[0]}
      else
        nixos-rebuild switch --refresh --accept-flake-config --flake ${flakeRef}
        rc=$?
      fi
      set -e
      if ((rc == 0)); then
        logger -t agent-rebuild-switch "switch to ${flakeRef} succeeded"
      else
        logger -t agent-rebuild-switch "switch to ${flakeRef} failed (exit $rc)"
      fi
      if [[ -n $log_file ]]; then
        printf '# agent-rebuild-switch log: finished %s exit status %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$rc" >>"$log_file" || true
        echo "agent-rebuild-switch log: $log_file" >&2
      fi
      exit "$rc"
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
  # Either way the run's output is also in /var/log/agent-rebuild/
  # (latest.log is the newest), readable by wheel, so `user` can read it.
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

  # CF-9 (#14949, #14107 M2): the laptop PM edition follows main on a
  # schedule, not on a human remembering to rebuild. An hourly timer
  # starts the same reviewed main-only switch above; the rebuild is
  # serialized by the switch's own lock, logs to
  # /var/log/agent-rebuild/, and a failed run simply retries next
  # hour. Hourly sits well inside the aggregator's six-hour
  # edition-drift alarm threshold (pm-stack/aggregator
  # edition-drift.ts, EDITION_DRIFT_STALE_MS), so a stalled follow is
  # alarmed by the sentinel long before it can silently become the
  # stale tier (#14059). Schedule of record:
  # doc/fleet-schedule-registry.md (pm-stack-follow-main.timer row).
  systemd.timers.pm-stack-follow-main = {
    description = "Follow main hourly via agent-rebuild-switch (CF-9, code#14949)";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar = "hourly";
      RandomizedDelaySec = "10m";
      # A laptop that slept through a fire catches up on wake, so a
      # merge made while it was closed is still followed unattended.
      Persistent = true;
      Unit = "agent-rebuild-switch.service";
    };
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
