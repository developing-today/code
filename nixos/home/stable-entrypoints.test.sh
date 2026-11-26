#!/usr/bin/env bash
# Tests for stable-entrypoints.sh and the stable-entrypoint
# contract (developing-today/code#14187).
#
# Oracle: the contract on code#14187 — the OpenChamber and mcpx
# systemd user units exec only the stable state-home paths
# (%h/.local/state/openchamber/start and
# %h/.local/state/mcpx/bin/mcpx, the latter with its daemon/config
# arguments); Home Manager activation creates valid initial targets
# and fails clearly when a target is missing or not executable; an
# unchanged target is left untouched (no unnecessary repoint, hence
# no unnecessary service bounce).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/stable-entrypoints.sh"
module="$here/stable-entrypoints.nix"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# Fake service builds: two generations of each target.
gen1="$tmp/gen1"
gen2="$tmp/gen2"
mkdir -p "$gen1" "$gen2"
for gen in "$gen1" "$gen2"; do
  printf '#!/bin/sh\nexit 0\n' >"$gen/openchamber-start"
  printf '#!/bin/sh\nexit 0\n' >"$gen/mcpx"
  chmod +x "$gen/openchamber-start" "$gen/mcpx"
done

state="$tmp/state"

# Case 1: first run creates both links, pointing at generation 1.
bash "$script" "$state" \
  openchamber/start "$gen1/openchamber-start" \
  mcpx/bin/mcpx "$gen1/mcpx"
[[ $(readlink "$state/openchamber/start") == "$gen1/openchamber-start" ]] ||
  fail "openchamber link does not point at generation 1"
[[ $(readlink "$state/mcpx/bin/mcpx") == "$gen1/mcpx" ]] ||
  fail "mcpx link does not point at generation 1"
[[ -x $state/openchamber/start && -x $state/mcpx/bin/mcpx ]] ||
  fail "created links are not executable"
echo 'ok - first run creates both stable links'

# Case 2: an unchanged target leaves the link untouched (same
# inode), the filesystem-level half of "no unnecessary bounce".
inode_before="$(stat -c %i "$state/openchamber/start")"
bash "$script" "$state" \
  openchamber/start "$gen1/openchamber-start" \
  mcpx/bin/mcpx "$gen1/mcpx"
[[ $(stat -c %i "$state/openchamber/start") == "$inode_before" ]] ||
  fail "unchanged target repointed the link"
echo 'ok - unchanged target leaves the link untouched'

# Case 3: a new generation repoints both links.
bash "$script" "$state" \
  openchamber/start "$gen2/openchamber-start" \
  mcpx/bin/mcpx "$gen2/mcpx"
[[ $(readlink "$state/openchamber/start") == "$gen2/openchamber-start" ]] ||
  fail "openchamber link was not repointed to generation 2"
[[ $(readlink "$state/mcpx/bin/mcpx") == "$gen2/mcpx" ]] ||
  fail "mcpx link was not repointed to generation 2"
echo 'ok - a new generation repoints both links'

# Case 4: a missing target fails clearly and changes nothing —
# validation happens before any link is touched, so the valid
# pair's link keeps pointing at generation 2.
err="$tmp/missing.err"
if bash "$script" "$state" \
  openchamber/start "$gen1/openchamber-start" \
  mcpx/bin/mcpx "$tmp/no-such-mcpx" 2>"$err"; then
  fail "missing target did not fail the run"
fi
grep -q "no-such-mcpx" "$err" || fail "missing-target error does not name the target"
[[ $(readlink "$state/openchamber/start") == "$gen2/openchamber-start" ]] ||
  fail "a failed run repointed a link"
echo 'ok - missing target fails clearly and changes nothing'

# Case 5: a non-executable target fails clearly.
plain="$tmp/plain-file"
printf 'not a program\n' >"$plain"
err="$tmp/nonexec.err"
if bash "$script" "$state" mcpx/bin/mcpx "$plain" 2>"$err"; then
  fail "non-executable target did not fail the run"
fi
grep -q "not executable" "$err" || fail "non-executable error is unclear"
echo 'ok - non-executable target fails clearly'

# Case 6: the module pins the exact stable paths in the units'
# executable fields, and no ExecStart/ExecReload in it names a
# store path.
grep -Fq 'ExecStart = lib.mkForce "%h/.local/state/openchamber/start"' "$module" ||
  fail "module does not pin the OpenChamber stable ExecStart"
grep -Fq 'ExecStart = lib.mkForce "%h/.local/state/mcpx/bin/mcpx daemon --config ' "$module" ||
  fail "module does not pin the mcpx stable ExecStart with its config argument"
grep -Fq 'ExecReload = lib.mkForce "%h/.local/state/mcpx/bin/mcpx reload"' "$module" ||
  fail "module does not pin the mcpx stable ExecReload"
if grep -E '(ExecStart|ExecReload) = .*/nix/store' "$module" >/dev/null; then
  fail "an Exec line in the module names a store path"
fi
grep -Fq 'entryBetween [ "mcpxDeploy" ] [ "reloadSystemd" ]' "$module" ||
  fail "activation entry is not ordered between reloadSystemd and mcpxDeploy"
echo 'ok - module pins the exact stable paths'

# Case 7 (full checkout only; the flake check copies nixos/home on
# its own): the host wires the module for the laptop user, and
# home/common documents the contract.
repo_root="$(cd "$here/../.." && pwd)"
if [[ -f $repo_root/nixos/hosts/default.nix ]]; then
  grep -Fq '../home/stable-entrypoints.nix' "$repo_root/nixos/hosts/default.nix" ||
    fail "host config does not import the stable-entrypoints module"
  grep -Fq 'services.stable-entrypoints.enable = true' "$repo_root/nixos/hosts/default.nix" ||
    fail "host config does not enable stable-entrypoints"
  grep -Fq 'stable-entrypoints' "$repo_root/home/common/default.nix" ||
    fail "home/common/default.nix does not document the contract"
  echo 'ok - host wiring and home/common documentation present'
fi

echo 'stable-entrypoints: all tests passed'
