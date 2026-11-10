#!/usr/bin/env bash
# Test for infra/dns/save.sh atomic encryption (#6536).
#
# Oracle: the previous terraform.tfstate.enc is the state of record.
#   1. A failing `sops -e` must leave it byte-identical, exit non-zero,
#      and leave neither a temp file nor the .lock behind.
#   2. A succeeding `sops -e` must install the new ciphertext.
#   3. When the .enc already decrypts to the plaintext, save.sh must
#      skip re-encryption (fast path) and leave the .enc byte-identical.
#
# The stub `sops` below is an identity cipher (decrypt(encrypt(x)) == x),
# which is exactly the property save.sh's own cmp checks rely on; the
# failing variant emits partial output before exiting 1, reproducing the
# truncation hazard the fix removes.
set -uo pipefail

here="$(dirname -- "$(realpath -- "$0")")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/dns"
cp "$here/save.sh" "$work/dns/save.sh"
chmod +x "$work/dns/save.sh"

cat >"$work/bin/sops" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
mode="$1"; file="$2"
case "$mode" in
  -d) cat "$file" ;;
  -e)
    if [ -n "${STUB_SOPS_FAIL_E:-}" ]; then
      printf 'PARTIAL-CIPHERTEXT'
      exit 1
    fi
    cat "$file"
    ;;
  *) echo "unexpected sops args: $*" >&2; exit 2 ;;
esac
STUB
chmod +x "$work/bin/sops"
export PATH="$work/bin:$PATH"

fail=0
check() { # name condition...
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "PASS: $name"
  else
    echo "FAIL: $name" >&2
    fail=1
  fi
}

printf '{"lineage":"L1","serial":1}\n' >"$work/dns/terraform.tfstate"
printf '{"lineage":"L1","serial":1}\n' >"$work/dns/terraform.tfstate.enc"
enc_before="$(sha256sum "$work/dns/terraform.tfstate.enc" | cut -d' ' -f1)"

# Scenario 3 (run first, while enc still matches plaintext): fast path.
"$work/dns/save.sh" >"$work/fast.log" 2>&1
check "fast path exits 0" test $? -eq 0
check "fast path leaves .enc byte-identical" test "$(sha256sum "$work/dns/terraform.tfstate.enc" | cut -d' ' -f1)" = "$enc_before"

# Scenario 1: state changed, sops -e fails after partial output.
printf '{"lineage":"L1","serial":2}\n' >"$work/dns/terraform.tfstate"
STUB_SOPS_FAIL_E=1 "$work/dns/save.sh" >"$work/fail.log" 2>&1
check "failing sops -e exits non-zero" test $? -ne 0
check "failing sops -e leaves .enc byte-identical" test "$(sha256sum "$work/dns/terraform.tfstate.enc" | cut -d' ' -f1)" = "$enc_before"
check "no temp file left behind" test -z "$(ls -A "$work/dns" | grep '^\.terraform\.tfstate\.enc\.tmp\.' || true)"
check "lock cleaned up on failure" test ! -e "$work/dns/.lock"

# Scenario 2: state changed, sops -e succeeds.
"$work/dns/save.sh" >"$work/ok.log" 2>&1
check "succeeding sops -e exits 0" test $? -eq 0
check "new ciphertext installed" cmp -s "$work/dns/terraform.tfstate.enc" "$work/dns/terraform.tfstate"
check "lock cleaned up on success" test ! -e "$work/dns/.lock"

if [ "$fail" -ne 0 ]; then
  echo "--- fast.log ---" >&2; cat "$work/fast.log" >&2
  echo "--- fail.log ---" >&2; cat "$work/fail.log" >&2
  echo "--- ok.log ---" >&2; cat "$work/ok.log" >&2
  exit 1
fi
echo "all save.sh atomicity checks passed"
