#!/usr/bin/env bash
# bootstrap.sh — bring up any fleet seat kind (developing-today/code, #12236).
#
# Operator script argument contract (#3352): every argument is declared and
# strictly parsed before any subprocess, network, file write, or lock step.
# Unknown flags, missing/empty option values, --opt= with empty value, and stray
# positional arguments exit 2 naming the offending token.
#
# Seat kinds:
#   linux           plain Linux box (default kind: generic)
#   codespace       GitHub Codespace (default kind: generic, name: codespace)
#   laptop-service  resident laptop service (default kind: laptop, name: laptop)
#   muse            Muse sandbox (delegates to scripts/muse/bootstrap.sh)
#
# Usage:
#   bash scripts/fleet/bootstrap.sh --seat <kind> [options]
#   bash scripts/fleet/bootstrap-linux.sh [options]
#   bash scripts/fleet/bootstrap-codespace.sh [options]
#   bash scripts/fleet/bootstrap-laptop-service.sh [options]
#
# Options:
#   --seat <seat>            linux | codespace | laptop-service | muse (default: linux)
#   --kind <agent-kind>      fleet agent kind passed to `fleet agent enroll`
#   --name <agent-name>      stable agent name ([a-z0-9][a-z0-9._-]{0,62})
#   --groups <g1,g2>         comma-separated groups for `fleet agent enroll`
#   --bin-dir <dir>          install directory for `fleet` (default: ~/.local/bin)
#   --fleet-url <url>        release URL for static fleet binary
#   --fleet-sha256 <hex>     expected 64-hex-char SHA-256 of the fleet binary
#   --profile <path>         explicit profile path passed to `fleet agent enroll`
#   --max-iterations <N>     max `fleet agent next` loop iterations (default: 1; 0 = none)
#   --no-next                enroll + doctor only; print the three-line loop
#   --dry-run                pass --dry-run to `fleet agent enroll`; skip doctor/next
#   -h, --help               show this help and exit 0
#
# Three-line agent loop:
#   fleet agent next --agent <name>    # exit 5 means no work on any rung this run
#   # do the task it returns, then run its `then` lines
#   # repeat while the run has budget
#
# Exit: 0 ok (including `fleet agent next` exit 5), 1 runtime/verification failure, 2 usage error.
set -euo pipefail

DEFAULT_FLEET_URL="https://github.com/developing-today/code/releases/download/fleet-static-ad0707b/fleet-x86_64-linux-musl"
DEFAULT_FLEET_SHA256="762bac6318329f202e47f8aeb7727cf6351e22ca93c278fd4d95377febcfa5f2"

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  sed -n '2,/^set -euo pipefail/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

die_usage() {
  echo "fleet-bootstrap: usage error: $*" >&2
  exit 2
}

die() {
  echo "fleet-bootstrap: error: $*" >&2
  exit 1
}

# ---- strict argument parsing (#3352) — no side effects before this completes ----
SEAT=""
KIND=""
NAME=""
AGENT_GROUPS=""
BIN_DIR=""
FLEET_URL=""
FLEET_SHA256=""
PROFILE=""
MAX_ITERATIONS=""
NO_NEXT=0
DRY_RUN=0
MUSE_PASSTHROUGH=()

need_value() {
  local flag="$1" val="${2-}" has_eq="$3"
  if [ "$has_eq" = "1" ]; then
    [ -n "$val" ] || die_usage "$flag requires a non-empty value"
  else
    [ $# -ge 4 ] || die_usage "$flag requires a value"
    local next="$4"
    case "$next" in
      ""|-*) die_usage "$flag requires a non-empty value (got '${next}')" ;;
    esac
  fi
}

# If `--seat muse` is explicitly the first option (or any --seat=muse), hand off
# directly to scripts/muse/bootstrap.sh after checking that no conflicting seat was given.
RAW_ARGS=("$@")

# Pre-scan only for --seat=muse / --seat muse vs another --seat before forwarding.
# Keep non-seat args intact if delegating to scripts/muse/bootstrap.sh.
i=0
while [ $i -lt ${#RAW_ARGS[@]} ]; do
  arg="${RAW_ARGS[$i]}"
  case "$arg" in
    --seat=*)
      val="${arg#--seat=}"
      [ -n "$val" ] || die_usage "--seat requires a non-empty value"
      [ -z "$SEAT" ] || die_usage "duplicate --seat"
      SEAT="$val"
      ;;
    --seat)
      [ $((i + 1)) -lt ${#RAW_ARGS[@]} ] || die_usage "--seat requires a value"
      val="${RAW_ARGS[$((i + 1))]}"
      case "$val" in ""|-*) die_usage "--seat requires a value" ;; esac
      [ -z "$SEAT" ] || die_usage "duplicate --seat"
      SEAT="$val"
      i=$((i + 1))
      ;;
    *)
      MUSE_PASSTHROUGH+=("$arg")
      ;;
  esac
  i=$((i + 1))
