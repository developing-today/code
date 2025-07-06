#!/usr/bin/env bash
# shellcheck disable=SC2086,SC2154
set -exuo pipefail
./hello cli
./hello go
./display go
