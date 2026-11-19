#!/usr/bin/env bash
DEFAULT_BUILD_SCRIPT="./lib/rebuild.sh"
AUTH_FILE="$HOME/auth"
usage() {
  echo "Usage: $0 [--auth auth_file] command [args...]"
  echo "  --auth auth_file : Path to authentication file (default: ~/auth)"
  echo "  command         : Command to execute"
  exit 1
}
if [ $# -eq 0 ]; then
  usage
fi
while [ $# -gt 0 ]; do
  case "$1" in
  --auth)
    shift
    if [ $# -eq 0 ]; then
      echo "Error: --auth requires a file argument"
      exit 1
    fi
    AUTH_FILE="$1"
    shift
    ;;
  *)
    break
    ;;
  esac
done
if [ $# -eq 0 ]; then
  usage
fi
if [ ! -f "$AUTH_FILE" ]; then
  echo "Error: Authentication file $AUTH_FILE not found"
  exit 1
fi
# Log the run (see lib/rebuild-log.sh): the log file is created here, as the
# user, before sudo; the root side only appends to it. Logging problems never
# stop the command.
# shellcheck source=rebuild-log.sh
source "$(dirname "${BASH_SOURCE[0]}")/rebuild-log.sh"
rebuild_log_init "$@" || true
# The braces keep the trap's own bookkeeping out of the set -x trace.
trap '{ rc=$?; set +x; } 2>/dev/null; rebuild_log_finish "$rc"; exit "$rc"' EXIT
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
ulimit -n "$(ulimit -Hn)"
sudo prlimit --pid $$ --nofile=1000000:1000000
# shellcheck disable=SC2312
set +x
sudo NIX_CONFIG="access-tokens = github.com=$(sudo cat "$AUTH_FILE")" REBUILD_LOG="${REBUILD_LOG:-}" "$@"
set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
