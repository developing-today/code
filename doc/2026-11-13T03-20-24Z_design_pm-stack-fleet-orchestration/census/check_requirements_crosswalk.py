#!/usr/bin/env python3
"""Read-only validation for the curated requirement crosswalk (CEN-01, #9500).

Usage: python3 -I check_requirements_crosswalk.py --check | --self-test
Validates the census crosswalk ledger against the inventory disposition
ledger: 389 semantic IDs occurring exactly once, every occurrence citation
resolving to the disposition ledger (file + line), group/alias edges,
the exact status vocabulary, evidence objects on verified/partial rows,
real design anchors, and the 215-key command map with no wildcard.
Occurrences no crosswalk row cites are emitted as explicit per-class
unmapped rows, never silently dropped. Semantic completeness is not
machine-provable; this checks structural integrity only.
"""

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

ALLOWED_STATUSES = {
    "verified existing",
    "partially implemented",
    "open implementation",
    "design-only",
    "research",
    "superseded with rationale",
    "blocked",
    "unverified",
}
EVIDENCE_REQUIRED_STATUSES = {"verified existing", "partially implemented"}
EVIDENCE_FIELDS = ("pin", "path", "symbol", "lines", "test", "timestamp")
MODALITIES = {"requirement", "candidate", "history", "example"}
SEMANTIC_DISPOSITIONS = {"requirement_link", "candidate_link", "history", "example"}
ID_PATTERN = re.compile(r"^[CHPOS]-R\d{3}$")
EXPECTED_REQUIREMENT_COUNT = 389
EXPECTED_COMMAND_COUNT = 215


def load_jsonl(path):
    rows = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path.name}:{number}: invalid JSON: {exc}") from exc
    return rows


def slugify(heading):
    text = heading.strip().lower()
    text = re.sub(r"[^\w\s-]", "", text)
    return re.sub(r"\s", "-", text)


def heading_slugs(path):
    slugs = set()
    for line in path.read_text().splitlines():
        if line.startswith("#"):
            slugs.add(slugify(line.lstrip("#")))
    return slugs


