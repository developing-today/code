#!/usr/bin/env bash
# lane-scratch: n/a — repo file (pkgs/mcpx/scripts/test.sh), queue task 13250
# test.sh — the mcpx test entrypoint (#1949, Approach C): one run
# seed per run, for every go test process this wrapper starts.
#
#   pkgs/mcpx/scripts/test.sh [go test args...]
#
# With no args it runs the whole suite (go test ./...). Any args are
# passed to go test verbatim, after the derived -shuffle flag:
#
#   pkgs/mcpx/scripts/test.sh -run '^TestX$' ./internal/codegen
#
# The seed (MCPX_TEST_SEED, #1949 section 4) is minted once here —
# never inside a test process — exported to every go test process
# (and, via os.Environ in the e2e harness, to every subprocess), and
# printed so a failure replays with one command:
#
#   MCPX_TEST_SEED=<id> pkgs/mcpx/scripts/test.sh -run '^TestX$' ./internal/codegen
#
# Behaviour (#1949 section 5):
#   - MCPX_TEST_SEED set: validated ([A-Za-z0-9._-]{1,128}) and used
#     as-is; an invalid value is an error, never a silent re-mint.
#   - Unset locally: warn, mint a UUIDv7, print it, and record it in
#     the gitignored pkgs/mcpx/.cache/test-seed/ (last + history).
#   - Unset in CI (CI=true or GITHUB_ACTIONS=true): exit 1. CI mints
#     in the workflow's seed job, once per run, and passes the id
#     down — a test process must never mint its own.
#
# -shuffle is derived from the same seed (#1949 section 2), so the
# seed alone reproduces both the generated inputs and the test
# order. Caveat (#1949 section 6): this reproduces inputs and
# order, not timing. Bare `go test` without the wrapper still works
# — the seed helper then mints per package process and warns; the
# wrapper is what makes the seed per-run instead of per-package.
set -euo pipefail

cd "$(dirname "$0")/.."

mint_v7() {
  python3 -c "import os,time;b=bytearray(int(time.time()*1000).to_bytes(6,'big')+os.urandom(10));b[6]=b[6]&15|0x70;b[8]=b[8]&63|0x80;h=b.hex();print(f'{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}')"
}

if [ -z "${MCPX_TEST_SEED:-}" ]; then
  if [ "${CI:-}" = "true" ] || [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    echo "test.sh: MCPX_TEST_SEED is unset in CI; CI must never mint inside a test run — the workflow seed job provides it" >&2
    exit 1
  fi
  MCPX_TEST_SEED="$(mint_v7)"
  echo "test.sh: warning: no MCPX_TEST_SEED; minted $MCPX_TEST_SEED for this run" >&2
  cache_dir=".cache/test-seed"
  mkdir -p "$cache_dir"
  printf '%s\n' "$MCPX_TEST_SEED" >"$cache_dir/last"
  printf '%s\n' "$MCPX_TEST_SEED" >>"$cache_dir/history"
fi

if ! printf '%s' "$MCPX_TEST_SEED" | grep -Eq '^[A-Za-z0-9._-]{1,128}$'; then
  echo "test.sh: MCPX_TEST_SEED=$MCPX_TEST_SEED is not a valid seed id ([A-Za-z0-9._-]{1,128})" >&2
  exit 1
fi
export MCPX_TEST_SEED

# Shuffle derivation, #1949 section 2 — the same formula the Go
# helper (internal/testsupport/seed.Shuffle) and the CI stress jobs
# implement; the helper's tests pin the formula from the spec.
shuffle="$(python3 -c "
import hashlib, os
root = hashlib.sha256(b'mcpx-test-seed/v1\x00' + os.environ['MCPX_TEST_SEED'].encode()).digest()
print(int.from_bytes(hashlib.sha256(root + b'shuffle').digest()[:8], 'big') & (2**63 - 1))
")"

echo "MCPX_TEST_SEED=$MCPX_TEST_SEED (shuffle=$shuffle; reproduce: MCPX_TEST_SEED=$MCPX_TEST_SEED pkgs/mcpx/scripts/test.sh <args>)"

if [ "$#" -eq 0 ]; then
  set -- ./...
fi
exec go test -shuffle="$shuffle" "$@"