done

if [ "$SEAT" = "muse" ]; then
  MUSE_SCRIPT="$SELF_DIR/../muse/bootstrap.sh"
  [ -f "$MUSE_SCRIPT" ] || die "missing Muse bootstrap at $MUSE_SCRIPT"
  exec bash "$MUSE_SCRIPT" "${MUSE_PASSTHROUGH[@]}"
fi

SEAT=""
while [ $# -gt 0 ]; do
  arg="$1"
  case "$arg" in
    -h|--help)
      usage 0
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --no-next)
      NO_NEXT=1
      shift
      ;;
    --seat=*)
      val="${arg#--seat=}"
      [ -n "$val" ] || die_usage "--seat requires a non-empty value"
      [ -z "$SEAT" ] || die_usage "duplicate --seat"
      SEAT="$val"
      shift
      ;;
    --seat)
      [ $# -ge 2 ] || die_usage "--seat requires a value"
      case "$2" in ""|-*) die_usage "--seat requires a non-empty value" ;; esac
      [ -z "$SEAT" ] || die_usage "duplicate --seat"
      SEAT="$2"
      shift 2
      ;;
    --kind=*)
      val="${arg#--kind=}"
      [ -n "$val" ] || die_usage "--kind requires a non-empty value"
      [ -z "$KIND" ] || die_usage "duplicate --kind"
      KIND="$val"
      shift
      ;;
    --kind)
      [ $# -ge 2 ] || die_usage "--kind requires a value"
      case "$2" in ""|-*) die_usage "--kind requires a non-empty value" ;; esac
      [ -z "$KIND" ] || die_usage "duplicate --kind"
      KIND="$2"
      shift 2
      ;;
    --name=*)
      val="${arg#--name=}"
      [ -n "$val" ] || die_usage "--name requires a non-empty value"
      [ -z "$NAME" ] || die_usage "duplicate --name"
      NAME="$val"
      shift
      ;;
    --name)
      [ $# -ge 2 ] || die_usage "--name requires a value"
      case "$2" in ""|-*) die_usage "--name requires a non-empty value" ;; esac
      [ -z "$NAME" ] || die_usage "duplicate --name"
      NAME="$2"
      shift 2
      ;;
    --groups=*)
      val="${arg#--groups=}"
      [ -n "$val" ] || die_usage "--groups requires a non-empty value"
      [ -z "$AGENT_GROUPS" ] || die_usage "duplicate --groups"
      AGENT_GROUPS="$val"
      shift
      ;;
    --groups)
      [ $# -ge 2 ] || die_usage "--groups requires a value"
      case "$2" in ""|-*) die_usage "--groups requires a non-empty value" ;; esac
      [ -z "$AGENT_GROUPS" ] || die_usage "duplicate --groups"
      AGENT_GROUPS="$2"
      shift 2
      ;;
    --bin-dir=*)
      val="${arg#--bin-dir=}"
      [ -n "$val" ] || die_usage "--bin-dir requires a non-empty value"
      [ -z "$BIN_DIR" ] || die_usage "duplicate --bin-dir"
      BIN_DIR="$val"
      shift
      ;;
    --bin-dir)
      [ $# -ge 2 ] || die_usage "--bin-dir requires a value"
      case "$2" in ""|-*) die_usage "--bin-dir requires a non-empty value" ;; esac
      [ -z "$BIN_DIR" ] || die_usage "duplicate --bin-dir"
      BIN_DIR="$2"
      shift 2
      ;;
    --fleet-url=*)
      val="${arg#--fleet-url=}"
      [ -n "$val" ] || die_usage "--fleet-url requires a non-empty value"
      [ -z "$FLEET_URL" ] || die_usage "duplicate --fleet-url"
      FLEET_URL="$val"
      shift
      ;;
    --fleet-url)
      [ $# -ge 2 ] || die_usage "--fleet-url requires a value"
      case "$2" in ""|-*) die_usage "--fleet-url requires a non-empty value" ;; esac
      [ -z "$FLEET_URL" ] || die_usage "duplicate --fleet-url"
      FLEET_URL="$2"
      shift 2
      ;;
    --fleet-sha256=*)
      val="${arg#--fleet-sha256=}"
      [ -n "$val" ] || die_usage "--fleet-sha256 requires a non-empty value"
      [ -z "$FLEET_SHA256" ] || die_usage "duplicate --fleet-sha256"
      FLEET_SHA256="$val"
      shift
      ;;
    --fleet-sha256)
      [ $# -ge 2 ] || die_usage "--fleet-sha256 requires a value"
      case "$2" in ""|-*) die_usage "--fleet-sha256 requires a non-empty value" ;; esac
      [ -z "$FLEET_SHA256" ] || die_usage "duplicate --fleet-sha256"
      FLEET_SHA256="$2"
      shift 2
      ;;
    --profile=*)
      val="${arg#--profile=}"
      [ -n "$val" ] || die_usage "--profile requires a non-empty value"
      [ -z "$PROFILE" ] || die_usage "duplicate --profile"
      PROFILE="$val"
      shift
      ;;
    --profile)
      [ $# -ge 2 ] || die_usage "--profile requires a value"
      case "$2" in ""|-*) die_usage "--profile requires a non-empty value" ;; esac
      [ -z "$PROFILE" ] || die_usage "duplicate --profile"
      PROFILE="$2"
      shift 2
      ;;
    --max-iterations=*)
      val="${arg#--max-iterations=}"
      [ -n "$val" ] || die_usage "--max-iterations requires a non-empty value"
      [ -z "$MAX_ITERATIONS" ] || die_usage "duplicate --max-iterations"
      MAX_ITERATIONS="$val"
      shift
      ;;
    --max-iterations)
      [ $# -ge 2 ] || die_usage "--max-iterations requires a value"
      case "$2" in ""|-*) die_usage "--max-iterations requires a non-empty value" ;; esac
      [ -z "$MAX_ITERATIONS" ] || die_usage "duplicate --max-iterations"
      MAX_ITERATIONS="$2"
      shift 2
      ;;
    --)
      shift
      [ $# -eq 0 ] || die_usage "unexpected positional argument: $1"
      ;;
    -*)
      die_usage "unknown option: $arg"
      ;;
    *)
      die_usage "unexpected positional argument: $arg"
      ;;
  esac
