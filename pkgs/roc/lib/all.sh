#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -exuo pipefail
./hello roc-lang_rust-basic-cli
./hello go-basic-cli
./display go-basic-cli
./hello rust-basic-cli
echo "Done"
