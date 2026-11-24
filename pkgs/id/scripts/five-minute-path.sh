#!/usr/bin/env bash
# five-minute-path.sh — Phase 12 exit criterion 1 (#12595, plan step A9).
#
# Runs the documented steps from an empty directory to a world a second
# node joins, and prints the elapsed time. The clock starts before the
# clone and stops after the join is verified, so the reported time is the
# whole empty-directory-to-joinable-world path, not just the join.
#
# The steps (also in pkgs/id/README.md, "Five-minute path"), timed in
# six groups — step 3 covers the three serve-side commands (start the
# host, read its node id, wait until it answers):
#   1. git clone --depth 1 <repo> <dir>/repo
#   2. cargo build --release --no-default-features -p id   (in <dir>/repo/pkgs/id)
#   3. id serve --world --world-name <world> --data-dir <dir>/host
#        --iroh-port <port> --no-relay --no-gossip --no-mdns   (background)
#      id --data-dir <dir>/host id
#      id world list <node> --addr 127.0.0.1:<port> --no-relay   (readiness poll)
#   4. id world invite <node> --name friend --world <world>
#        --addr 127.0.0.1:<port> --no-relay
#   5. id world join <node> --capability <cap> --world <world>
#        --addr 127.0.0.1:<port> --no-relay --command-mode --json
#        (one server frame observed, then /quit)
#   6. id world records <node> --capability <cap> --world <world>
#        --addr 127.0.0.1:<port> --no-relay
#
# The admin token is a fresh random value passed only through the
# ID_WORLD_ADMIN_TOKEN environment variable, never on a command line.
# The guest capability likewise travels via ID_WORLD_CAPABILITY.
# All traffic is direct loopback (--addr 127.0.0.1:<port> --no-relay),
# so no relay, DHT, gossip, or mDNS is involved.
#
# Exit status: 0 when the path completes under the budget (PASS),
# 1 when a step fails or the elapsed time exceeds the budget (FAIL),
# 2 on a usage error.
#
# Argument contract (#3352): the whole command line is declared below
# and parsed strictly before any subprocess, network, file write, or
# server start. Anything unaccounted for is a usage error naming the
# offending argument. --opt=value and --opt value are identical; a
# present-but-valueless option is a usage error, never the default.
# --dry-run is honoured on every path: it prints the steps and exits
# before any tool runs.
#
# Usage: five-minute-path.sh [--dir DIR] [--repo URL] [--ref REF]
#        [--world NAME] [--budget SECONDS] [--iroh-port PORT]
#        [--step-timeout SECONDS] [--join-timeout SECONDS]
#        [--id-bin PATH] [--clean] [--dry-run] [--help]
#
#   --dir DIR             Run directory; must be absent or empty.
#                         Default: a fresh mktemp -d.
#   --repo URL            Repo to clone. Default:
#                         https://github.com/developing-today/code.git
#   --ref REF             Branch or tag to clone (git --branch).
#                         Default: the remote's default branch.
#   --world NAME          World name. Default: five-minute.
#   --budget SECONDS      PASS when elapsed is under this. Default: 300.
#   --iroh-port PORT      Fixed Iroh port for the host. Default: 17001.
#   --step-timeout SECS   Cap for the serve-readiness poll. Default: 120.
#   --join-timeout SECS   Cap for waiting on the first join frame.
#                         Default: 120.
#   --id-bin PATH         Use this id binary; skip the clone and build
#                         steps (reported as skipped in the RESULT line).
#   --clean               Remove the run directory on success.
#   --dry-run             Print the steps and exit 0; run nothing.
#   --help                Print this usage and exit 0.
#
set -euo pipefail

PROG="five-minute-path.sh"
DEFAULT_REPO="https://github.com/developing-today/code.git"
DEFAULT_WORLD="five-minute"
DEFAULT_BUDGET=300
DEFAULT_IROH_PORT=17001
DEFAULT_STEP_TIMEOUT=120
DEFAULT_JOIN_TIMEOUT=120

run_dir=""
repo="$DEFAULT_REPO"
ref=""
world="$DEFAULT_WORLD"
budget="$DEFAULT_BUDGET"
iroh_port="$DEFAULT_IROH_PORT"
step_timeout="$DEFAULT_STEP_TIMEOUT"
join_timeout="$DEFAULT_JOIN_TIMEOUT"
id_bin=""
clean=0
dry_run=0

usage() {
  sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d; s/^# \?//'
}

fail_usage() {
  printf '%s: usage error: %s\n' "$PROG" "$1" >&2
  printf 'usage: %s [--dir DIR] [--repo URL] [--ref REF] [--world NAME] [--budget SECONDS] [--iroh-port PORT] [--step-timeout SECONDS] [--join-timeout SECONDS] [--id-bin PATH] [--clean] [--dry-run] [--help]\n' "$PROG" >&2
  exit 2
}

