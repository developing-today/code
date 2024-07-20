#!/usr/bin/env bash
set -euo pipefail
ulimit -n $(ulimit -Hn)
sudo prlimit --pid $$ --nofile=1000000:1000000
# shellcheck disable=SC2312
sudo NIX_CONFIG="access-tokens = github.com=$(cat /home/user/auth)" ./rebuild.sh
