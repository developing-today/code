#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
. ./build hello "${@}"