is_positive_int() {
  case "$1" in
    '' | *[!0-9]*) return 1 ;;
    *) [ "$1" -gt 0 ] ;;
  esac
}

# Strict parse: every option is declared; --opt=value and --opt value
# are identical; a missing value is a usage error.
while [ $# -gt 0 ]; do
  case "$1" in
    --dir | --repo | --ref | --world | --budget | --iroh-port | --step-timeout | --join-timeout | --id-bin)
      opt="$1"
      if [ $# -lt 2 ] || [ -z "$2" ]; then
        fail_usage "$opt requires a non-empty value"
      fi
      val="$2"
      shift 2
      case "$opt" in
        --dir) run_dir="$val" ;;
        --repo) repo="$val" ;;
        --ref) ref="$val" ;;
        --world) world="$val" ;;
        --budget) budget="$val" ;;
        --iroh-port) iroh_port="$val" ;;
        --step-timeout) step_timeout="$val" ;;
        --join-timeout) join_timeout="$val" ;;
        --id-bin) id_bin="$val" ;;
      esac
      ;;
    --dir=* | --repo=* | --ref=* | --world=* | --budget=* | --iroh-port=* | --step-timeout=* | --join-timeout=* | --id-bin=*)
      opt="${1%%=*}"
      val="${1#*=}"
      if [ -z "$val" ]; then
        fail_usage "$opt requires a non-empty value"
      fi
      shift
      case "$opt" in
        --dir) run_dir="$val" ;;
        --repo) repo="$val" ;;
        --ref) ref="$val" ;;
        --world) world="$val" ;;
        --budget) budget="$val" ;;
        --iroh-port) iroh_port="$val" ;;
        --step-timeout) step_timeout="$val" ;;
        --join-timeout) join_timeout="$val" ;;
        --id-bin) id_bin="$val" ;;
      esac
      ;;
    --clean) clean=1; shift ;;
    --dry-run) dry_run=1; shift ;;
    --help) usage; exit 0 ;;
    --*) fail_usage "unaccounted argument: $1" ;;
    *) fail_usage "unaccounted argument: $1" ;;
  esac
done

is_positive_int "$budget" || fail_usage "--budget must be a positive integer, got: $budget"
is_positive_int "$iroh_port" || fail_usage "--iroh-port must be a positive integer, got: $iroh_port"
[ "$iroh_port" -le 65535 ] || fail_usage "--iroh-port must be 1-65535, got: $iroh_port"
is_positive_int "$step_timeout" || fail_usage "--step-timeout must be a positive integer, got: $step_timeout"
is_positive_int "$join_timeout" || fail_usage "--join-timeout must be a positive integer, got: $join_timeout"
case "$world" in
  '' | *[!a-z0-9_-]*) fail_usage "--world must match [a-z0-9_-]+, got: $world" ;;
esac
if [ -n "$id_bin" ] && [ ! -x "$id_bin" ]; then
  fail_usage "--id-bin is not executable: $id_bin"
fi

# Per-step elapsed seconds, as "name=secs" pairs in order.
step_times=""
step_start=0

step_begin() {
  step_start="$(date +%s)"
}

step_end() {
  local name="$1"
  local now_s
  now_s="$(date +%s)"
  step_times="$step_times $name=$((now_s - step_start))"
}

serve_pid=""

