#!/usr/bin/env python3
"""Standalone validator for the 9110-R2 Atlas content_hash research deliverables.

Issue: https://github.com/developing-today/code/issues/10147 (slice 9110-R2)

Run:  python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/validate_atlas_content_hash_research.py

Stdlib only. Independently re-derives every checkable claim in
pkgs/atlas-search/test-vectors/content-hash-v1.json and in the research
record doc/.../crosscut/research-results/atlas-content-hash-producer.md:

* Chunk scope — the vector file must record the producer as
  unavailable (status, null producer/algorithm/profile/input bytes,
  zero known-answer vectors). Any future pin flips these fields and
  must extend this validator with a recomputation of the pinned
  producer's known-answer vector before an interop claim is made.
* Manifest scope (a different identity, recorded only to keep the
  two apart) — rebuilds each vector's canonical form from its
  fields per Manifest::canonical (crates/atlas-core/src/manifest.rs),
  compares byte-for-byte with the recorded canonical string, and
  recomputes SHA-256.
* Report — checks the research record carries the required findings
  (verdict, forbidden-claims statement, statement IDs, consumer
  inventory, producer-contract requirements).

Exit code 0 iff every check passes.
"""

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
VECTORS = ROOT / "pkgs/atlas-search/test-vectors/content-hash-v1.json"
REPORT = (
    ROOT
    / "doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/research-results/atlas-content-hash-producer.md"
)

FAILURES = []


def check(ok, label, detail=""):
    if ok:
        print(f"ok   {label}")
    else:
        FAILURES.append(label)
        print(f"FAIL {label} {detail}")


def canonical_form(vector):
    out = f"v{vector['version']}\n"
    for d in vector["dimensions"]:
        out += (
            f"{d['name']}|{d['shape']}|{d['semantics']}|{d.get('label_column') or ''}\n"
        )
    return out


def main():
    data = json.loads(VECTORS.read_text())

    chunk = data["chunk_content_hash"]
    check(
        chunk["status"] == "producer_unavailable",
        "chunk status is producer_unavailable",
    )
    check(chunk["producer"] is None, "chunk producer is null")
    check(chunk["algorithm"] is None, "chunk algorithm is null")
    check(chunk["profile"] is None, "chunk profile is null")
    check(chunk["input_bytes"] is None, "chunk input_bytes is null")
    check(chunk["vectors"] == [], "chunk known-answer vectors are empty")
    check(
        "forbidden" in chunk["interop_claims"],
        "chunk interop claims recorded as forbidden",
    )

    manifest = data["manifest_content_hash"]
    check(manifest["status"] == "pinned", "manifest status is pinned")
    check(len(manifest["vectors"]) >= 2, "manifest has at least two vectors")
    for i, vector in enumerate(manifest["vectors"]):
        rebuilt = canonical_form(vector)
        check(
            rebuilt == vector["canonical"],
            f"manifest vector {i} canonical form rebuilds byte-for-byte",
        )
        digest = hashlib.sha256(rebuilt.encode()).hexdigest()
        check(digest == vector["sha256"], f"manifest vector {i} sha256 recomputes")

    report = REPORT.read_text()
    for needle, label in [
        ("Producer unavailable", "report carries the verdict"),
        ("forbids interop claims", "report forbids interop claims"),
        ("S0.7", "report names statement S0.7"),
        ("S0.13", "report names statement S0.13"),
        ("S1.118", "report names statement S1.118"),
        ("local.rs", "report inventories the local.rs consumer"),
        ("chunks_raw", "report inventories the staging schema"),
        ("eval3", "report inventories the eval fingerprint consumer"),
        ("compute_hash", "report disambiguates the manifest producer"),
        ("What a future producer must pin", "report states the producer contract"),
    ]:
        check(needle in report, label)

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed")
        return 1
    print("\nall checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
