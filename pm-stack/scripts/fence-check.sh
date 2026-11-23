#!/usr/bin/env bash
# fence-check — verify the old pm-stack writer is fenced before failover promotion.
#
# Implements runbook step 1 of doc/pm-stack/COLD-RESTORE-RUNBOOK.md for
# #3989 acceptance criterion 3 ("the old writer is fenced before failover
# promotion"): promotion may proceed only when none of the old host's
# writers can still accept writes. An unreachable old host is fenced by
# definition, and it must stay disabled before it is allowed to rejoin.
#
# FENCED means: none of the named writer containers is running and none
# of the named writer units is active. Anything unreadable reports
# UNFENCED (fail closed): this checker never reports FENCED when it
# cannot tell. It changes nothing; it only reads.
#
# Writer set (runbook step 1 + the 2026-11-05 ownership inventory):
#   containers: pm-stack-kaneo-1, pm-stack-postgres-1, pm-grist, pm-pad
#   units: docker-pm-grist.service, docker-pm-pad.service
#
# Argument contract (#3352): flags only, no positionals. `--opt value`
# and `--opt=value` behave identically. Anything else is a usage error
# (exit 2) before any check runs:
#   --units-dir DIR        read per-unit ActiveState from DIR/<unit> files
#                          (fixture mode; default: query systemctl live)
#   --containers-file FILE read `docker ps` style "<name> <state>" lines
#                          from FILE (fixture mode; default: query docker live)
#   --unreachable          old host is unreachable: fenced by definition
#   -h, --help             print usage and exit 0
set -uo pipefail

WRITER_CONTAINERS="pm-stack-kaneo-1 pm-stack-postgres-1 pm-grist pm-pad"
WRITER_UNITS="docker-pm-grist.service docker-pm-pad.service"

UNITS_DIR=""
CONTAINERS_FILE=""
UNREACHABLE=0

usage() {
  cat <<'USAGE'
usage: fence-check.sh [--units-dir DIR] [--containers-file FILE] [--unreachable] [-h|--help]

Verifies the old pm-stack writer is fenced (runbook step 1, #3989
criterion 3). Prints FENCED or UNFENCED with a reason.
Exit: 0 fenced, 1 unfenced, 2 usage error.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --unreachable)
      UNREACHABLE=1
      shift
      ;;
    --units-dir|--containers-file)
      opt="$1"
      [ "$#" -ge 2 ] || { echo "fence-check: $opt requires a value" >&2; exit 2; }
      val="$2"
      [ -n "$val" ] || { echo "fence-check: $opt requires a non-empty value" >&2; exit 2; }
      case "$opt" in
        --units-dir) UNITS_DIR="$val" ;;
        --containers-file) CONTAINERS_FILE="$val" ;;
      esac
      shift 2
      ;;
    --units-dir=*|--containers-file=*)
      opt="${1%%=*}"
      val="${1#*=}"
      [ -n "$val" ] || { echo "fence-check: $opt requires a non-empty value" >&2; exit 2; }
      case "$opt" in
        --units-dir) UNITS_DIR="$val" ;;
        --containers-file) CONTAINERS_FILE="$val" ;;
      esac
      shift
      ;;
    --*)
      echo "fence-check: unexpected argument '$1'" >&2
      usage >&2
      exit 2
      ;;
    *)
      echo "fence-check: unexpected argument '$1'" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ "$UNREACHABLE" -eq 1 ]; then
  echo "FENCED: old host unreachable — fenced by definition (runbook step 1); keep its writers disabled before it rejoins"
  exit 0
fi

offenders=""

# --- containers ---
if [ -n "$CONTAINERS_FILE" ]; then
  if [ ! -r "$CONTAINERS_FILE" ]; then
    echo "UNFENCED: cannot read containers fixture '$CONTAINERS_FILE' — refusing to report fenced when it cannot tell"
    exit 1
  fi
  running_names="$(awk '{print $1}' "$CONTAINERS_FILE")"
  for want in $WRITER_CONTAINERS; do
    if printf '%s\n' "$running_names" | grep -Fxq -- "$want"; then
      offenders="${offenders} container:$want"
    fi
  done
else
  if ! command -v docker >/dev/null 2>&1; then
    echo "UNFENCED: docker is unavailable — cannot establish container state, refusing to report fenced"
    exit 1
  fi
  ps_out=""
  if ! ps_out="$(docker ps --format '{{.Names}} {{.State}}' 2>/dev/null)"; then
    echo "UNFENCED: 'docker ps' failed — cannot establish container state, refusing to report fenced"
    exit 1
  fi
  for want in $WRITER_CONTAINERS; do
    if printf '%s\n' "$ps_out" | awk '{print $1}' | grep -Fxq -- "$want"; then
      offenders="${offenders} container:$want"
    fi
  done
fi

# --- units ---
if [ -n "$UNITS_DIR" ]; then
  if [ ! -d "$UNITS_DIR" ]; then
    echo "UNFENCED: units dir '$UNITS_DIR' is missing — refusing to report fenced when it cannot tell"
    exit 1
  fi
  for unit in $WRITER_UNITS; do
    if [ ! -r "$UNITS_DIR/$unit" ]; then
      echo "UNFENCED: no state for unit $unit in '$UNITS_DIR' — refusing to report fenced when it cannot tell"
      exit 1
    fi
    state="$(tr -d '[:space:]' <"$UNITS_DIR/$unit")"
    if [ "$state" = "active" ]; then
      offenders="${offenders} unit:$unit"
    fi
  done
else
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "UNFENCED: systemctl is unavailable — cannot establish unit state, refusing to report fenced"
    exit 1
  fi
  for unit in $WRITER_UNITS; do
    state=""
    state="$(systemctl show -p ActiveState --value "$unit" 2>/dev/null || true)"
    if [ -z "$state" ]; then
      state="$(systemctl --user show -p ActiveState --value "$unit" 2>/dev/null || true)"
    fi
    if [ -z "$state" ]; then
      echo "UNFENCED: cannot read state for unit $unit — refusing to report fenced when it cannot tell"
      exit 1
    fi
    if [ "$state" = "active" ]; then
      offenders="${offenders} unit:$unit"
    fi
  done
fi

if [ -n "$offenders" ]; then
  # shellcheck disable=SC2086
  echo "UNFENCED: old writer still active -$offenders — stop it before promotion (runbook step 1)"
  exit 1
fi

echo "FENCED: no writer container running and no writer unit active"
exit 0
