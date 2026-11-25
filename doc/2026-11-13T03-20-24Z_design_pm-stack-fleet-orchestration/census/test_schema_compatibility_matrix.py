"""Offline CEN-04 document oracle; never reads a production store.

CLI: Python unittest's declared options, or exactly --list to list test IDs.
Argument parsing completes before document reads. No custom operator writes,
subprocesses, network calls or runtime/schema acceptance are provided.
"""

import copy
import hashlib
import json
from pathlib import Path
import re
import sys
import unittest


PACKAGE = Path(__file__).resolve().parents[1]
ROOT = PACKAGE.parents[1]
COMPONENT = PACKAGE / "components/9198-requirement-crosswalk-current-state-census.md"
SOURCE = ROOT / (
    "doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/"
    "2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility.md"
)
MARKER = "<!-- cen04-schema-mirror -->"
# §3's named facts, including the distinct absent/projection facts within rows.
FACT_SECTIONS = {
    "queue_identity": "3.1", "queue_state": "3.2", "urgency_profile": "3.3",
    "lease_claim": "3.4", "blocked_dependencies": "3.5", "broker_events": "3.6",
    "journal": "3.6", "checkpoints": "3.7", "notes": "3.7", "cursors": "3.8",
    "operation_receipts": "3.9", "creation_outbox": "3.9",
    "project_fields": "3.10", "metadata_block": "3.10", "metadata_mirror": "3.10",
    "merge_intent": "3.11", "placement_sessions": "3.12",
    "coordination_replica": "3.13", "fallback_markers": "3.13",
    "aggregator_actions": "3.14", "shared_comments": "3.14", "projector": "3.14",
    "pmp_links": "3.14", "pm_target": "3.14",
}


def sections(text):
    """Exact numbered section bodies; includes nested headings in parent bodies."""
    headings = list(re.finditer(r"^(#{2,3}) (\d+(?:\.\d+)?)\.? .+$", text, re.M))
    result = {}
    for index, heading in enumerate(headings):
        end = next((h.start() for h in headings[index + 1:]
                    if len(h[1]) <= len(heading[1])), len(text))
        if heading[2] in result:
            raise ValueError("duplicate source section")
        result[heading[2]] = text[heading.start():end].strip()
    return result


def normalized(text):
    return " ".join(re.sub(r"[`*_]", "", text).casefold().split())


def refusals(section):
    return [line.split("|")[1].strip() for line in section.splitlines()
            if line.startswith("| ") and " = " in line.split("|")[1]]


def load_mirror():
    text = COMPONENT.read_text()
    matches = re.findall(re.escape(MARKER) + r"\s*```json\n(.*?)\n```", text, re.S)
    if len(matches) != 1:
        raise ValueError("missing or duplicate CEN-04 census integration")
    return json.loads(matches[0])


def validate(mirror, source_text, persisted_facts=()):
    """Oracle: source section inventory/digests, quotes, refusals and admission rule.

    Digests cover the whole normative paragraphs, including APIs, evidence,
    rollback and gates; quotes bind structured owners/homes to those paragraphs.
    This validates documentation consistency, not live ownership or migration.
    """
    source = sections(source_text)
    required = {key for key in source if key.startswith("3.")} | {"4", "5", "6", "7", "9"}
    if set(mirror["sections"]) != required:
        raise ValueError("source section inventory drift")
    for key, digest in mirror["sections"].items():
        if hashlib.sha256(source[key].encode()).hexdigest() != digest:
            raise ValueError(f"source document drift in section {key}")
    if mirror["refusals"] != refusals(source["4"]):
        raise ValueError("equivalence refusal drift")
    if mirror["equivalences"]:
        raise ValueError("forbidden equivalence: no name-based mappings admitted")
    facts = {}
    covered = set()
    for row in mirror["facts"]:
        fact = row["fact"]
        if fact in facts or len(row["owners"]) != 1 or not row["owners"][0].strip():
            raise ValueError(f"duplicate owner or fact: {fact}")
        facts[fact] = row
        section = row["source"]
        if section not in required or not section.startswith("3."):
            raise ValueError("unregistered source section")
        covered.add(section)
        for quote in (row["owners"][0], row["home"], row["disposition"]):
            if not quote.strip() or normalized(quote) not in normalized(source[section]):
                raise ValueError(f"unsupported owner/home/disposition: {fact}")
        if row["disposition"] not in {"reuse", "extend", "absent", "conflict"}:
            raise ValueError("invalid disposition")
        for field in ("legacy_mapping", "write_read_api", "rollback"):
            if not row[field] or any(ref not in required for ref in row[field]):
                raise ValueError(f"missing normative {field}: {fact}")
        if row["legacy_mapping"] != [section] or row["write_read_api"] != [section]:
            raise ValueError("mapping/API must cite the fact's own row")
        if "7" not in row["rollback"]:
            raise ValueError("missing disable-only rollback")
        if row["disposition"] in {"absent", "conflict"} and not row["gates"]:
            raise ValueError("ungated unresolved fact")
        if any(ref not in required for ref in row["gates"]):
            raise ValueError("unregistered gate")
        if "muse-dev2" in row["home"]:
            raise ValueError("PM target conflict: muse-dev2 is not an admitted home")
    if covered != {key for key in required if key.startswith("3.")}:
        raise ValueError("uncovered source matrix row")
    if {fact: row["source"] for fact, row in facts.items()} != FACT_SECTIONS:
        raise ValueError("named fact inventory drift")
    if not set(persisted_facts) <= facts.keys():
        raise ValueError("unregistered persisted fact")