def core_errors(
    crosswalk,
    dispositions,
    requirement_ids,
    commands,
    slice_map,
    pinned_commit,
    anchor_index,
):
    """Validate crosswalk relationships without filesystem I/O.

    dispositions: {occurrence_id: disposition row}
    anchor_index: {design_ref_file: set of heading slugs}
    Returns (errors, unmapped) where unmapped is {disposition_class: [ids]}.
    """
    errors = []

    # 1. Exactly 389 semantic IDs, each occurring exactly once, matching the
    #    curated inventory requirement set.
    if len(crosswalk) != EXPECTED_REQUIREMENT_COUNT:
        errors.append(
            f"crosswalk has {len(crosswalk)} rows, expected {EXPECTED_REQUIREMENT_COUNT}"
        )
    seen = Counter()
    for row in crosswalk:
        rid = row.get("requirement_id")
        if not rid or not ID_PATTERN.match(str(rid)):
            errors.append(f"malformed requirement_id: {rid!r}")
            continue
        seen[rid] += 1
    for rid, count in sorted(seen.items()):
        if count != 1:
            errors.append(
                f"requirement {rid} occurs {count} times, expected exactly once"
            )
    crosswalk_ids = set(seen)
    if requirement_ids is not None and crosswalk_ids != set(requirement_ids):
        missing = sorted(set(requirement_ids) - crosswalk_ids)
        extra = sorted(crosswalk_ids - set(requirement_ids))
        if missing:
            errors.append(f"requirement IDs missing from crosswalk: {missing[:5]}")
        if extra:
            errors.append(f"crosswalk IDs not in inventory requirements: {extra[:5]}")

    # 2. Disposition ledger occurrence IDs are unique (a duplicated ledger
    #    row makes every citation to it ambiguous).
    #    (Uniqueness is checked by the caller building the dict; the raw
    #    count is compared there. Here citations resolve against the dict.)

    referenced = set()
    for row in crosswalk:
        rid = row.get("requirement_id")

        # 3. Exact status vocabulary; modality preserved, never promoted.
        status = row.get("status")
        if status not in ALLOWED_STATUSES:
            errors.append(f"{rid}: status {status!r} not in allowed vocabulary")
        modality = row.get("modality")
        if modality not in MODALITIES:
            errors.append(f"{rid}: modality {modality!r} not in allowed vocabulary")
        elif modality != "requirement" and not row.get("candidate_disposition"):
            errors.append(
                f"{rid}: {modality} modality without candidate_disposition "
                "(source modality must be preserved explicitly)"
            )

        # 4. Every occurrence citation resolves to the disposition ledger,
        #    with matching source file and line (span).
        occ_ids = row.get("source_occurrence_ids") or []
        refs = row.get("source_refs") or []
        ref_ids = [ref.get("occurrence_id") for ref in refs]
        if sorted(occ_ids) != sorted(ref_ids):
            errors.append(f"{rid}: source_occurrence_ids do not match source_refs")
        for occ in occ_ids:
            referenced.add(occ)
            if occ not in dispositions:
                errors.append(
                    f"{rid}: occurrence {occ} missing from disposition ledger"
                )
        for ref in refs:
            disp = dispositions.get(ref.get("occurrence_id"))
            if disp is None:
                continue
            if disp.get("source_file") != ref.get("source_file"):
                errors.append(
                    f"{rid}: occurrence {ref.get('occurrence_id')} source_file "
                    f"{ref.get('source_file')!r} != ledger {disp.get('source_file')!r}"
                )
            if (disp.get("locator") or {}).get("line") != ref.get("line"):
                errors.append(
                    f"{rid}: occurrence {ref.get('occurrence_id')} span line "
                    f"{ref.get('line')} != ledger line {(disp.get('locator') or {}).get('line')}"
                )

        # 5. Group/alias edges resolve; duplicate links are preserved.
        for child in row.get("child_requirement_ids") or []:
            if child not in crosswalk_ids:
                errors.append(f"{rid}: group child {child} not found in crosswalk")
        if row.get("is_group_node") and not (row.get("child_requirement_ids") or []):
            errors.append(f"{rid}: group node with no child_requirement_ids")
        dup = row.get("duplicate_of")
        if dup is not None:
            if dup == rid:
                errors.append(f"{rid}: duplicate_of points to itself")
            elif dup not in crosswalk_ids:
                errors.append(
                    f"{rid}: duplicate_of target {dup} not found in crosswalk"
                )

        # 6. Evidence object: verified/partial rows carry the full bounded
        #    evidence fields at the pinned commit; every row carries the
        #    explicit (possibly null-filled) evidence object.
        evidence = row.get("existing_evidence")
        if evidence is None:
            errors.append(f"{rid}: missing existing_evidence object")
        elif status in EVIDENCE_REQUIRED_STATUSES:
            for field in EVIDENCE_FIELDS:
                if not evidence.get(field):
                    errors.append(
                        f"{rid}: {status} row without source evidence field {field!r}"
                    )
            if evidence.get("pin") and evidence["pin"] != pinned_commit:
                errors.append(
                    f"{rid}: evidence pin {evidence['pin']!r} != census pin {pinned_commit!r}"
                )

        # 7. Design anchors are real: file known, heading slug present.
        for ref in row.get("design_refs") or []:
            if "#" not in ref:
                errors.append(f"{rid}: design ref {ref!r} has no anchor")
                continue
            file_part, anchor = ref.split("#", 1)
            slugs = anchor_index.get(file_part)
            if slugs is None:
                errors.append(f"{rid}: design ref file {file_part!r} not found")
            elif anchor not in slugs:
                errors.append(f"{rid}: unresolved design anchor {ref!r}")

        # 8. Command inventory refs resolve to real command keys whose
        #    slice assignment matches the command-slice map.
        for cref in row.get("command_inventory_refs") or []:
            key = cref.get("key") if isinstance(cref, dict) else cref
            if key not in commands:
                errors.append(f"{rid}: command ref {key!r} not in command inventory")
                continue
            mapped = slice_map.get(key)
            if mapped is not None and isinstance(cref, dict):
                if cref.get("primary_slice") != mapped.get("primary_slice"):
                    errors.append(
                        f"{rid}: command {key} primary_slice {cref.get('primary_slice')!r} "
                        f"!= slice map {mapped.get('primary_slice')!r}"
                    )

    # 9. Command map integrity: 215 keys on both sides, identical sets,
    #    and no wildcard anywhere in keys or slice assignments.
    if len(commands) != EXPECTED_COMMAND_COUNT:
        errors.append(
            f"command inventory has {len(commands)} keys, "
            f"expected {EXPECTED_COMMAND_COUNT}"
        )
    if set(commands) != set(slice_map):
        errors.append("command-slice-map keys differ from command inventory keys")
    for key, mapped in slice_map.items():
        values = [
            key,
            mapped.get("primary_slice") or "",
            *(mapped.get("supporting_slices") or []),
        ]
        if any("*" in str(v) for v in values):
            errors.append(f"wildcard in command mapping for {key!r}")

    # 10. Unmapped occurrences are explicit, per disposition class. A
    #     semantic occurrence with no semantic reference at all is a
    #     structural gap, not an unmapped row.
    unmapped = {}
    for occ, disp in sorted(dispositions.items()):
        if occ in referenced:
            continue
        unmapped.setdefault(disp.get("disposition"), []).append(occ)
        if disp.get("disposition") in SEMANTIC_DISPOSITIONS and not disp.get(
            "semantic_refs"
        ):
            errors.append(
                f"occurrence {occ}: semantic disposition "
                f"{disp.get('disposition')!r} unmapped and without semantic_refs"
            )
    return errors, unmapped


