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
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
ulimit -n "$(ulimit -Hn)"
sudo prlimit --pid $$ --nofile=1000000:1000000
# shellcheck disable=SC2312
set +x
sudo NIX_CONFIG="access-tokens = github.com=$(sudo cat "$AUTH_FILE")" "$@"
set -Eeuxo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