class SchemaCompatibilityMatrixTests(unittest.TestCase):
    def setUp(self):
        self.source = SOURCE.read_text()
        self.mirror = load_mirror()

    def test_schema_fact_has_one_owner_and_legacy_mapping(self):
        validate(self.mirror, self.source)

    def test_duplicate_owner_is_rejected(self):
        mutant = copy.deepcopy(self.mirror)
        mutant["facts"][0]["owners"].append("second writer")
        with self.assertRaisesRegex(ValueError, "duplicate owner"):
            validate(mutant, self.source)

    def test_duplicate_fact_is_rejected(self):
        mutant = copy.deepcopy(self.mirror)
        mutant["facts"].append(copy.deepcopy(mutant["facts"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate owner or fact"):
            validate(mutant, self.source)

    def test_unregistered_persisted_fact_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "unregistered persisted fact"):
            validate(self.mirror, self.source, ["unregistered.new_column"])

    def test_each_forbidden_equivalence_is_rejected(self):
        # Model-generated cases: every normative §4 refusal, not a sample pair.
        for pair in refusals(sections(self.source)["4"]):
            with self.subTest(pair=pair):
                mutant = copy.deepcopy(self.mirror)
                mutant["equivalences"].append(pair)
                with self.assertRaisesRegex(ValueError, "forbidden equivalence"):
                    validate(mutant, self.source)

    def test_source_document_drift_is_rejected(self):
        # Mutate each bound section independently, including per-row gates.
        for key, body in sections(self.source).items():
            if key in self.mirror["sections"]:
                with self.subTest(section=key):
                    changed = self.source.replace(body, body + "\nChanged contract.", 1)
                    with self.assertRaisesRegex(ValueError, "source document drift"):
                        validate(self.mirror, changed)

    def test_missing_source_row_is_rejected(self):
        mutant = copy.deepcopy(self.mirror)
        mutant["facts"] = [r for r in mutant["facts"] if r["source"] != "3.8"]
        with self.assertRaisesRegex(ValueError, "uncovered source matrix row"):
            validate(mutant, self.source)

    def test_missing_legacy_mapping_and_ungated_fact_are_rejected(self):
        for field in ("legacy_mapping", "gates"):
            with self.subTest(field=field):
                mutant = copy.deepcopy(self.mirror)
                row = next(r for r in mutant["facts"] if r["disposition"] == "absent")
                row[field] = []
                with self.assertRaisesRegex(ValueError, "missing normative|ungated"):
                    validate(mutant, self.source)

    def test_dead_pm_home_is_rejected(self):
        mutant = copy.deepcopy(self.mirror)
        next(r for r in mutant["facts"] if r["disposition"] == "conflict")["home"] = "muse-dev2"
        with self.assertRaisesRegex(ValueError, "PM target conflict"):
            validate(mutant, self.source)


if __name__ == "__main__":
    if sys.argv[1:] == ["--list"]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SchemaCompatibilityMatrixTests)
        print("\n".join(test.id() for test in suite))
    else:
        unittest.main()
