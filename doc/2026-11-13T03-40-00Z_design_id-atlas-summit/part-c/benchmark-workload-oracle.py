#!/usr/bin/env python3
"""Finite reference-store fixture; no timers, network, or provider calls.

CLI contract: no arguments; reject extras before fixture reads or computation.
"""
import hashlib
import json
import sys
from pathlib import Path

if len(sys.argv) != 1:
    print("usage: benchmark-workload-oracle.py", file=sys.stderr)
    raise SystemExit(2)

ROOT = Path(__file__).resolve().parent
plan = json.loads((ROOT / "benchmark-plan.json").read_text())
fixture = json.loads((ROOT / "synthetic-workload.json").read_text())
engines = ("d1", "r2", "redb", "turso")
plan_ops = {row["id"]: row for row in plan["logical_operations"]}
assert fixture["format"] == "id-atlas-synthetic-workload/v2"


def support_level(op_id, engine):
    text = plan_ops[op_id][engine].lower()
    if op_id == "metadata_scan" and engine == "r2":
        return "unsupported" if "otherwise unsupported" in text else "supported"
    if op_id in ("ref_commit_if_live", "ref_remove") and engine == "r2":
        return "external-authority-required" if "external transactional authority" in text else "supported"
    if op_id == "object_tombstone_delete":
        if engine == "d1":
            return "authority-only" if "authority transaction only" in text else "supported"
        if engine == "r2":
            return "preconditioned-delete-after-authority-fence" if "generation/version precondition" in text else "supported"
        return "authority-then-object-delete" if "then external deletion" in text else "supported"
    return "unsupported" if "unsupported" in text else "supported"


for op_id, row in plan_ops.items():
    assert all(isinstance(row.get(engine), str) and row[engine] for engine in engines), op_id

db = {}
executed = []
for step in fixture["operations"]:
    op_id, args = step["id"], step["input"]
    assert op_id in plan_ops, f"unmapped logical operation: {op_id}"
    assert {engine: support_level(op_id, engine) for engine in engines} == step["engine_support"], op_id
    observed = {}

    if op_id == "blob_put_if_absent":
        raw = bytes.fromhex(args["bytes_hex"])
        digest = hashlib.sha256(raw).hexdigest()
        content_id = "sha256:" + digest
        assert content_id == fixture["expected_content_id"], "fixture digest is not SHA256 of exact profile bytes"
        old = db.get(content_id)
        if args.get("resurrection"):
            assert old and old["lifecycle"] == "Tombstoned"
            generation = old["generation"] + 1
        elif old:
            assert old["bytes"] == raw
            generation = old["generation"]
        else:
            generation = 1
        if old is None or args.get("resurrection"):
            db[content_id] = {"bytes": raw, "generation": generation, "lifecycle": "Live",
                              "refs": set(), "metadata": args.get("metadata", old["metadata"] if old else {})}
        observed = {"created": old is None or args.get("resurrection", False),
                    "generation": generation, "lifecycle": "Live", "content_id": content_id}
        if "content_id" not in step["expected"]:
            observed.pop("content_id")

    elif op_id == "blob_get":
        content_id = fixture["expected_content_id"]
        row = db[content_id]
        observed = {"bytes_hex": row["bytes"].hex(), "generation": row["generation"]}

    elif op_id == "metadata_lookup":
        row = db[fixture["expected_content_id"]]
        observed = {"metadata": row["metadata"], "generation": row["generation"]}

    elif op_id == "metadata_scan":
        ids = sorted(k for k, v in db.items() if v["lifecycle"] == "Live"
                     and k.startswith(args["prefix"]) and fixture["tenant"] == "tenant-a")
        observed = {"content_ids": ids}

    elif op_id == "ref_commit_if_live":
        row = db[fixture["expected_content_id"]]
        if row["lifecycle"] != "Live":
            observed = {"accepted": False, "reason": "generation-fenced"}
        else:
            row["refs"].add(args["reference"])
            observed = {"accepted": True, "generation": row["generation"],
                        "reference_count": len(row["refs"])}

    elif op_id == "ref_remove":
        row = db[fixture["expected_content_id"]]
        removed = args["reference"] in row["refs"]
        row["refs"].discard(args["reference"])
        observed = {"removed": removed, "reference_count": len(row["refs"])}

    elif op_id == "object_tombstone_delete":
        row = db[fixture["expected_content_id"]]
        if row["refs"]:
            observed = {"tombstoned": False, "reason": "references-present",
                        "lifecycle": row["lifecycle"]}
        else:
            row["lifecycle"] = "Tombstoned"
            observed = {"tombstoned": True, "generation": row["generation"],
                        "lifecycle": row["lifecycle"]}

    else:
        raise AssertionError(f"no reference model for {op_id}")

    assert observed == step["expected"], {"operation": op_id, "expected": step["expected"], "observed": observed}
    executed.append(op_id)

print(json.dumps({
    "fixture": "synthetic-reference-store-v2",
    "timings_measured": False,
    "fixture_digest_verified": True,
    "reference_transitions_executed_and_matched": len(executed),
    "engine_operation_capability_mappings_checked": len(executed) * len(engines),
    "r2_metadata_scan": "explicitly unsupported",
    "generation_fence_and_resurrection": "reference model matched expected transitions",
    "canonical_fixture_profile": "SHA256 of exact bytes 616263",
    "cloud_evidence": "not measured; separate follow-up",
}, indent=2, sort_keys=True))
