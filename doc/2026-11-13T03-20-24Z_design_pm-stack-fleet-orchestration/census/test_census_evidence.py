# lane-scratch: code-c /tmp/lane-code-c-20261116-0412 task 15934
#!/usr/bin/env python3
"""CEN-02 — pinned V1 source/test and dated-issue evidence audit (#9433).

Independently re-checks the positive and negative evidence claims in
census/requirements-crosswalk.jsonl against the PINNED source tree
(a038f084dbeb525494d7bb928ad91f597db2ac51) and the row's own dated
evidence fields. Source census files (track-a.md, track-b.md,
commands.jsonl, anchors.json) are consumed read-only.

Bounded-claim doctrine (the oracle): a capability claim is supported
ONLY by evidence that names the EXACT symbol of that capability in a
real source/test artifact at the pin. None of the following can
upgrade a claim about a separate target capability:

  * an ADJACENT VERB's evidence (work.note.add != work.note.edit);
  * a CLOSED PARENT ISSUE (issue state is an observation about the
    parent, not proof of the target capability);
  * a MOCK (a test double proves the double, not the capability);
  * a PROJECTOR DELIVERY ACK (a webhook delivery acknowledgment is
    transport evidence for the projector, not Fleet command
    acceptance — and #8187 evidence is not #6533 proof).

Evidence that is descriptive prose (a phrase describing an area, not
a literal symbol) is classified `descriptive_bounded`: recorded, never
upgraded to `exact`. Dated audit results are additive: this file's
--write-audit mode appends an `evidence_audit` object to each JSONL
row; it never rewrites a row's status or its pinned claims.

Run:  python3 test_census_evidence.py            (tests)
      python3 test_census_evidence.py --write-audit   (tests + record)
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

CENSUS_DIR = Path(__file__).resolve().parent
REPO_ROOT = CENSUS_DIR.parents[3]  # doc/<design>/census -> repo root
CROSSWALK_JSONL = CENSUS_DIR / "requirements-crosswalk.jsonl"
PIN = "a038f084dbeb525494d7bb928ad91f597db2ac51"
POSITIVE_STATUSES = ("verified existing", "partially implemented")
REQUIRED_EVIDENCE_FIELDS = ("pin", "path", "symbol", "lines", "test", "timestamp")
AUDITED_AT = "2026-11-16T10:40:00Z"  # task 15934 audit run (lane code-c)

_file_cache: dict[str, list[str] | None] = {}


def pinned_file_lines(path: str) -> list[str] | None:
    """Lines of `path` at the pinned commit, or None if absent there."""
    if path not in _file_cache:
        proc = subprocess.run(
            ["git", "show", f"{PIN}:{path}"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
        _file_cache[path] = proc.stdout.splitlines() if proc.returncode == 0 else None
    return _file_cache[path]


def split_evidence_list(value: str) -> list[str]:
    return [part.strip() for part in value.split(";") if part.strip()]


def parse_line_ranges(lines_field: str) -> list[tuple[int, int]]:
    """All (lo, hi) ranges mentioned in a census `lines` field."""
    ranges: list[tuple[int, int]] = []
    for m in re.finditer(r"(\d+)\s*[-–]\s*(\d+)", lines_field):
        ranges.append((int(m.group(1)), int(m.group(2))))
    return ranges


def classify_symbol(symbol: str, windows: list[str]) -> str:
    """`exact` iff the symbol string appears literally in a cited window.

    Anything else is a descriptive phrase: bounded evidence about an
    area, never an exact-symbol verification.
    """
    joined = "\n".join(windows)
    return "exact" if symbol in joined else "descriptive_bounded"


def classify_test(test_field: str) -> str:
    """`artifact` iff the field names a path that exists at the pin."""
    for token in re.split(r"[;\s]+", test_field):
        token = token.strip(".,()")
        if "/" in token and pinned_file_lines(token) is not None:
            return "artifact"
    return "note_only"


def evidence_supports(claim_symbol: str, evidence: dict) -> bool:
    """The bounded-claim predicate the oracle pins down.

    Evidence supports a capability claim only when ALL hold:
      * the evidence symbol is EXACTLY the claimed symbol (no
        adjacent verbs, no parent/child substitution);
      * the evidence kind is a real source or test artifact
        (issue observations and delivery ACKs are not capability
        evidence);
      * the evidence is not a mock;
      * the symbol was verified literally at the pin (class `exact`).
    """
    if evidence.get("symbol") != claim_symbol:
        return False
    if evidence.get("kind") not in ("source", "test"):
        return False
    if evidence.get("mock"):
        return False
    if evidence.get("symbol_class") != "exact":
        return False
    return True


def audit_row(row: dict) -> dict:
    """Table-driven audit of one crosswalk row (see module docstring)."""
    record: dict = {"audited_at": AUDITED_AT, "pin": PIN}
    status = row.get("status")
    if status not in POSITIVE_STATUSES:
        missing = row.get("missing_evidence") or []
        record["verdict"] = "missing_recorded" if missing else "MISSING_EVIDENCE_ABSENT"
        record["missing_evidence_count"] = len(missing)
        return record

    ev = row.get("existing_evidence") or {}
    problems: list[str] = []
    for field in REQUIRED_EVIDENCE_FIELDS:
        if not ev.get(field):
            problems.append(f"evidence field absent: {field}")
    if ev.get("pin") != PIN:
        problems.append(f"pin mismatch: {ev.get('pin')}")
    paths = split_evidence_list(ev.get("path", ""))
    windows: list[str] = []
    max_lines: dict[str, int] = {}
    for path in paths:
        lines = pinned_file_lines(path)
        if lines is None:
            problems.append(f"path absent at pin: {path}")
        else:
            max_lines[path] = len(lines)
    ranges = parse_line_ranges(ev.get("lines", ""))
    if not ranges:
        problems.append("no parseable line ranges")
    else:
        # A range is in-bounds if it fits ANY cited file (segments in
        # the census field are per-file; the binding is by order, and
        # several rows cite one file). Out-of-bounds everywhere = bad.
        for lo, hi in ranges:
            if not any(lo >= 1 and hi <= n for n in max_lines.values()):
                problems.append(f"line range out of bounds: {lo}-{hi}")
        for path in paths:
            lines = pinned_file_lines(path) or []
            for lo, hi in ranges:
                if hi <= len(lines):
                    windows.append("\n".join(lines[lo - 1 : hi]))
    record["paths_checked"] = len(paths)
    record["symbol_class"] = classify_symbol(ev.get("symbol", ""), windows)
    record["test_class"] = classify_test(ev.get("test", ""))
    record["verdict"] = "audited" if not problems else "AUDIT_PROBLEMS"
    if problems:
        record["problems"] = problems
    return record


def load_rows() -> list[dict]:
    return [
        json.loads(l) for l in CROSSWALK_JSONL.read_text().splitlines() if l.strip()
    ]


# Spot checks named by the CEN-02 done-check: pinned artifacts whose
# cited line ranges must exist at the pin (projector webhook intake,
# delivery-id test, validate.ts snapping, turso_queue claim/filter).
SPOT_CHECKS = [
    ("pm-stack/projector/src/webhook.ts", 7, 57),
    ("pm-stack/projector/src/server.ts", 147, 201),
    ("pm-stack/projector/test/webhook-delivery-id.test.ts", 90, 154),
    ("pm-stack/dispatch/src/validate.ts", 11, 60),
    ("pkgs/fleet/crates/fleet-queue/src/turso_queue.rs", 3110, 3129),
]


class TestCensusEvidence(unittest.TestCase):
    def test_census_evidence_requires_exact_symbol_and_bounded_claim(self):
        """The CEN-02 oracle: counterexamples cannot upgrade a claim."""
        target = "work.note.edit"
        exact = {
            "symbol": target,
            "kind": "source",
            "mock": False,
            "symbol_class": "exact",
        }
        # Positive control: exact-symbol source evidence supports.
        self.assertTrue(evidence_supports(target, exact))

        # Counterexample 1 — adjacent verb: evidence for the sibling
        # verb work.note.add says nothing about work.note.edit.
        adjacent = dict(exact, symbol="work.note.add")
        self.assertFalse(evidence_supports(target, adjacent))

        # Counterexample 2 — closed parent issue: an issue observation
        # (even a closed parent) is not capability evidence.
        closed_parent = dict(exact, kind="issue")
        self.assertFalse(evidence_supports(target, closed_parent))

        # Counterexample 3 — mock: a test double proves the double.
        mock = dict(exact, kind="test", mock=True)
        self.assertFalse(evidence_supports(target, mock))

        # Counterexample 4 — projector delivery ACK: transport
        # acknowledgment for the projector is not Fleet command
        # acceptance of the target capability.
        ack = dict(exact, kind="ack")
        self.assertFalse(evidence_supports(target, ack))

        # And a descriptive (non-literal) symbol never upgrades, even
        # from a real source artifact.
        descriptive = dict(exact, symbol_class="descriptive_bounded")
        self.assertFalse(evidence_supports(target, descriptive))

    def test_audit_covers_every_row(self):
        rows = load_rows()
        self.assertEqual(len(rows), 389)
        for row in rows:
            record = audit_row(row)
            self.assertIn("verdict", record, row["requirement_id"])
            if row["status"] in POSITIVE_STATUSES:
                # Required evidence fields present + pin correct +
                # every cited path exists at the pin + ranges parse.
                self.assertEqual(record["verdict"], "audited", row["requirement_id"])
                self.assertIn(record["symbol_class"], ("exact", "descriptive_bounded"))
                self.assertIn(record["test_class"], ("artifact", "note_only"))
            else:
                # Explicit missing evidence is recorded for every
                # non-positive row — no silent gaps.
                self.assertEqual(
                    record["verdict"], "missing_recorded", row["requirement_id"]
                )

    def test_spot_checks_pinned_artifacts(self):
        for path, lo, hi in SPOT_CHECKS:
            lines = pinned_file_lines(path)
            self.assertIsNotNone(lines, f"{path} absent at pin")
            self.assertGreaterEqual(len(lines), hi, f"{path} shorter than {hi}")


def write_audit() -> None:
    rows = load_rows()
    with CROSSWALK_JSONL.open("w") as fh:
        for row in rows:
            row["evidence_audit"] = audit_row(row)
            fh.write(json.dumps(row) + "\n")
    positives = [r for r in rows if r["status"] in POSITIVE_STATUSES]
    exact = sum(1 for r in positives if r["evidence_audit"]["symbol_class"] == "exact")
    print(
        f"audited {len(rows)} rows: {len(positives)} positive "
        f"({exact} exact-symbol, {len(positives) - exact} descriptive_bounded), "
        f"{len(rows) - len(positives)} missing-evidence recorded"
    )


if __name__ == "__main__":
    if "--write-audit" in sys.argv:
        sys.argv.remove("--write-audit")
        write_audit()
    unittest.main(verbosity=2)
