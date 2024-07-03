#!/usr/bin/env bash

nix-shell -p bubblewrap nix-output-monitor nixpkgs-review --run "GITHUB_TOKEN=$(cat /home/user/auth) nixpkgs-review pr --print-result --post-result --sandbox $1"
