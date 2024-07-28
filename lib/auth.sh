#!/usr/bin/env bash
set -exuo pipefail
ulimit -n $(ulimit -Hn)
sudo prlimit --pid $$ --nofile=1000000:1000000
# shellcheck disable=SC2312
set +x
sudo NIX_CONFIG="access-tokens = github.com=$(cat ~/auth)" ./rebuild.sh
set -x