done

SEAT="${SEAT:-linux}"
case "$SEAT" in
  linux|codespace|laptop-service) ;;
  *) die_usage "unknown --seat '$SEAT' (expected: linux, codespace, laptop-service, muse)" ;;
esac

if [ -z "$KIND" ]; then
  case "$SEAT" in
    linux) KIND="generic" ;;
    codespace) KIND="generic" ;;
    laptop-service) KIND="laptop" ;;
  esac
fi

if [ -z "$NAME" ]; then
  if [ -n "${FLEET_AGENT:-}" ]; then
    NAME="$FLEET_AGENT"
  else
    case "$SEAT" in
      linux) NAME="$KIND" ;;
      codespace) NAME="codespace" ;;
      laptop-service) NAME="laptop" ;;
    esac
  fi
fi

if ! [[ "$NAME" =~ ^[a-z0-9][a-z0-9._-]{0,62}$ ]]; then
  die_usage "invalid --name '$NAME' (must match ^[a-z0-9][a-z0-9._-]{0,62}$)"
fi

if ! [[ "$KIND" =~ ^[a-z0-9][a-z0-9._-]{0,62}$ ]]; then
  die_usage "invalid --kind '$KIND'"
fi

BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
FLEET_URL="${FLEET_URL:-${FLEET_URL_ENV:-$DEFAULT_FLEET_URL}}"
FLEET_SHA256="${FLEET_SHA256:-${FLEET_SHA256_ENV:-$DEFAULT_FLEET_SHA256}}"
if ! [[ "$FLEET_SHA256" =~ ^[0-9a-fA-F]{64}$ ]]; then
  die_usage "invalid --fleet-sha256 '$FLEET_SHA256' (expected 64 hex chars)"
fi
FLEET_SHA256="${FLEET_SHA256,,}"

MAX_ITERATIONS="${MAX_ITERATIONS:-1}"
if ! [[ "$MAX_ITERATIONS" =~ ^[0-9]+$ ]]; then
  die_usage "invalid --max-iterations '$MAX_ITERATIONS' (expected non-negative integer)"
