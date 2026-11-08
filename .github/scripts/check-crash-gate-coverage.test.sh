#!/usr/bin/env bash
# check-crash-gate-coverage.test.sh — tests for check-crash-gate-coverage.sh (#2963).
#
# The guard enumerates #[ignore]d crash tests with an awk state machine.
# These fixtures pin the declaration shapes it must see — visibility
# modifiers, async fn, indentation inside a mod block, blank lines
# between the attribute and the item, and #[ignore = "..."] reasons
# wrapped across lines — plus the fail-loudly behaviour for a #[ignore]
# it cannot pair with a fn declaration, and the unchanged behaviour for
# the plain shape and the corpus-writer exclusions.
#
# Each case runs the real guard in a scratch tree (the guard resolves
# its inputs relative to its own path) with the real pkgs/id/justfile.
#
# Runs anywhere bash+awk exist:
#   .github/scripts/check-crash-gate-coverage.test.sh
set -u

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
guard="$script_dir/check-crash-gate-coverage.sh"
repo_root="$(cd "$script_dir/../.." && pwd)"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/.github/scripts" "$tmp/pkgs/id/tests/crash"
cp "$guard" "$tmp/.github/scripts/check-crash-gate-coverage.sh"
cp "$repo_root/pkgs/id/justfile" "$tmp/pkgs/id/justfile"
scratch_guard="$tmp/.github/scripts/check-crash-gate-coverage.sh"
crash_dir="$tmp/pkgs/id/tests/crash"

failures=0
pass() { echo "ok - $1"; }
fail() { echo "FAIL - $1" >&2; failures=$((failures + 1)); }

assert_exit() { # <desc> <expected> <actual>
  if [ "$2" -eq "$3" ]; then pass "$1"; else fail "$1 (exit $3, want $2)"; fi
}
assert_contains() { # <desc> <haystack> <needle>
  case "$2" in
    *"$3"*) pass "$1" ;;
    *) fail "$1 (missing: $3)" ;;
  esac
}

run_guard() { # writes the fixture passed on stdin, prints guard output; exit = guard exit
  cat >"$crash_dir/fixture.rs"
  "$scratch_guard" 2>&1
}

COVERED_HEAD='#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
'

# --- plain shape, covered: passes -------------------------------------
out="$(printf '%s' "$COVERED_HEAD" | run_guard)"
code=$?
assert_exit "plain covered shape passes" 0 "$code"
assert_contains "plain covered shape counted" "$out" "covers all 1"

# --- plain shape, uncovered: fails naming the test ---------------------
out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "gate"]
fn brand_new_gate_runs_nowhere() {}
EOF
)"
code=$?
assert_exit "plain uncovered shape fails" 1 "$code"
assert_contains "plain uncovered shape named" "$out" "brand_new_gate_runs_nowhere"

# --- evasion shapes with an uncovered name: each must fail by name -----
evasion_case() { # <desc> <test-name>
  code=$?
  assert_exit "$1 fails" 1 "$code"
  assert_contains "$1 named" "$out" "$2"
}

out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "gate"]
pub fn pub_gate_runs_nowhere() {}
EOF
)"
evasion_case "pub fn shape" "pub_gate_runs_nowhere"

out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "gate"]
pub(crate) fn pub_crate_gate_runs_nowhere() {}
EOF
)"
evasion_case "pub(crate) fn shape" "pub_crate_gate_runs_nowhere"

out="$(
  cat <<'EOF' | run_guard
#[tokio::test]
#[ignore = "gate"]
async fn async_gate_runs_nowhere() {}
EOF
)"
evasion_case "async fn shape" "async_gate_runs_nowhere"

out="$(
  cat <<'EOF' | run_guard
mod nested {
    #[test]
    #[ignore = "gate"]
    fn indented_gate_runs_nowhere() {}
}
EOF
)"
evasion_case "indented (mod) shape" "indented_gate_runs_nowhere"

out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "gate"]

fn blank_line_gate_runs_nowhere() {}
EOF
)"
evasion_case "blank-line shape" "blank_line_gate_runs_nowhere"

out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "a reason long enough \
to wrap across two lines"]
fn wrapped_gate_runs_nowhere() {}
EOF
)"
evasion_case "wrapped-reason shape" "wrapped_gate_runs_nowhere"

# --- evasion shape with a covered name: enumerated, so it passes -------
out="$(
  cat <<'EOF' | run_guard
#[tokio::test]
#[ignore = "gate"]
async fn covered_full_lying_sync_control_produces_failures_gate() {}
EOF
)"
code=$?
assert_exit "async covered shape passes" 0 "$code"
assert_contains "async covered shape counted" "$out" "covers all 1"

# --- corpus-writer exclusions still apply -------------------------------
out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "corpus"]
fn write_corpus() {}

#[test]
#[ignore = "gate"]
fn covered_ingest_survives_every_crash_point_gate() {}
EOF
)"
code=$?
assert_exit "corpus writer excluded, covered gate passes" 0 "$code"
assert_contains "exclusion reported" "$out" "corpus writers excluded"

# --- a #[ignore] with no fn after it fails loudly ------------------------
out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "gate"]
const NOT_A_TEST: u32 = 0;
EOF
)"
code=$?
assert_exit "unpairable #[ignore] fails" 1 "$code"
assert_contains "unpairable #[ignore] is named as drift" "$out" "not followed by a recognizable fn declaration"