cleanup() {
  if [ -n "$serve_pid" ] && kill -0 "$serve_pid" 2>/dev/null; then
    kill "$serve_pid" 2>/dev/null || true
    wait "$serve_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

die() {
  printf '%s: FAIL: %s\n' "$PROG" "$1" >&2
  printf '%s: run directory kept at %s (serve log: %s)\n' "$PROG" "$run_dir" "$serve_log" >&2
  exit 1
}

# --dry-run prints the exact steps and exits before any tool runs
# and before the run directory is created.
if [ "$dry_run" -eq 1 ]; then
  id_path="${id_bin:-<dir>/repo/pkgs/id/target/release/id}"
  display_dir="${run_dir:-<fresh temporary directory>}"
  printf 'five-minute-path dry run (run dir: %s)\n' "$display_dir"
  if [ -z "$id_bin" ]; then
    if [ -n "$ref" ]; then
      printf '  1. git clone --depth 1 --branch %s %s %s/repo\n' "$ref" "$repo" "$display_dir"
    else
      printf '  1. git clone --depth 1 %s %s/repo\n' "$repo" "$display_dir"
    fi
    printf '  2. cargo build --release --no-default-features -p id --manifest-path %s/repo/pkgs/id/Cargo.toml\n' "$display_dir"
  else
    printf '  1. clone: skipped (--id-bin)\n'
    printf '  2. build: skipped (--id-bin)\n'
  fi
  printf '  3. ID_WORLD_ADMIN_TOKEN=<generated> %s --data-dir %s/host serve --world --world-name %s --iroh-port %s --no-relay --no-gossip --no-mdns\n' "$id_path" "$display_dir" "$world" "$iroh_port"
  printf '     %s --data-dir %s/host id\n' "$id_path" "$display_dir"
  printf '     %s world list <node> --addr 127.0.0.1:%s --no-relay (readiness poll)\n' "$id_path" "$iroh_port"
  printf '  4. %s world invite <node> --name friend --world %s --addr 127.0.0.1:%s --no-relay\n' "$id_path" "$world" "$iroh_port"
  printf '  5. %s world join <node> --capability <cap> --world %s --addr 127.0.0.1:%s --no-relay --command-mode --json\n' "$id_path" "$world" "$iroh_port"
  printf '  6. %s world records <node> --capability <cap> --world %s --addr 127.0.0.1:%s --no-relay\n' "$id_path" "$world" "$iroh_port"
  printf 'budget: %ss; world: %s; iroh port: %s\n' "$budget" "$world" "$iroh_port"
  exit 0
fi

# Run directory: absent (created) or empty; anything else is a usage
# error. This runs after the --dry-run branch, so a dry run creates
# no directory.
if [ -z "$run_dir" ]; then
  run_dir="$(mktemp -d)"
elif [ -e "$run_dir" ]; then
  if [ -n "$(find "$run_dir" -mindepth 1 -print -quit 2>/dev/null)" ]; then
    fail_usage "--dir is not empty: $run_dir"
  fi
else
  mkdir -p "$run_dir"
fi

repo_dir="$run_dir/repo"
host_dir="$run_dir/host"
serve_log="$run_dir/serve.log"
join_out="$run_dir/join.out"
join_err="$run_dir/join.err"
records_out="$run_dir/records.out"

for tool in date mktemp od; do
  command -v "$tool" >/dev/null 2>&1 || die "required tool not on PATH: $tool"
done
if [ -z "$id_bin" ]; then
  # git and cargo are needed only for the clone and build steps.
  for tool in git cargo; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not on PATH: $tool"
  done
fi

clock_start="$(date +%s)"

# A fresh random admin token, passed only via the environment.
if command -v openssl >/dev/null 2>&1; then
  admin_token="$(openssl rand -hex 16)"
else
  admin_token="$(od -An -tx1 -N 16 /dev/urandom | tr -d ' \n')"
fi
[ -n "$admin_token" ] || die "could not generate an admin token"
export ID_WORLD_ADMIN_TOKEN="$admin_token"

if [ -n "$id_bin" ]; then
  id_path="$id_bin"
  step_times=" clone=skipped build=skipped"
else
  id_path="$repo_dir/pkgs/id/target/release/id"
  step_begin
  printf 'step 1/6: clone %s\n' "$repo"
  clone_args=(clone --depth 1)
  if [ -n "$ref" ]; then
    clone_args+=(--branch "$ref")
  fi
  clone_args+=("$repo" "$repo_dir")
  git "${clone_args[@]}" || die "git clone failed"
  step_end clone

  step_begin
  printf 'step 2/6: build id (release, library-only)\n'
  cargo build --release --no-default-features -p id \
    --manifest-path "$repo_dir/pkgs/id/Cargo.toml" \
    || die "cargo build failed"
  [ -x "$id_path" ] || die "build finished but no binary at $id_path"
  step_end build
fi

step_begin
printf 'step 3/6: serve world %s on 127.0.0.1:%s\n' "$world" "$iroh_port"
"$id_path" --data-dir "$host_dir" serve --world --world-name "$world" \
  --iroh-port "$iroh_port" --no-relay --no-gossip --no-mdns \
  >"$serve_log" 2>&1 &
serve_pid=$!

# The host's node id: the same key file serve uses, so this also
# proves the host's data directory is up.
node=""
deadline=$(( $(date +%s) + step_timeout ))
while [ -z "$node" ]; do
  if ! kill -0 "$serve_pid" 2>/dev/null; then
    die "serve exited during startup; see $serve_log"
  fi
  node="$("$id_path" --data-dir "$host_dir" id 2>/dev/null || true)"
  if [[ "$node" =~ ^[0-9a-f]{64}$ ]]; then
    break
  fi
  node=""
  if [ "$(date +%s)" -ge "$deadline" ]; then
    die "serve did not produce a node id within ${step_timeout}s; see $serve_log"
  fi
  sleep 2
done

# Readiness: the public world list must answer over the same direct
# loopback path the invite, join, and records steps will use.
deadline=$(( $(date +%s) + step_timeout ))
while true; do
  if "$id_path" world list "$node" --addr "127.0.0.1:$iroh_port" --no-relay \
      >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$serve_pid" 2>/dev/null; then
    die "serve exited while waiting for readiness; see $serve_log"
  fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    die "serve was not reachable via world list within ${step_timeout}s; see $serve_log"
  fi
  sleep 2
done
step_end serve

step_begin
printf 'step 4/6: invite a friend to %s\n' "$world"
if ! capability="$("$id_path" world invite "$node" --name friend --world "$world" \
  --addr "127.0.0.1:$iroh_port" --no-relay 2>"$run_dir/invite.err")"; then
  die "world invite failed; see $run_dir/invite.err"
fi
[ -n "$capability" ] || die "world invite printed no capability"
step_end invite

step_begin
printf 'step 5/6: second node joins %s\n' "$world"
export ID_WORLD_CAPABILITY="$capability"
# The join runs on a coproc so the script can watch for the first
# server frame — proof the join handshake completed — before sending
# /quit. Feeding /quit up front would race the join response. The
# coproc's stdout is the pipe (read here); only stderr goes to a file.
coproc JOIN {
  "$id_path" world join "$node" --world "$world" \
    --addr "127.0.0.1:$iroh_port" --no-relay --command-mode --json \
    2>"$join_err"
}
# shellcheck disable=SC2154 # JOIN_PID is set by the coproc above
join_pid="$JOIN_PID"
# Capture the fds now: bash unsets the JOIN array once the coproc
# exits, so the first-frame read and /quit below must use these.
# shellcheck disable=SC2154 # JOIN is set by the coproc above
join_out_fd="${JOIN[0]}"
# shellcheck disable=SC2154 # JOIN is set by the coproc above
join_in_fd="${JOIN[1]}"
first_frame=""
if IFS= read -r -t "$join_timeout" first_frame <&"$join_out_fd"; then
  printf '%s\n' "$first_frame" >"$join_out"
else
  kill "$join_pid" 2>/dev/null || true
  wait "$join_pid" 2>/dev/null || true
  die "join produced no server frame within ${join_timeout}s; see $join_err"
fi
printf '/quit\n' >&"$join_in_fd" || true
# If that write failed the join is already gone; the bounded wait
# below sees it dead and `wait` reports its real exit status.
# The write end stays open until the script exits; the pump ends on
# /quit, so no explicit close is needed.
# The pump ends on /quit; give it a bounded wait, then fail loudly.
wait_deadline=$(( $(date +%s) + 30 ))
while kill -0 "$join_pid" 2>/dev/null; do
  if [ "$(date +%s)" -ge "$wait_deadline" ]; then
    kill "$join_pid" 2>/dev/null || true
    wait "$join_pid" 2>/dev/null || true
    die "join did not exit after /quit; see $join_err"
  fi
  sleep 1
done
# No drain here: bash closes the coproc's fds the moment the join
# process exits, so trailing frames cannot be collected after it
# ends. The first frame above is the join proof; join.out holds it.
wait "$join_pid" || die "join exited non-zero; see $join_err"
[ -s "$join_out" ] || die "join exited 0 but printed no server frame"
step_end join

step_begin
printf 'step 6/6: read the world records with the guest capability\n'
"$id_path" world records "$node" --world "$world" \
  --addr "127.0.0.1:$iroh_port" --no-relay \
  >"$records_out" 2>"$run_dir/records.err" \
  || die "world records failed; see $run_dir/records.err"
[ -s "$records_out" ] || die "world records printed nothing"
step_end records

cleanup
trap - EXIT
serve_pid=""

clock_end="$(date +%s)"
elapsed=$((clock_end - clock_start))

printf '\nstep times (seconds):%s\n' "$step_times"
printf 'total elapsed: %ss (budget: %ss)\n' "$elapsed" "$budget"
if [ "$elapsed" -lt "$budget" ]; then
  verdict="PASS"
  status=0
else
  verdict="FAIL"
  status=1
fi
printf 'RESULT verdict=%s elapsed_s=%s budget_s=%s world=%s steps=%s run_dir=%s\n' \
  "$verdict" "$elapsed" "$budget" "$world" "${step_times# }" "$run_dir"

if [ "$status" -eq 0 ] && [ "$clean" -eq 1 ]; then
  rm -rf "$run_dir"
  printf 'run directory removed (--clean)\n'
else
  printf 'run directory kept at %s\n' "$run_dir"
fi
exit "$status"
