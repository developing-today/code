#!/usr/bin/env bash
# Planted-bug checks for membership / voiding (P2.3 / P2.8, #1395) and the tag
# observed-remove fold (P5.1, #1421).
#
# A test suite that passes both the correct and a broken implementation proves
# nothing. This script applies one named mutation at a time from tests/planted/,
# runs the suites that are supposed to catch it, and requires each run to FAIL.
# The unmutated baseline must pass first, so a red baseline cannot fake a catch.
#
# Usage:  pkgs/id/tests/planted-bug-check.sh        (or: just planted-bug-check)
# Exit:   0 = baseline green and every planted bug caught
#         1 = a planted bug survived or missed its expected regression test
#         2 = baseline red, a patch could not be applied/reversed, or the tree was dirty
#
# Applied patches are reversed after every run, including on error/interrupt.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ID_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PATCH_DIR="$SCRIPT_DIR/planted"
REPO_ROOT="$(git -C "$ID_DIR" rev-parse --show-toplevel)"

# Files the mutations touch, relative to the repo root.
MUTATED_FILES=(
  "pkgs/id/crates/id-core/src/membership/layer.rs"
  "pkgs/id/crates/id-core/src/membership/frontier.rs"
  "pkgs/id/src/groups/delegation.rs"
  "pkgs/id/src/tag_claims.rs"
)

APPLIED_PATCHES=()

restore() {
  local i patch
  for ((i = ${#APPLIED_PATCHES[@]} - 1; i >= 0; i--)); do
    patch="${APPLIED_PATCHES[$i]}"
    if ! git -C "$REPO_ROOT" apply --reverse "$patch"; then
      echo "planted-bug-check: could not reverse applied patch $patch" >&2
      return 1
    fi
  done
  APPLIED_PATCHES=()
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ! restore; then
    status=2
  fi
  exit "$status"
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for f in "${MUTATED_FILES[@]}"; do
  if [ -n "$(git -C "$REPO_ROOT" status --porcelain -- "$f")" ]; then
    echo "planted-bug-check: $f has uncommitted changes; refusing to mutate it" >&2
    exit 2
  fi
done

cd "$ID_DIR"

# Suites per mutation. The voiding suites judge grants/removals in the groups
# engine; the frontier unit tests pin the mapping directly and the membership
# suites exercise it through the layer.
SUITES_voiding_authority="cargo test --test voiding_scenarios --test voiding_properties"
SUITES_delegation_voiding="cargo test --test delegation_voiding_scenarios"
SUITES_frontier_mapping="cargo test -p id-core --lib membership::frontier && cargo test --test membership_properties --test membership_scenarios"
SUITES_tag_claims="cargo test --test tag_claims"

run_suites() {
  # $1 = space-separated suite commands joined by &&
  eval "$1" >"$2" 2>&1
}

echo "== baseline: all suites must pass unmutated =="
if ! run_suites "$SUITES_voiding_authority && $SUITES_frontier_mapping && $SUITES_delegation_voiding && $SUITES_tag_claims" /tmp/planted-bug-baseline.log; then
  echo "planted-bug-check: BASELINE RED — suites fail without any mutation; see /tmp/planted-bug-baseline.log" >&2
  exit 2
fi
echo "baseline green"

failed=0
check_mutation() {
  local name="$1" patch="$2" suites="$3" expected_failure="${4:-}"
  echo "== mutation: $name =="
  if ! git -C "$REPO_ROOT" apply "$patch"; then
    echo "planted-bug-check: patch $patch did not apply (source moved?); re-derive the mutation" >&2
    exit 2
  fi
  APPLIED_PATCHES+=("$patch")
  if run_suites "$suites" "/tmp/planted-bug-$name.log"; then
    echo "planted-bug-check: SURVIVED — $name was NOT caught; the suites passed with the bug planted (see /tmp/planted-bug-$name.log)" >&2
    failed=1
  elif [ -n "$expected_failure" ] && ! grep -Eq "^test ${expected_failure}[[:space:]]+\.\.\. FAILED$" "/tmp/planted-bug-$name.log"; then
    echo "planted-bug-check: $name failed without the expected test $expected_failure failing (see /tmp/planted-bug-$name.log)" >&2
    failed=1
  else
    echo "caught: $name makes the suites fail, as required"
  fi
  if ! restore; then
    echo "planted-bug-check: failed to restore after $name" >&2
    exit 2
  fi
}

# voiding-authority: the resolver-filter verdict in
# Membership::is_effective is inverted, so the concurrent operations of a
# removed member take effect instead of being voided — the authority
# voiding the event-store resolve path (pass two) actually runs for the
# S2 suites. Plant history (task 6582): two mutations at Engine::void in
# src/groups/delegation.rs (strong-removal gate, causal-past) SURVIVED
# these suites, because voiding_scenarios / voiding_properties exercise
# authority voiding through the event store and contain no
# delegation-grant coverage, so nothing in them reaches Engine::void.
# The plant therefore lives here, in the exercised path. Grant-voiding
# scenarios reaching Engine::void were the separate gap #7071; they now
# exist (delegation_voiding_scenarios) and carry their own plant below.
check_mutation "voiding-authority" "$PATCH_DIR/voiding-authority.patch" "$SUITES_voiding_authority"
# frontier-mapping: a group operation no longer contributes itself to the
# auth frontier (the mapping test in CausalIndex::combine is inverted).
check_mutation "frontier-mapping" "$PATCH_DIR/frontier-mapping.patch" "$SUITES_frontier_mapping"
# delegation-voiding: the strong-removal gate at Engine::void in
# src/groups/delegation.rs is inverted, so a grant concurrent with its
# grantor's removal stands under strong remove and is voided under weak
# remove — exactly backwards (design §6.2/§6.3). The delegation_voiding
# scenarios judge a grant, its grantor's concurrent removal, and the
# grantee's onward operation through the groups fold (#7071).
check_mutation "delegation-voiding" "$PATCH_DIR/delegation-voiding.patch" "$SUITES_delegation_voiding"
# tag-clear-unseen: compacted_claims removes a named claim even when the clear
# did not see it. The unseen-claim regression test is the oracle.
check_mutation "tag-clear-unseen" "$PATCH_DIR/tag-clear-unseen.patch" "$SUITES_tag_claims" "test_a_clear_of_an_unseen_claim_does_nothing_to_it"
# tag-clear-transitive-history: compacted_claims forgets claims inherited
# through a parent event. The whole-causal-past regression test is the oracle.
check_mutation "tag-clear-transitive-history" "$PATCH_DIR/tag-clear-transitive-history.patch" "$SUITES_tag_claims" "test_a_clear_removes_a_claim_its_frontier_reaches_only_through_another_author"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "planted-bug-check: OK — baseline green, all planted bugs caught"