fi

# ---- 1. Install pinned fleet binary (exact filename `fleet`, sha256-verified) ----
mkdir -p "$BIN_DIR"
FLEET_BIN="$BIN_DIR/fleet"

verify_sha256() {
  local file="$1" expected="$2"
  echo "${expected}  ${file}" | sha256sum -c - >/dev/null 2>&1
}

if [ -f "$FLEET_BIN" ] && verify_sha256 "$FLEET_BIN" "$FLEET_SHA256"; then
  chmod +x "$FLEET_BIN"
  echo "[fleet-bootstrap] binary: already installed and sha256-verified at $FLEET_BIN"
else
  tmp_fleet="$(mktemp "$BIN_DIR/.fleet-download.XXXXXX")"
  trap 'rm -f "$tmp_fleet"' EXIT
  echo "[fleet-bootstrap] binary: downloading $FLEET_URL"
  curl -fsSL -o "$tmp_fleet" "$FLEET_URL" || die "download failed from $FLEET_URL"
  if ! verify_sha256 "$tmp_fleet" "$FLEET_SHA256"; then
    actual="$(sha256sum "$tmp_fleet" | awk '{print $1}')"
    die "sha256 mismatch for downloaded fleet binary (expected $FLEET_SHA256, got $actual)"
  fi
  chmod +x "$tmp_fleet"
  mv -f "$tmp_fleet" "$FLEET_BIN"
  trap - EXIT
  echo "[fleet-bootstrap] binary: verified sha256 and installed $FLEET_BIN"
fi

# ---- 2. Enroll ----
ENROLL_ARGS=(agent enroll --kind "$KIND" --name "$NAME")
[ -n "$AGENT_GROUPS" ] && ENROLL_ARGS+=(--groups "$AGENT_GROUPS")
[ -n "$PROFILE" ] && ENROLL_ARGS+=(--profile "$PROFILE")
[ "$DRY_RUN" = "1" ] && ENROLL_ARGS+=(--dry-run)

echo "[fleet-bootstrap] enroll: seat=$SEAT kind=$KIND name=$NAME"
set +e
"$FLEET_BIN" "${ENROLL_ARGS[@]}"
enroll_rc=$?
set -e

if [ "$DRY_RUN" = "1" ]; then
  echo "[fleet-bootstrap] dry-run complete (enroll exit $enroll_rc)"
  echo "[fleet-bootstrap] next loop:"
  echo "  $FLEET_BIN agent next --agent $NAME"
  echo "  # do the task it returns, then run its \`then\` lines"
  echo "  # repeat while the run has budget"
  exit "$enroll_rc"
fi

if [ "$enroll_rc" -ne 0 ] && [ "$enroll_rc" -ne 3 ]; then
  die "fleet agent enroll exited $enroll_rc"
fi

# ---- 3. Doctor ----
echo "[fleet-bootstrap] doctor: checking legs for $NAME"
set +e
"$FLEET_BIN" agent doctor --agent "$NAME"
doctor_rc=$?
set -e
if [ "$doctor_rc" -ne 0 ] && [ "$doctor_rc" -ne 3 ]; then
  die "fleet agent doctor exited $doctor_rc"
fi
if [ "$enroll_rc" -eq 3 ] && [ "$doctor_rc" -eq 3 ]; then
  die "no usable intake rungs reachable from this seat (enroll and doctor both returned 3)"
fi

# ---- 4. Three-line loop ----
echo "[fleet-bootstrap] loop:"
echo "  $FLEET_BIN agent next --agent $NAME    # exit 5 = no work on any rung"
echo "  # do the task it returns, then run its \`then\` lines"
echo "  # repeat while the run has budget"

if [ "$NO_NEXT" = "1" ] || [ "$MAX_ITERATIONS" -eq 0 ]; then
  exit 0
fi

iter=1
while [ "$iter" -le "$MAX_ITERATIONS" ]; do
  echo "[fleet-bootstrap] next ($iter/$MAX_ITERATIONS): $FLEET_BIN agent next --agent $NAME"
  set +e
  "$FLEET_BIN" agent next --agent "$NAME"
  next_rc=$?
  set -e
  if [ "$next_rc" -eq 5 ]; then
    echo "[fleet-bootstrap] next: no work on any rung (exit 5)"
    exit 0
  elif [ "$next_rc" -ne 0 ]; then
    die "fleet agent next exited $next_rc"
  fi
  iter=$((iter + 1))
done
