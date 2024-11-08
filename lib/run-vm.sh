#!/usr/bin/env bash

set -ex

sudo git add .

nixos-rebuild build-vm --flake ".#vm_${1:-amd-server}"

sudo ./result/bin/run-* -net tap # user
