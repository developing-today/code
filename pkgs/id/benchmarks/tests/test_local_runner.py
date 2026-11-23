#!/usr/bin/env python3
"""Fixture-only tests for the paired local runner and result validator.

Oracle: malformed CLI input fails before work starts; a valid result is
accepted only when embedded corpus and fixture digests recompute and both
engines carry matching operation samples and required metadata.
"""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def require_rejected(call, label: str) -> None:
    try:
        call()
    except (ValueError, SystemExit):
        return
    raise AssertionError(f"oracle failed: {label} was accepted")


def fixture_result(validator):
    corpus = b"abc"
    digest = "sha256:" + hashlib.sha256(corpus).hexdigest()
    fixture = json.dumps({
        "expected_content_id": digest,
        "operations": [{"id": "blob_put_if_absent", "input": {"bytes_hex": "616263"}}],
    }, separators=(",", ":")).encode()
    env = {
        "runner_version": "test", "git_revision": "0" * 40,
        "hardware_model": "fixture", "cpu_count": 1, "memory_bytes": 1,
        "os_kernel": "fixture", "runtime_versions": {"python": "test"},
        "start_time_utc": "2026-11-23T00:00:00Z", "cache_mode": "warm",
        "storage_shard_count": 1, "dataset_digest": digest,
    }
    runs = []
    raw = {}
    for engine in ("redb", "turso"):
        runs.append({
            "schema_version": 1, "workload_id": "fixture", "engine": engine,
            "dataset_digest": digest,
            "environment": env, "metrics": {
                "p50_ms": 1.0, "p95_ms": 1.0, "p99_ms": 1.0,
                "throughput_ops_s": 1.0, "error_rate": 0.0,
            },
            "raw_samples": [1.0] * 5,
            "correctness": {
                "fixture_digest_verified": True,
                "provider_calls": 0,
                "reference_transitions_executed_and_matched": 5,
                "engine_operation_capability_mappings_checked": 1,
            },
        })
        raw[engine] = [{"operation": "blob_put_if_absent", "samples_ms": [1.0] * 5}]
    return {
        "format": "id-atlas-local-result/v1", "schema_version": 1,
        "comparison_scope": "local-only", "dataset_digest": digest,
        "corpus_bytes_base64": base64.b64encode(corpus).decode(),
        "fixture_digest": "sha256:" + hashlib.sha256(fixture).hexdigest(),
        "fixture_bytes_base64": base64.b64encode(fixture).decode(),
        "operation_ids": ["blob_put_if_absent"], "runs": runs, "raw_operation_samples": raw,
    }


def main(argv: list[str]) -> int:
    if argv != ["--fixture-only"]:
        print("usage: test_local_runner.py --fixture-only", file=sys.stderr)
        return 2
    runner = load_module("local_runner", HERE / "run_local.py")
    validator = load_module("local_validator", HERE / "validate_local_results.py")

    runner.parse_args([
        "--engines", "redb,turso", "--corpus", "corpus.bin",
        "--corpus-digest", "sha256:" + "a" * 64, "--result", "result.json",
    ])
    require_rejected(lambda: runner.parse_args(["--unknown"]), "unknown option")
    require_rejected(lambda: runner.parse_args([
        "--engines", "redb,turso", "--engines", "redb,turso",
        "--corpus", "c", "--corpus-digest", "sha256:" + "a" * 64,
        "--result", "r",
    ]), "duplicate option")
    require_rejected(lambda: runner.parse_args([
        "--engines", "redb,turso", "--corpus", "c",
        "--corpus-digest", "bad", "--result", "r",
    ]), "malformed digest")
    fixture = {"operations": [{"id": "blob_put_if_absent", "input": {"bytes_hex": "616263"}}]}
    require_rejected(lambda: runner.corpus_payload(b"other", fixture,
        "sha256:" + hashlib.sha256(b"other").hexdigest()), "corpus/workload mismatch")

    result = fixture_result(validator)
    validator.validate_result(result)
    corrupted = json.loads(json.dumps(result))
    corrupted["corpus_bytes_base64"] = base64.b64encode(b"tampered").decode()
    require_rejected(lambda: validator.validate_result(corrupted), "corpus digest mismatch")
    corrupted = json.loads(json.dumps(result))
    corrupted["runs"][1]["environment"]["dataset_digest"] = "sha256:" + "f" * 64
    require_rejected(lambda: validator.validate_result(corrupted), "engine digest mismatch")
    print("fixture-only local runner and validator oracles passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
