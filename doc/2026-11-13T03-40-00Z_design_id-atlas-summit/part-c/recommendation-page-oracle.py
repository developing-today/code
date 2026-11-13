#!/usr/bin/env python3
"""Tiny deterministic page identity/restart vector; no production state or crypto claim.

CLI contract: no arguments. Reject extras before fixture file I/O.
"""
import hashlib
import json
import sys
from pathlib import Path

if len(sys.argv) != 1:
    print("usage: recommendation-page-oracle.py", file=sys.stderr)
    raise SystemExit(2)

ROOT = Path(__file__).resolve().parent
fixture = json.loads((ROOT / "recommendation-page-fixture.json").read_text())


def cbor_head(major, value):
    if value < 24:
        return bytes([(major << 5) | value])
    if value < 256:
        return bytes([(major << 5) | 24, value])
    if value < 65536:
        return bytes([(major << 5) | 25]) + value.to_bytes(2, "big")
    if value < 2**32:
        return bytes([(major << 5) | 26]) + value.to_bytes(4, "big")
    return bytes([(major << 5) | 27]) + value.to_bytes(8, "big")


def deterministic_cbor(value):
    if isinstance(value, bool):
        return b"\xf5" if value else b"\xf4"
    if isinstance(value, int):
        return cbor_head(0, value) if value >= 0 else cbor_head(1, -1 - value)
    if isinstance(value, bytes):
        return cbor_head(2, len(value)) + value
    if isinstance(value, str):
        data = value.encode("utf-8")
        return cbor_head(3, len(data)) + data
    if isinstance(value, list):
        return cbor_head(4, len(value)) + b"".join(deterministic_cbor(item) for item in value)
    if isinstance(value, dict):
        entries = [(deterministic_cbor(key), deterministic_cbor(item)) for key, item in value.items()]
        entries.sort(key=lambda pair: (len(pair[0]), pair[0]))
        return cbor_head(5, len(entries)) + b"".join(k + v for k, v in entries)
    raise TypeError(f"unsupported fixture CBOR type: {type(value).__name__}")


def signed_envelope(body, signature_hex):
    return {"body": body, "signature": bytes.fromhex(signature_hex)}


def digest_id(value):
    return "sha256:" + hashlib.sha256(deterministic_cbor(value)).hexdigest()


parent_spec = fixture["parent_envelope"]
body = parent_spec["body"]
parent = signed_envelope(body, parent_spec["signature_hex"])
parent_id = digest_id(parent)
assert parent_id == fixture["expected_parent_snapshot_id"]
assert not any(key in body for key in ("pages", "page_envelope_ids", "page_ids")), "parent preimage must not include page IDs"

cutoff = body["cutoff_sequence"]
ordered = sorted(
    (row for row in body["candidate_set"] if row["arrival_sequence"] <= cutoff),
    key=lambda row: (-row["score_micros"], row["id"]),
)
late = fixture["post_cutoff_candidate"]
assert late["arrival_sequence"] > cutoff and late not in ordered


def make_page(position, signature_hex):
    if position < 0 or position > len(ordered) or position % body["page_size"]:
        raise ValueError("InvalidPagePosition")
    entries = ordered[position:position + body["page_size"]]
    page_body = {
        "parent_snapshot_id": parent_id,
        "position": position,
        "entries": entries,
        "done": position + len(entries) >= len(ordered),
    }
    envelope = signed_envelope(page_body, signature_hex)
    return envelope, digest_id(envelope)


def page_from_cursor(cursor, signature_hex):
    if cursor["parent_snapshot_id"] != parent_id:
        raise ValueError("WrongParentSnapshotId")
    return make_page(cursor["position"], signature_hex)


actual_pages = []
for expected in fixture["expected_pages"]:
    envelope, envelope_id = make_page(expected["position"], expected["signature_hex"])
    assert envelope_id == expected["expected_envelope_id"]
    assert [item["id"] for item in envelope["body"]["entries"]] == expected["expected_ids"]
    assert envelope["body"]["done"] is expected["done"]
    actual_pages.append({"envelope": envelope, "envelope_id": envelope_id})

first = fixture["expected_pages"][0]
cursor = {"parent_snapshot_id": parent_id, "position": first["position"]}
replayed, replayed_id = page_from_cursor(cursor, first["signature_hex"])
assert replayed_id == fixture["expected_restart_envelope_id"] == actual_pages[0]["envelope_id"]
assert replayed == actual_pages[0]["envelope"]

next_cursor = {"parent_snapshot_id": parent_id,
               "position": first["position"] + len(replayed["body"]["entries"])}
assert next_cursor == fixture["expected_next_cursor"]
wrong = fixture["wrong_parent_cursor"]
try:
    page_from_cursor(wrong, first["signature_hex"])
except ValueError as error:
    assert str(error) == wrong["expected_error"]
else:
    raise AssertionError("wrong-parent cursor was accepted")

print(json.dumps({
    "fixture": fixture["format"],
    "production_state_claim": False,
    "cryptographic_signature_verification": False,
    "parent_snapshot_id": parent_id,
    "page_envelope_ids": [page["envelope_id"] for page in actual_pages],
    "cursor_parent_and_position": "matched",
    "restart_same_page_envelope": "matched",
    "wrong_parent_cursor": "rejected",
    "post_cutoff_candidate": "excluded",
    "tie_order": "score DESC, immutable ID ASC",
}, indent=2, sort_keys=True))
