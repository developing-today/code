"""Check the synthetic Muse timeline against the hand-authored visible maps in fixture.json.

Usage: python3 check_fixture.py (no arguments; --help describes the command).
The fixture models already-validated, scope-ordered source events. Nothing is imported.
"""

import argparse
import json
from pathlib import Path


def apply_batch(rows, floors, batch):
    scope = batch["scope"]
    revision = batch["sequence"]
    complete = batch["coverage"] == "complete"
    objects = dict(batch["objects"])
    floor = floors.get(scope, -1)
    if complete:
        for row_scope, key in rows:
            if row_scope == scope:
                objects.setdefault(key, None)
    pending = {}
    for key, value in objects.items():
        address = (scope, key)
        previous_revision, previous_value = rows.get(address, (-1, None))
        effective_revision = max(previous_revision, floor)
        if revision < effective_revision:
            continue
        if revision == effective_revision:
            if value != previous_value:
                return "conflict"
            continue
        pending[address] = (revision, value)
    changed = bool(pending) or (complete and revision > floor)
    rows.update(pending)
    if complete:
        floors[scope] = max(floor, revision)
    return "applied" if changed else "unchanged"


def visible_at(rows, ceiling):
    return {
        f"{scope}/{key}": value[1]
        for (scope, key), (_, value) in rows.items()
        if value is not None and (ceiling == "restricted" or value[0] == "general")
    }


def check():
    fixture = json.loads(Path(__file__).with_name("fixture.json").read_text())
    rows, floors, receipts = {}, {}, set()
    for index, (delivery, verdict, state_key) in enumerate(fixture["steps"], start=1):
        batch = fixture["batches"][delivery]
        if delivery in receipts:
            result = "replayed"
        else:
            result = apply_batch(rows, floors, batch)
            if result != "conflict":
                receipts.add(delivery)
        state = fixture["states"][state_key]
        for ceiling in ("general", "restricted"):
            expected = dict(state["general"])
            if ceiling == "restricted":
                expected.update(state["restricted_extra"])
            if result != verdict or visible_at(rows, ceiling) != expected:
                raise ValueError(f"fixture step {index} ({delivery}), {ceiling} view")
    print(f"{len(fixture['steps'])} Muse fixture transitions matched both visibility oracles")


if __name__ == "__main__":
    argparse.ArgumentParser(description=__doc__).parse_args()
    check()
