#!/usr/bin/env bash
# Tests for the atomic sops writers (#6555): lib/sops_encrypt.sh and the
# init path of lib/bitwarden-to-sops.sh.
#
# Oracle: byte-identity. After a failed encryption, a pre-existing target
# must be byte-identical to before the run (or, for a first-run init, must
# not exist at all); after a successful encryption the target holds exactly
# the new ciphertext. The reference implementation for the expected shape
# is lib/bws-to-sops.sh (mktemp beside the target, mv only on success).
# sops is stubbed on PATH: SOPS_STUB_MODE=fail models any sops failure
# (bad key, sops error, interruption) — exit 1, no output; =ok models
# success by copying its input to stdout as the "ciphertext". bws is
# stubbed with a fixed dummy secret; no live credentials are used.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
ENC="$SCRIPT_DIR/sops_encrypt.sh"
BW="$SCRIPT_DIR/bitwarden-to-sops.sh"
[ -f "$ENC" ] || {
  echo "FAIL: $ENC not found" >&2
  exit 1
}
[ -f "$BW" ] || {
  echo "FAIL: $BW not found" >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || {
  echo "FAIL: jq not found in PATH" >&2
  exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
STUB_BIN="$TMP/bin"
mkdir -p "$STUB_BIN" "$TMP/home"

cat >"$STUB_BIN/sops" <<'STUB'
#!/usr/bin/env bash
# Stub sops: fail mode exits 1 with no output; ok mode "encrypts" by
# copying its input (the last argument, a process-substitution fd) to
# stdout.
if [ "${SOPS_STUB_MODE:-ok}" = "fail" ]; then
  echo "stub sops: simulated encryption failure" >&2
  exit 1
fi
last="${!#}"
cat "$last"
STUB

cat >"$STUB_BIN/bws" <<'STUB'
#!/usr/bin/env bash
# Stub bws: only `secret get <query>` is supported.
if [ "${1:-}" = "secret" ] && [ "${2:-}" = "get" ]; then
  printf '{"key":"MY_SECRET","value":"dummy-value"}\n'
  exit 0
fi
echo "stub bws: unsupported invocation: $*" >&2
exit 2
STUB
chmod +x "$STUB_BIN/sops" "$STUB_BIN/bws"

FAILURES=0
pass() { echo "PASS: $1"; }
fail() {
  echo "FAIL: $1" >&2
  FAILURES=$((FAILURES + 1))
}

# check <name> <condition...>: pass when the condition command succeeds.
check() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name"; fi
}

leftover_temps() { # <dir> <prefix> -> count of leftover temp files
  find "$1" -maxdepth 1 -name "$2.*" | wc -l
}

# ---------------------------------------------------------------- sops_encrypt, script form
enc_case() { # <name> <mode>
  local name="$1" mode="$2" dir="$TMP/enc-$1"
  mkdir -p "$dir"
  local target="$dir/out.enc" sentinel="$dir/sentinel"
  printf 'PREVIOUS-CIPHERTEXT' >"$sentinel"
  cp "$sentinel" "$target"
  local out rc
  out="$(printf 'new-plaintext' | SOPS_STUB_MODE="$mode" PATH="$STUB_BIN:$PATH" bash "$ENC" "$target" 2>&1)"
  rc=$?
  if [ "$mode" = "fail" ]; then
    [ "$rc" -ne 0 ] && pass "sops_encrypt script/$name: non-zero exit" || fail "sops_encrypt script/$name: exit was $rc, want non-zero"
    cmp -s "$sentinel" "$target" && pass "sops_encrypt script/$name: target byte-identical" || fail "sops_encrypt script/$name: target changed on failure"
    case "$out" in *"Encrypted output saved"*) fail "sops_encrypt script/$name: success message printed on failure" ;; *) pass "sops_encrypt script/$name: no success message on failure" ;; esac
  else
    [ "$rc" -eq 0 ] && pass "sops_encrypt script/$name: zero exit" || fail "sops_encrypt script/$name: exit was $rc, want 0"
    printf 'new-plaintext' | cmp -s - "$target" && pass "sops_encrypt script/$name: target replaced with new ciphertext" || fail "sops_encrypt script/$name: target does not hold the new ciphertext"
    case "$out" in *"Encrypted output saved"*) pass "sops_encrypt script/$name: success message on success" ;; *) fail "sops_encrypt script/$name: success message missing on success" ;; esac
  fi
  [ "$(leftover_temps "$dir" '.sops_encrypt')" -eq 0 ] && pass "sops_encrypt script/$name: no leftover temp files" || fail "sops_encrypt script/$name: leftover temp files"
}