# --- Codex findings on PR #4272: raw strings, non-ASCII names, comments --
out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = r#"requires support for the '"' delimiter"#]
fn uncovered_delimiter_gate() {}

#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
EOF
)"
code=$?
assert_exit "raw-string reason shape fails" 1 "$code"
assert_contains "raw-string reason shape named" "$out" "uncovered_delimiter_gate"

out="$(
  cat <<'EOF' | run_guard
#[test]
#[ignore = "corpus"]
fn write_corpusé() {}

#[test]
#[ignore = "gate"]
fn covered_ingest_survives_every_crash_point_gate() {}
EOF
)"
code=$?
assert_exit "non-ASCII name is not truncated into the corpus exclusion" 1 "$code"
assert_contains "non-ASCII name preserved whole" "$out" "write_corpusé"

out="$(
  cat <<'EOF' | run_guard
// These #[ignore]d tests run only in release.
// See the crash-gate recipes for invocation.
/* A block comment mentioning #[ignore] is not an attribute either. */
#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
EOF
)"
code=$?
assert_exit "comment mentions of #[ignore] are ignored" 0 "$code"
assert_contains "comment-mention fixture counted" "$out" "covers all 1"

# --- Codex round 2 on PR #4272: comment-close code, same-line attrs ----
out="$(
  cat <<'EOF' | run_guard
/* a block comment that starts here
   and ends on this line */ #[ignore = "gate"]
fn uncovered_after_block_comment_gate() {}

#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
EOF
)"
code=$?
assert_exit "code after a block comment closes is processed" 1 "$code"
assert_contains "test after block comment named" "$out" "uncovered_after_block_comment_gate"

out="$(
  cat <<'EOF' | run_guard
#[test] #[ignore = "gate"]
fn uncovered_same_line_attrs_gate() {}

#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
EOF
)"
code=$?
assert_exit "same-line #[test] #[ignore] is enumerated" 1 "$code"
assert_contains "same-line test named" "$out" "uncovered_same_line_attrs_gate"

# --- generated invariant coverage (PBT-style; testing doctrine) --------
# Oracle: generated names are known by construction. For every generated
# declaration shape, an ignored test whose name contains no split filter
# MUST be named by a failing guard -- it can never silently disappear --
# and one whose name contains a filter MUST pass with its count intact.
gen_i=0
for gen_vis in "" "pub " "pub(crate) "; do
  for gen_async in "" "async "; do
    for gen_layout in sep same; do
      for gen_parity in 0 1; do
        gen_i=$((gen_i + 1))
        case $((gen_i % 3)) in
          0) gen_comment="// generated: #[ignore] in a comment is not an attribute" ;;
          1) gen_comment="/* generated: #[ignore] in a block comment */" ;;
          *) gen_comment="" ;;
        esac
        case $((gen_i % 2)) in
          0) gen_reason='"gate"' ;;
          *) gen_reason='r#"gate"#' ;;
        esac
        if [ $((gen_i % 2)) -eq 0 ]; then gen_indent="    "; else gen_indent=""; fi
        if [ "$gen_parity" -eq 0 ]; then gen_name="gen_uncovered_${gen_i}_gate"; gen_want=1; else gen_name="gen_covered_${gen_i}_full_sweep_has_no_failures_gate"; gen_want=0; fi
        out="$(
          {
            [ -n "$gen_comment" ] && printf '%s\n' "$gen_comment"
            [ -n "$gen_indent" ] && printf '%s\n' "mod gen_mod_${gen_i} {"
            if [ "$gen_layout" = same ]; then
              printf '%s%s\n' "$gen_indent" "#[test] #[ignore = $gen_reason]"
            else
              printf '%s%s\n%s%s\n' "$gen_indent" "#[test]" "$gen_indent" "#[ignore = $gen_reason]"
            fi
            printf '%s%s%sfn %s() {}\n' "$gen_indent" "$gen_vis" "$gen_async" "$gen_name"
            [ -n "$gen_indent" ] && printf '%s\n' "}"
          } | run_guard
        )"
        code=$?
        assert_exit "generated case $gen_i exit (vis='$gen_vis' async='$gen_async' layout=$gen_layout parity=$gen_parity)" "$gen_want" "$code"
        if [ "$gen_want" -eq 1 ]; then assert_contains "generated case $gen_i named" "$out" "$gen_name"; else assert_contains "generated case $gen_i counted" "$out" "covers all 1"; fi
      done
    done
  done
done

# --- Codex round 2 on PR #4272: escaped char literal ends at its quote --
out="$(
  cat <<'EOF' | run_guard
const TEXT: &str = { let _ = '\\'; "hello
world" };

#[test]
#[ignore = "gate"]
fn uncovered_after_char_literal_gate() {}

#[test]
#[ignore = "gate"]
fn covered_full_sweep_has_no_failures_gate() {}
EOF
)"
code=$?
assert_exit "escaped char literal does not desync the lexer" 1 "$code"
assert_contains "test after char literal and multiline string is named" "$out" "uncovered_after_char_literal_gate"
if [ "$failures" -ne 0 ]; then
  echo "$failures check-crash-gate-coverage test(s) FAILED" >&2
  exit 1
fi
echo "all check-crash-gate-coverage tests passed"
