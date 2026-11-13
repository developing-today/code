#!/usr/bin/env python3
"""Finite reference-store and generation-key model; no timers/provider calls.

CLI contract: accepts no arguments and rejects extras before fixture file I/O.
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
assert fixture["format"] == "id-atlas-synthetic-workload/v3"


def support_level(op_id, engine):
    text = plan_ops[op_id][engine].lower().replace("-", " ")
    if op_id == "metadata_scan" and engine == "r2":
        return "unsupported" if "otherwise unsupported" in text else "supported"
    if op_id in ("ref_commit_if_live", "ref_remove") and engine == "r2":
        return "external-authority-required" if "external transactional authority" in text else "supported"
    if op_id == "object_tombstone_delete":
        if engine == "r2":
            return "external-authority-fence-required" if "external authoritative fence" in text else "unsupported"
        return "authority-fence-only" if "authority transaction only" in text else "unsupported"
    if op_id == "object_delete_captured_key":
        if engine == "r2":
            return ("plain-delete-captured-never-reused-key-after-fence"
                    if "plain deleteobject on captured never reused key" in text
                    else "unsupported")
        if engine == "d1":
            return "delete-captured-key-after-fence" if "delete captured generation row" in text else "unsupported"
        return ("authority-then-plain-delete-captured-key"
                if "authority fence then external plain delete" in text and "never reused" in text
                else "unsupported")
    return "unsupported" if "unsupported" in text else "supported"


for op_id, row in plan_ops.items():
    assert all(isinstance(row.get(engine), str) and row[engine] for engine in engines), op_id

rows = {}
physical = {}
ever_used_keys = set()
executed = []


def physical_key(content_id, generation):
    # Test-only key profile; the property tested is uniqueness/non-reuse, not
    # a production object-key encoding recommendation.
    short = content_id.split(":", 1)[1][:8]
    return f"fixture/sha256-{short}/gen-{generation}"


for step in fixture["operations"]:
    op_id, args = step["id"], step["input"]
    assert op_id in plan_ops, f"unmapped operation: {op_id}"
    actual_support = {engine: support_level(op_id, engine) for engine in engines}
    assert actual_support == step["engine_support"], {"operation": op_id, "support": actual_support}
    observed = {}

    if op_id == "blob_put_if_absent":
        raw = bytes.fromhex(args["bytes_hex"])
        content_id = "sha256:" + hashlib.sha256(raw).hexdigest()
        assert content_id == fixture["expected_content_id"], "fixture digest mismatch"
        prior = rows.get(content_id)
        if args.get("resurrection"):
            assert prior and prior["generations"][prior["current"]]["lifecycle"] == "Tombstoned"
            assert prior["bytes"] == raw, "resurrection must preserve canonical content identity"
            generation = prior["current"] + 1
            key = physical_key(content_id, generation)
            assert key not in ever_used_keys, "physical generation key reused"
            generation_row = {"lifecycle": "Live", "refs": set(), "row_revision": 1, "key": key}
            prior["generations"][generation] = generation_row
            prior["current"] = generation
            physical[key] = raw
            ever_used_keys.add(key)
            observed = {"accepted": True, "created": True, "generation": generation,
                        "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                        "content_id": content_id}
        elif prior:
            generation_row = prior["generations"][prior["current"]]
            if generation_row["lifecycle"] == "Tombstoned":
                observed = {"accepted": False, "created": False, "generation": prior["current"],
                            "lifecycle": "Tombstoned", "reason": "explicit-resurrection-required"}
            else:
                assert prior["bytes"] == raw and prior["metadata"] == args["metadata"]
                observed = {"accepted": True, "created": False, "generation": prior["current"],
                            "lifecycle": "Live", "reference_count": len(generation_row["refs"]),
                            "row_revision": generation_row["row_revision"]}
        else:
            generation = 1
            key = physical_key(content_id, generation)
            assert key not in ever_used_keys
            rows[content_id] = {"bytes": raw, "metadata": args["metadata"], "current": generation,
                                "generations": {generation: {"lifecycle": "Live", "refs": set(),
                                                             "row_revision": 1, "key": key}}}
            physical[key] = raw
            ever_used_keys.add(key)
            observed = {"accepted": True, "created": True, "generation": generation,
                        "physical_key": key, "lifecycle": "Live", "row_revision": 1,
                        "content_id": content_id}
        if "content_id" not in step["expected"]:
            observed.pop("content_id", None)

    elif op_id == "blob_get":
        row = rows[fixture["expected_content_id"]]
        generation = row["current"]
        gen = row["generations"][generation]
        observed = {"bytes_hex": physical[gen["key"]].hex(), "generation": generation,
                    "lifecycle": gen["lifecycle"]}

    elif op_id == "metadata_lookup":
        row = rows[fixture["expected_content_id"]]
        observed = {"metadata": row["metadata"], "generation": row["current"]}

    elif op_id == "metadata_scan":
        ids = sorted(cid for cid, row in rows.items()
                     if row["generations"][row["current"]]["lifecycle"] == "Live"
                     and cid.startswith(args["prefix"]) and fixture["tenant"] == "tenant-a")
        observed = {"content_ids": ids}

    elif op_id == "ref_commit_if_live":
        row = rows[fixture["expected_content_id"]]
        gen = row["generations"].get(args["generation"])
        if not gen or gen["lifecycle"] != "Live":
            observed = {"accepted": False, "reason": "generation-fenced"}
        else:
            gen["refs"].add(args["reference"])
            observed = {"accepted": True, "generation": args["generation"],
                        "reference_count": len(gen["refs"])}

    elif op_id == "ref_remove":
        row = rows[fixture["expected_content_id"]]
        gen = row["generations"][args["generation"]]
        removed = args["reference"] in gen["refs"]
        gen["refs"].discard(args["reference"])
        observed = {"removed": removed, "reference_count": len(gen["refs"])}

    elif op_id == "object_tombstone_delete":
        row = rows[fixture["expected_content_id"]]
        gen = row["generations"][args["generation"]]
        if gen["refs"]:
            observed = {"tombstoned": False, "reason": "references-present", "lifecycle": gen["lifecycle"]}
        else:
            gen["lifecycle"] = "Tombstoned"
            observed = {"tombstoned": True, "generation": args["generation"],
                        "physical_key": gen["key"], "lifecycle": "Tombstoned"}

    elif op_id == "object_delete_captured_key":
        row = rows[fixture["expected_content_id"]]
        generation = args["generation"]
        gen = row["generations"][generation]
        assert gen["lifecycle"] == "Tombstoned", "delete before authoritative fence"
        key = gen["key"]
        assert key in ever_used_keys and all(
            other["key"] != key for number, other in row["generations"].items() if number != generation
        ), "captured physical key aliases another generation"
        deleted = physical.pop(key, None) is not None
        current = row["current"]
        current_gen = row["generations"][current]
        observed = {"deleted_key": key, "deleted": deleted,
                    "generation_2_key_present": current == 2 and current_gen["key"] in physical,
                    "generation_2_reference_count": len(current_gen["refs"]),
                    "content_id_unchanged": fixture["expected_content_id"] == "sha256:" + hashlib.sha256(row["bytes"]).hexdigest()}

    else:
        raise AssertionError(f"no reference model for {op_id}")

    assert observed == step["expected"], {"operation": op_id, "expected": step["expected"], "observed": observed}
    executed.append(op_id)

print(json.dumps({
    "fixture": "synthetic-reference-store-v3",
    "timings_measured": False,
    "provider_calls": 0,
    "fixture_digest_verified": True,
    "row_preserving_put_replay": "matched",
    "ordinary_put_after_tombstone": "rejected without resurrection",
    "explicit_resurrection": "new generation key; canonical ContentId unchanged",
    "stale_g1_delete_after_g2_put": "g2 bytes and reference remain",
    "r2_conditional_delete_claim": "none; model uses captured non-reused old key only after fence",
    "reference_transitions_executed_and_matched": len(executed),
    "engine_operation_capability_mappings_checked": len(executed) * len(engines),
    "r2_metadata_scan": "explicitly unsupported",
}, indent=2, sort_keys=True))
