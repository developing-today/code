#!/usr/bin/env bash
nixos-rebuild --use-remote-sudo --offline --no-net --accept-flake-config --json switch --json --upgrade --json --print-build-logs --verbose --keep-going --log-format internal-json --fallback --show-trace --flake '.' |& nom --json