def run_check(root):
    census = root / "census"
    inventory = root / "inventory"
    crosswalk = load_jsonl(census / "requirements-crosswalk.jsonl")
    disposition_rows = load_jsonl(inventory / "dispositions.jsonl")
    dispositions = {}
    duplicate_occurrences = []
    for disp in disposition_rows:
        occ = disp.get("occurrence_id")
        if occ in dispositions:
            duplicate_occurrences.append(occ)
        dispositions[occ] = disp
    requirements = load_jsonl(inventory / "requirements.jsonl")
    requirement_ids = {(r.get("requirement_id") or r.get("id")) for r in requirements}
    command_rows = load_jsonl(census / "commands.jsonl")
    commands = {r["key"]: r for r in command_rows}
    slice_rows = load_jsonl(census / "command-slice-map.jsonl")
    slice_map = {r["key"]: r for r in slice_rows}
    anchors = json.loads((census / "anchors.json").read_text())
    pinned_commit = anchors.get("pinnedCommit")

    anchor_index = {}
    design_files = set()
    for row in crosswalk:
        for ref in row.get("design_refs") or []:
            if "#" in ref:
                design_files.add(ref.split("#", 1)[0])
    for file_part in sorted(design_files):
        path = root / file_part
        if path.exists():
            anchor_index[file_part] = heading_slugs(path)

    errors, unmapped = core_errors(
        crosswalk,
        dispositions,
        requirement_ids,
        commands,
        slice_map,
        pinned_commit,
        anchor_index,
    )
    for occ in duplicate_occurrences:
        errors.append(f"occurrence {occ} duplicated in disposition ledger")

    # The human ledger must state the same disposition count the machine
    # ledger carries; a stale count is a crosswalk integrity defect.
    md_text = (census / "requirements-crosswalk.md").read_text()
    stated = re.search(r"Source dispositions:\*\* ([\d,]+) occurrence rows", md_text)
    if stated is None:
        errors.append("requirements-crosswalk.md does not state a disposition count")
    elif int(stated.group(1).replace(",", "")) != len(disposition_rows):
        errors.append(
            f"requirements-crosswalk.md states {stated.group(1)} disposition rows, "
            f"ledger has {len(disposition_rows)}"
        )

    print(f"crosswalk rows: {len(crosswalk)}")
    print(f"disposition ledger rows: {len(disposition_rows)}")
    print(f"command keys: {len(commands)}")
    for cls in sorted(unmapped, key=str):
        ids = unmapped[cls]
        print(f"unmapped[{cls}]: {len(ids)} occurrence(s): {', '.join(ids)}")
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        print(f"FAIL: {len(errors)} validation error(s)")
        return 1
    print(
        "OK: crosswalk integrity validated "
        f"({len(crosswalk)} IDs, {len(disposition_rows)} dispositions, "
        f"{len(commands)} command keys)"
    )
    return 0


def run_self_test():
    """Synthetic negative/positive cases, mirroring the test module."""
    base_row = {
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
    dispositions = {
        "C-001": {
            "occurrence_id": "C-001",
            "source_file": "02_charter_v1_1.md",
            "locator": {"line": 1},
            "disposition": "requirement_link",
            "semantic_refs": ["C-R001"],
        }
    }
    commands = {f"cmd.{i}": {} for i in range(EXPECTED_COMMAND_COUNT)}
    slice_map = {
        k: {"primary_slice": "CLI-02", "supporting_slices": []} for k in commands
    }
    anchors = {"components/x.md": {"real-anchor"}}
    rows = [dict(base_row, requirement_id=f"C-R{i:03d}") for i in range(1, 390)]
    errors, _ = core_errors(
        rows,
        dispositions,
        {r["requirement_id"] for r in rows},
        commands,
        slice_map,
        "pin",
        anchors,
    )
    assert not errors, f"positive synthetic case failed: {errors[:3]}"
    bad = [dict(r) for r in rows]
    bad[0] = dict(
        bad[0],
        source_occurrence_ids=["C-999"],
        source_refs=[
            {
                "occurrence_id": "C-999",
                "source_file": "02_charter_v1_1.md",
                "line": 1,
                "quote": "x",
            }
        ],
    )
    errors, _ = core_errors(
        bad, dispositions, None, commands, slice_map, "pin", anchors
    )
    assert any("C-999" in e for e in errors), "missing occurrence not rejected"
    print("self-test OK")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--check", action="store_true", help="validate the live census files"
    )
    group.add_argument("--self-test", action="store_true", help="run synthetic cases")
    args = parser.parse_args(argv)
    if args.self_test:
        return run_self_test()
    root = Path(__file__).resolve().parent.parent
    return run_check(root)


if __name__ == "__main__":
    sys.exit(main())
