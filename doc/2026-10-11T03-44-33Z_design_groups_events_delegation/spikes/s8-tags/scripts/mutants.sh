#!/usr/bin/env bash
# Plant one bug per copy of the crate, run the property tests on it, and record the result.
# Each copy builds in its own target dir, so a mutant cannot leak into the main build.
# Usage: CARGO="$(rustup which --toolchain 1.97.0 cargo)" bash scripts/mutants.sh
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 1
export CARGO_TARGET_DIR="$root/mutants/target"
mkdir -p logs
: > logs/mutants.txt

mut() {
  local name=$1 expr=$2 dir="mutants/$1"
  rm -rf "$dir"; mkdir -p "$dir"
  cp -r Cargo.toml Cargo.lock src tests "$dir"/
  perl -0pi -e "$expr" "$dir/src/lib.rs"
  if cmp -s src/lib.rs "$dir/src/lib.rs"; then
    echo "== $name: mutation did not apply" >> logs/mutants.txt; return
  fi
  (cd "$dir" && "$CARGO" test --test props --no-fail-fast 2>&1) > "logs/mutant_$name.txt"
  {
    echo "== $name"
    grep -E "^test |test result" "logs/mutant_$name.txt"
    grep -E "minimal failing input|Test failed|assertion" "logs/mutant_$name.txt" | head -4
  } >> logs/mutants.txt
}

# m1: a clear removes every named dot, ignoring its clock (breaks unobserved-remove).
mut m1_clear_ignores_clock 's/filter\(\|dot\| covers\(clock, dot\)\)\s*\{/filter(|_| true) {/'
# m2: a clear is applied without waiting for its clock (breaks out-of-order delivery).
mut m2_clear_not_gated_on_clock 's/Body::Clear \{ clock, \.\. \} => clock\s*\.iter\(\)\s*\.all\(\|\(device, seq\)\| vv\.get\(device\)\.copied\(\)\.unwrap_or\(0\) >= \*seq\),/Body::Clear { .. } => true,/'
# m4: the join keeps every dot of the left side (asymmetric, so the laws catch it).
mut m4_join_keeps_all 's/if b\.live\.contains_key\(dot\) \|\| !covers\(&b\.vv, dot\) \{/if true {/'
# m4b: the join keeps every dot on both sides (symmetric, so only the oracle check catches it).
mut m4b_join_union_symmetric 's/if b\.live\.contains_key\(dot\) \|\| !covers\(&b\.vv, dot\) \{/if true {/; s/if !a\.live\.contains_key\(dot\) && !covers\(&a\.vv, dot\) \{/if true {/'
# m5: the join ignores dots that only the right side holds (breaks commutativity).
mut m5_join_one_sided 's/for \(dot, entry\) in &b\.live \{/for (dot, entry) in std::iter::empty::<(\&Dot, \&Entry)>() {/'