# ---------------------------------------------------------------- sops_encrypt, sourced form
enc_sourced_case() { # <name> <mode>
  local name="$1" mode="$2" dir="$TMP/encs-$1"
  mkdir -p "$dir"
  local target="$dir/out.enc" sentinel="$dir/sentinel"
  printf 'PREVIOUS-CIPHERTEXT' >"$sentinel"
  cp "$sentinel" "$target"
  local out rc
  out="$(SOPS_STUB_MODE="$mode" PATH="$STUB_BIN:$PATH" bash -c 'source "$1"; printf "new-plaintext" | sops_encrypt "$2"' _ "$ENC" "$target" 2>&1)"
  rc=$?
  if [ "$mode" = "fail" ]; then
    [ "$rc" -ne 0 ] && pass "sops_encrypt sourced/$name: non-zero return" || fail "sops_encrypt sourced/$name: return was $rc, want non-zero"
    cmp -s "$sentinel" "$target" && pass "sops_encrypt sourced/$name: target byte-identical" || fail "sops_encrypt sourced/$name: target changed on failure"
    case "$out" in *"Encrypted output saved"*) fail "sops_encrypt sourced/$name: success message printed on failure" ;; *) pass "sops_encrypt sourced/$name: no success message on failure" ;; esac
  else
    [ "$rc" -eq 0 ] && pass "sops_encrypt sourced/$name: zero return" || fail "sops_encrypt sourced/$name: return was $rc, want 0"
    printf 'new-plaintext' | cmp -s - "$target" && pass "sops_encrypt sourced/$name: target replaced with new ciphertext" || fail "sops_encrypt sourced/$name: target does not hold the new ciphertext"
  fi
  [ "$(leftover_temps "$dir" '.sops_encrypt')" -eq 0 ] && pass "sops_encrypt sourced/$name: no leftover temp files" || fail "sops_encrypt sourced/$name: leftover temp files"
}

# ---------------------------------------------------------------- bitwarden-to-sops init path
bw_init_case() { # <name> <mode>
  local name="$1" mode="$2" dir="$TMP/bw-$1"
  mkdir -p "$dir"
  local target="$dir/new-target.yaml"
  local out rc
  out="$(HOME="$TMP/home" BWS_ACCESS_TOKEN=dummy SOPS_STUB_MODE="$mode" PATH="$STUB_BIN:$PATH" bash "$BW" --bws-secret test-secret --target "$target" --no-auto-env 2>&1)"
  rc=$?
  if [ "$mode" = "fail" ]; then
    [ "$rc" -ne 0 ] && pass "bitwarden-to-sops init/$name: non-zero exit" || fail "bitwarden-to-sops init/$name: exit was $rc, want non-zero"
    [ ! -e "$target" ] && pass "bitwarden-to-sops init/$name: no file at target after failed init" || fail "bitwarden-to-sops init/$name: target exists after failed init (later runs would take the update branch)"
    case "$out" in *"Created and encrypted"*) fail "bitwarden-to-sops init/$name: success message printed on failure" ;; *) pass "bitwarden-to-sops init/$name: no success message on failure" ;; esac
  else
    [ "$rc" -eq 0 ] && pass "bitwarden-to-sops init/$name: zero exit" || fail "bitwarden-to-sops init/$name: exit was $rc, want 0: $out"
    [ -s "$target" ] && pass "bitwarden-to-sops init/$name: target created non-empty" || fail "bitwarden-to-sops init/$name: target missing or empty after successful init"
    grep -q 'my_secret: "dummy-value"' "$target" 2>/dev/null && pass "bitwarden-to-sops init/$name: target holds the encrypted payload" || fail "bitwarden-to-sops init/$name: target does not hold the expected payload"
  fi
  [ "$(leftover_temps "$dir" '.bitwarden-to-sops')" -eq 0 ] && pass "bitwarden-to-sops init/$name: no leftover temp files" || fail "bitwarden-to-sops init/$name: leftover temp files"
}

enc_case fail fail
enc_case ok ok
enc_sourced_case fail fail
enc_sourced_case ok ok
bw_init_case fail fail
bw_init_case ok ok

if [ "$FAILURES" -ne 0 ]; then
  echo "sops-atomic-writers: $FAILURES check(s) failed" >&2
  exit 1
fi
echo "sops-atomic-writers: all checks passed"
