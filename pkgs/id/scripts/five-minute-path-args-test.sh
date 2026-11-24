#!/usr/bin/env bash
# Regression test for five-minute-path.sh argument contract (#12595, #3352).
# Unaccounted arguments, missing values, and invalid values must be
# rejected with exit 2 before any tool runs; --help and --dry-run must
# exit 0 without running any tool.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/five-minute-path.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "five-minute-path-args-test: FAIL: $1" >&2
  exit 1
}

# Stub git/cargo to prove no clone/build work happens when arg
# rejection is broken (rejection must occur before tool checks).
mkdir -p "$tmp/bin"
for tool in git cargo; do
  cat >"$tmp/bin/$tool" <<'SH'
#!/usr/bin/env bash
echo "$0-ran" >>"$TRACE"
exit 0
SH
  chmod +x "$tmp/bin/$tool"
done

export TRACE="$tmp/trace"
PATH="$tmp/bin:$PATH"

# Every case below must exit 2, name the offending argument, and run
# no tool.
run_reject() {
  local desc="$1"
  shift
  if (cd "$tmp" && bash "$script" "$@" >out 2>err); then
    fail "$desc was accepted"
  fi
  [ ! -f "$TRACE" ] || fail "$desc ran a tool despite rejection"
  grep -q 'usage error' "$tmp/err" || fail "$desc printed no usage error"
  grep -q 'usage: five-minute-path.sh' "$tmp/err" || fail "$desc printed no usage line"
}

run_reject "unexpected positional" unexpected
run_reject "unknown flag" --bogus
run_reject "unknown flag with value" --bogus=1
run_reject "missing --dir value" --dir
run_reject "empty --dir=value" --dir=
run_reject "missing --budget value" --budget
run_reject "non-numeric --budget" --budget soon
run_reject "zero --budget" --budget 0
run_reject "out-of-range --iroh-port" --iroh-port 70000
run_reject "bad --world name" --world 'UPPER SPACE'
run_reject "missing --id-bin file" --id-bin "$tmp/no-such-id"
run_reject "non-empty --dir" --dir "$tmp/bin"

# --help exits 0 and prints the contract.
bash "$script" --help >"$tmp/help" 2>"$tmp/help.err" || fail "--help failed"
grep -q 'Usage: five-minute-path.sh' "$tmp/help" || fail "--help printed no usage"
grep -q 'Argument contract (#3352)' "$tmp/help" || fail "--help printed no contract note"
[ ! -f "$TRACE" ] || fail "--help ran a tool"

# --dry-run exits 0, prints the steps, and runs no tool.
bash "$script" --dry-run --dir "$tmp/run" >"$tmp/dry" 2>"$tmp/dry.err" || fail "--dry-run failed"
grep -q 'git clone' "$tmp/dry" || fail "--dry-run printed no clone step"
grep -q 'cargo build' "$tmp/dry" || fail "--dry-run printed no build step"
grep -q 'world join' "$tmp/dry" || fail "--dry-run printed no join step"
[ ! -f "$TRACE" ] || fail "--dry-run ran a tool"
[ ! -e "$tmp/run" ] || fail "--dry-run created the run directory"

# --dry-run with --id-bin skips the clone/build steps.
bash "$script" --dry-run --id-bin "$tmp/bin/git" >"$tmp/dry-bin" 2>&1 || fail "--dry-run --id-bin failed"
grep -q 'clone: skipped' "$tmp/dry-bin" || fail "--dry-run --id-bin printed no clone skip"
grep -q 'build: skipped' "$tmp/dry-bin" || fail "--dry-run --id-bin printed no build skip"

echo "five-minute-path-args tests passed"
