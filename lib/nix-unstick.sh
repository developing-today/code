#!/usr/bin/env bash
# Detects and (optionally) kills processes holding flocks on the nix store's
# big-lock (/nix/var/nix/db/big-lock). Orphaned nix daemon workers (reparented
# to PID 1 after their client was killed) hold shared locks forever and starve
# any root-side nix command that needs the exclusive lock.
#
# Usage:
#   nix-unstick.sh            # list holders/waiters of big-lock
#   nix-unstick.sh --kill     # kill ORPHANED (ppid=1) holders
#   nix-unstick.sh --kill-all # kill ALL holders except the nix-daemon itself
#
# Exit codes: 0 = lock free (or now free), 1 = contention still present
set -Eeuo pipefail

BIG_LOCK=/nix/var/nix/db/big-lock
MODE="${1:-check}"

if [[ ! -e $BIG_LOCK ]]; then
  echo "error: $BIG_LOCK not found" >&2
  exit 1
fi

inode=$(stat -c %i "$BIG_LOCK")

holders=""
waiters=""
while IFS= read -r raw; do
  line="${raw#"${raw%%[![:space:]]*}"}" # trim leading whitespace
  kind=holder
  pid_field=5
  # waiter lines look like: "19: -> FLOCK ADVISORY WRITE <pid> ..."
  if [[ $line == *"-> "* ]]; then
    kind=waiter
    line="${line#*-> }"
    pid_field=4
  fi
  [[ $line != *"$inode"* ]] && continue
  pid=$(echo "$line" | awk -v f="$pid_field" '{print $f}')
  if [[ $kind == "holder" ]]; then
    holders="$holders $pid"
  else
    waiters="$waiters $pid"
  fi
done </proc/locks

trim() { echo "$1" | tr ' ' '\n' | sed '/^$/d' | sort -u | tr '\n' ' '; }
holders=$(trim "${holders:- }")
waiters=$(trim "${waiters:- }")

describe() {
  local p=$1 ppid cmd
  ppid=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
  cmd=$(tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null | head -c 90)
  echo "pid=$p ppid=${ppid:-?} cmd=${cmd:-<gone>}"
}

echo "big-lock inode: $inode"
echo "holders:${holders:- none}"
for p in $holders; do echo "  $(describe "$p")"; done
echo "waiters:${waiters:- none}"
for p in $waiters; do echo "  $(describe "$p")"; done

daemon_pid=$(systemctl show nix-daemon -p MainPID --value 2>/dev/null)

case "$MODE" in
check) ;;
--kill)
  for p in $holders; do
    [[ $p == "$daemon_pid" ]] && continue # never kill the daemon itself
    ppid=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
    if [[ $ppid == "1" ]]; then
      echo "killing orphaned holder: $p"
      kill "$p"
    fi
  done
  # orphaned daemon workers block SIGTERM/SIGINT; escalate to SIGKILL for survivors
  sleep 2
  for p in $holders; do
    [[ $p == "$daemon_pid" ]] && continue
    ppid=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
    if [[ $ppid == "1" && -d /proc/$p ]]; then
      echo "SIGKILL survivor: $p"
      kill -9 "$p"
    fi
  done
  ;;
--kill-all)
  for p in $holders; do
    [[ $p == "$daemon_pid" ]] && continue # never kill the daemon itself
    echo "killing holder: $p"
    kill "$p"
  done
  sleep 2
  for p in $holders; do
    [[ $p == "$daemon_pid" ]] && continue
    if [[ -d /proc/$p ]]; then
      echo "SIGKILL survivor: $p"
      kill -9 "$p"
    fi
  done
  ;;
*)
  echo "usage: $0 [--kill|--kill-all]" >&2
  exit 1
  ;;
esac

sleep 1
# re-read: contention = some process is WAITING for the lock
if grep -qE "^[[:space:]]*[0-9]+: -> FLOCK.*$inode" /proc/locks; then
  echo "status: STILL CONTENDED (waiters remain)"
  exit 1
fi
echo "status: lock free"
