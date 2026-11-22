#!/usr/bin/env python3
"""Tests for census/check_requirements_crosswalk.py (CEN-01, #9500).

Run: python3 -m unittest test_requirements_crosswalk -v
(or execute this file directly). The live-data test validates the real
census ledgers; the synthetic tests name each rejected defect class.
"""

import copy
import importlib.util
import json
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

spec = importlib.util.spec_from_file_location(
    "check_requirements_crosswalk", HERE / "check_requirements_crosswalk.py"
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def synthetic_inputs():
    row = {
        "requirement_id": "C-R001",
        "status": "design-only",
        "modality": "requirement",
        "candidate_disposition": None,
        "source_occurrence_ids": ["C-001"],
        "source_refs": [
            {
                "occurrence_id": "C-001",
                "source_file": "02_charter_v1_1.md",
                "line": 1,
                "quote": "x",
            }
        ],
        "child_requirement_ids": [],
        "is_group_node": False,
        "duplicate_of": None,
        "existing_evidence": {
            "pin": None,
            "path": None,
            "symbol": None,
            "lines": None,
            "test": None,
            "timestamp": None,
        },
        "design_refs": ["components/x.md#real-anchor"],
        "command_inventory_refs": [],
    }
    rows = [
        dict(row, requirement_id=f"C-R{i:03d}")
        for i in range(1, checker.EXPECTED_REQUIREMENT_COUNT + 1)
    ]
    dispositions = {
        "C-001": {
            "occurrence_id": "C-001",
            "source_file": "02_charter_v1_1.md",
            "locator": {"line": 1},
            "disposition": "requirement_link",
            "semantic_refs": ["C-R001"],
        }
    }
    commands = {f"cmd.{i}": {} for i in range(checker.EXPECTED_COMMAND_COUNT)}
    slice_map = {
        k: {"primary_slice": "CLI-02", "supporting_slices": []} for k in commands
    }
    anchors = {"components/x.md": {"real-anchor"}}
    return rows, dispositions, commands, slice_map, anchors


def run(rows, dispositions, commands, slice_map, anchors):
    errors, _unmapped = checker.core_errors(
        rows, dispositions, None, commands, slice_map, "pin", anchors
    )
    return errors


class CrosswalkIntegrityTest(unittest.TestCase):
    def test_requirements_crosswalk_rejects_missing_duplicate_occurrence_and_unresolved_design_anchor(
        self,
    ):
        rows, dispositions, commands, slice_map, anchors = synthetic_inputs()

        # Missing occurrence: a citation that resolves to nothing.
        missing = copy.deepcopy(rows)
        missing[0]["source_occurrence_ids"] = ["C-999"]
        missing[0]["source_refs"] = [
            {
                "occurrence_id": "C-999",
                "source_file": "02_charter_v1_1.md",
                "line": 1,
                "quote": "x",
            }
        ]
        errors = run(missing, dispositions, commands, slice_map, anchors)
        self.assertTrue(any("C-999" in e and "missing" in e for e in errors), errors)

        # Duplicate occurrence of a semantic ID: C-R001 appears twice.
        duplicate = copy.deepcopy(rows)
        duplicate[1]["requirement_id"] = "C-R001"
        errors = run(duplicate, dispositions, commands, slice_map, anchors)
        self.assertTrue(
            any("C-R001" in e and "exactly once" in e for e in errors), errors
        )

        # Unresolved design anchor: heading slug that does not exist.
        bad_anchor = copy.deepcopy(rows)
        bad_anchor[0]["design_refs"] = ["components/x.md#no-such-anchor"]
        errors = run(bad_anchor, dispositions, commands, slice_map, anchors)
        self.assertTrue(any("unresolved design anchor" in e for e in errors), errors)

    def test_rejects_wrong_span_status_typo_partial_without_evidence_group_child_and_wildcard(
        self,
    ):
        rows, dispositions, commands, slice_map, anchors = synthetic_inputs()

        wrong_span = copy.deepcopy(rows)
        wrong_span[0]["source_refs"][0]["line"] = 99
        errors = run(wrong_span, dispositions, commands, slice_map, anchors)
        self.assertTrue(any("span line" in e for e in errors), errors)

        status_typo = copy.deepcopy(rows)
        status_typo[0]["status"] = "partialy implemented"
        errors = run(status_typo, dispositions, commands, slice_map, anchors)
        self.assertTrue(any("not in allowed vocabulary" in e for e in errors), errors)

        no_evidence = copy.deepcopy(rows)
        no_evidence[0]["status"] = "partially implemented"
        no_evidence[0]["existing_evidence"] = {
            "pin": "pin",
            "path": "p",
            "symbol": "s",
            "lines": "1",
            "test": "t",
            "timestamp": None,
        }
        errors = run(no_evidence, dispositions, commands, slice_map, anchors)
        self.assertTrue(
            any("without source evidence field 'timestamp'" in e for e in errors),
            errors,
        )

        bad_child = copy.deepcopy(rows)
        bad_child[0]["is_group_node"] = True
        bad_child[0]["child_requirement_ids"] = ["C-R999"]
        errors = run(bad_child, dispositions, commands, slice_map, anchors)
        self.assertTrue(
            any("group child C-R999 not found" in e for e in errors), errors
        )

        wildcard = copy.deepcopy(slice_map)
        wildcard["cmd.0"] = {"primary_slice": "*", "supporting_slices": []}
        errors = run(rows, dispositions, commands, wildcard, anchors)
        self.assertTrue(any("wildcard" in e for e in errors), errors)

    def test_live_census_crosswalk_passes(self):
        crosswalk = checker.load_jsonl(HERE / "requirements-crosswalk.jsonl")
        disposition_rows = checker.load_jsonl(ROOT / "inventory" / "dispositions.jsonl")
        dispositions = {d["occurrence_id"]: d for d in disposition_rows}
        self.assertEqual(
            len(dispositions),
            len(disposition_rows),
            "duplicate occurrence_id in disposition ledger",
        )
        requirements = checker.load_jsonl(ROOT / "inventory" / "requirements.jsonl")
        requirement_ids = {
            (r.get("requirement_id") or r.get("id")) for r in requirements
        }
        commands = {r["key"]: r for r in checker.load_jsonl(HERE / "commands.jsonl")}
        slice_map = {
            r["key"]: r for r in checker.load_jsonl(HERE / "command-slice-map.jsonl")
        }
        anchors_doc = json.loads((HERE / "anchors.json").read_text())
        anchor_index = {}
        files = {
            ref.split("#", 1)[0]
            for row in crosswalk
            for ref in (row.get("design_refs") or [])
            if "#" in ref
        }
        for file_part in files:
            anchor_index[file_part] = checker.heading_slugs(ROOT / file_part)
        errors, unmapped = checker.core_errors(
            crosswalk,
            dispositions,
            requirement_ids,
            commands,
            slice_map,
            anchors_doc["pinnedCommit"],
            anchor_index,
        )
        self.assertEqual(errors, [])
        # Unmapped occurrences are explicit per class, and every unmapped
        # class is non-semantic or carries semantic refs (checked in core).
        self.assertEqual(
            sum(len(v) for v in unmapped.values()),
            len(disposition_rows)
            - len(
                {o for r in crosswalk for o in (r.get("source_occurrence_ids") or [])}
            ),
        )


if __name__ == "__main__":
    unittest.main()
