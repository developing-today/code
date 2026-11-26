#!/usr/bin/env bash
# Temporary verification script for the #803 final slice. NOT part of the PR.
# Runs the warning-mode census, then the per-crate clippy -D gates.
set -u
here="$(dirname "$0")"
bash "$here/census.sh"
bash "$here/gate.sh"
echo "VERIFY1 DONE"
