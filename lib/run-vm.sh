#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

sudo git add .

nixos-rebuild build-vm --flake ".#vm_${1:-amd-server}"

sudo ./result/bin/run-* -net tap # user
