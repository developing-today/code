"""Tests for the repo-queue worker shift roster reconciliation (#15341).

Oracle: issue #15341's first acceptance criterion — the registry's 22
shift jobs (2026-11-03 review) reconciled against the 33 lane
definitions in the newer repo projections, with the generation
difference explained. The expected rosters are the ones of record in
the #14222 inventory (rows 5-7) and in
doc/2026-11-26T07-50-00Z_reconciliation_repo-queue-shift-roster/.

The test parses doc/fleet-schedule-registry.md (generation 1, Surface
1) and the reconciliation record (generation 2), and asserts the
counts, the disabled pair, the complete enumeration of both rosters,
and the zero id overlap that makes the two rosters read as disjoint
in a name-based diff.

Run: python3 scripts/test_fleet_schedule_registry_roster.py
"""

import os
import re
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REGISTRY = os.path.join(ROOT, "doc", "fleet-schedule-registry.md")
RECORD = os.path.join(
    ROOT,
    "doc",
    "2026-11-26T07-50-00Z_reconciliation_repo-queue-shift-roster",
    "2026-11-26T07-50-00Z_reconciliation_repo-queue-shift-roster.md",
)

PREFIX = "repo-queue-worker-shift"

CODE_PILOTS = [PREFIX] + [f"{PREFIX}-{c}" for c in "bcdefghijklmnop"]
GENERATION_1 = set(
    CODE_PILOTS
    + [
        f"{PREFIX}-nix",
        f"{PREFIX}-nix-b",
        f"{PREFIX}-openchamber",
        f"{PREFIX}-openchamber-b",
        f"{PREFIX}-opencode",
        f"{PREFIX}-opencode-b",
    ]
)
GENERATION_1_DISABLED = {f"{PREFIX}-nix-b", f"{PREFIX}-opencode-b"}

GENERATION_2 = set(
    [f"{PREFIX}-any-{c}" for c in "abcdefghijklmnopqrstuvwxy"]
    + [f"{PREFIX}-code-{c}" for c in "abcde"]
    + [f"{PREFIX}-not-code-{c}" for c in "abc"]
)


def registry_shift_rows():
    """Map each generation-1 job id in the registry to its state."""
    rows = {}
    with open(REGISTRY, encoding="utf-8") as fh:
        for line in fh:
            m = re.match(r"\| `(repo-queue-worker-shift[^`]*)`\s*\|", line)
            if m:
                state = re.search(r"\|\s*(enabled|disabled)\s*\|", line)
                rows[m.group(1)] = state.group(1) if state else None
    return rows


def record_ids():
    with open(RECORD, encoding="utf-8") as fh:
        return set(re.findall(r"`(repo-queue-worker-shift[^`]*)`", fh.read()))


class RosterReconciliationTest(unittest.TestCase):
    def test_roster_sizes(self):
        self.assertEqual(len(GENERATION_1), 22)
        self.assertEqual(len(GENERATION_2), 33)

    def test_registry_holds_exactly_generation_1(self):
        rows = registry_shift_rows()
        self.assertEqual(set(rows), GENERATION_1)

    def test_registry_disabled_pair(self):
        rows = registry_shift_rows()
        disabled = {k for k, v in rows.items() if v == "disabled"}
        enabled = {k for k, v in rows.items() if v == "enabled"}
        self.assertEqual(disabled, GENERATION_1_DISABLED)
        self.assertEqual(len(enabled), 20)

    def test_record_enumerates_both_generations(self):
        ids = record_ids()
        self.assertTrue(GENERATION_1 <= ids, GENERATION_1 - ids)
        self.assertTrue(GENERATION_2 <= ids, GENERATION_2 - ids)

    def test_generations_have_no_id_overlap(self):
        self.assertEqual(GENERATION_1 & GENERATION_2, set())


if __name__ == "__main__":
    unittest.main()
