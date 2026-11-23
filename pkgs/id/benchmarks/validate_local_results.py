#!/usr/bin/env python3
"""Validate a paired C9133 local benchmark result.

CLI contract: exactly one --input PATH option; arguments are parsed strictly
before opening files. The validator recomputes the embedded corpus digest and
checks redb/Turso parity, raw per-operation samples, and per-engine metadata.
"""

from __future__ import annotations

import base64
import hashlib
import json
import math
import sys
from pathlib import Path
from typing import Any

ENGINES = ("redb", "turso")
ENVIRONMENT_FIELDS = (
    "runner_version", "git_revision", "hardware_model", "cpu_count",
    "memory_bytes", "os_kernel", "runtime_versions", "start_time_utc",
    "cache_mode", "storage_shard_count", "dataset_digest",
)


class ValidationError(ValueError):
    pass


def parse_args(argv: list[str]) -> Path:
    if len(argv) != 2 or argv[0] != "--input" or not argv[1] or argv[1].startswith("--"):
        raise ValidationError("usage: validate_local_results.py --input PATH")
    return Path(argv[1])


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValidationError(message)


def validate_result(document: Any) -> None:
    require(isinstance(document, dict), "result must be an object")
    require(document.get("schema_version") == 1, "schema_version must be 1")
    require(document.get("format") == "id-atlas-local-result/v1", "format must be id-atlas-local-result/v1")
    require(document.get("comparison_scope") == "local-only", "local evidence must be marked local-only")
    try:
        corpus = base64.b64decode(document["corpus_bytes_base64"], validate=True)
    except (KeyError, TypeError, ValueError) as exc:
        raise ValidationError("corpus_bytes_base64 must contain the exact corpus bytes") from exc
    digest = "sha256:" + hashlib.sha256(corpus).hexdigest()
    require(document.get("dataset_digest") == digest, "dataset_digest does not match embedded corpus bytes")
    try:
        fixture_bytes = base64.b64decode(document["fixture_bytes_base64"], validate=True)
    except (KeyError, TypeError, ValueError) as exc:
        raise ValidationError("fixture_bytes_base64 must contain the exact workload fixture") from exc
    fixture_digest = "sha256:" + hashlib.sha256(fixture_bytes).hexdigest()
    require(document.get("fixture_digest") == fixture_digest, "fixture_digest does not match embedded fixture bytes")
    try:
        fixture = json.loads(fixture_bytes)
        fixture_operations = fixture["operations"]
        first_payload = next(
            bytes.fromhex(item["input"]["bytes_hex"])
            for item in fixture_operations
            if item["id"] == "blob_put_if_absent" and "bytes_hex" in item["input"]
        )
    except (KeyError, TypeError, ValueError, json.JSONDecodeError, StopIteration) as exc:
        raise ValidationError("embedded workload fixture is malformed") from exc
    require(corpus == first_payload, "corpus bytes do not match the embedded workload payload")
    require(fixture.get("expected_content_id") == digest,
            "fixture expected_content_id differs from independently computed corpus digest")

    runs = document.get("runs")
    require(isinstance(runs, list) and len(runs) == len(ENGINES), "runs must contain exactly redb and turso")
    by_engine = {run.get("engine"): run for run in runs if isinstance(run, dict)}
    require(set(by_engine) == set(ENGINES), "runs must contain one redb and one turso record")
    operation_samples = document.get("raw_operation_samples")
    require(isinstance(operation_samples, dict) and set(operation_samples) == set(ENGINES),
            "raw_operation_samples must contain both engines")

    expected_operations = document.get("operation_ids")
    require(isinstance(expected_operations, list) and expected_operations,
            "operation_ids must be a non-empty list")
    require(expected_operations == [item.get("id") for item in fixture_operations],
            "operation_ids differ from embedded fixture operations")
    for engine in ENGINES:
        run = by_engine[engine]
        require(run.get("schema_version") == 1, f"{engine}: schema_version must be 1")
        require(isinstance(run.get("workload_id"), str) and bool(run["workload_id"]),
                f"{engine}: workload_id must be non-empty")
        require(run.get("dataset_digest") == digest, f"{engine}: dataset digest differs from corpus")
        environment = run.get("environment")
        require(isinstance(environment, dict), f"{engine}: environment must be an object")
        require(all(field in environment for field in ENVIRONMENT_FIELDS),
                f"{engine}: environment is missing required fields")
        for field in ("runner_version", "git_revision", "hardware_model", "os_kernel", "start_time_utc"):
            require(isinstance(environment.get(field), str) and bool(environment[field]),
                    f"{engine}: environment.{field} must be non-empty")
        for field in ("cpu_count", "memory_bytes"):
            require(isinstance(environment.get(field), int) and environment[field] > 0,
                    f"{engine}: environment.{field} must be a positive integer")
        require(environment.get("dataset_digest") == digest, f"{engine}: environment digest differs")
        require(environment.get("cache_mode") in ("cold", "warm"), f"{engine}: invalid cache_mode")
        require(environment.get("storage_shard_count") in (1, 4, 16, 64),
                f"{engine}: invalid storage_shard_count")
        require(isinstance(environment.get("runtime_versions"), dict),
                f"{engine}: runtime_versions must be an object")
        metrics = run.get("metrics")
        require(isinstance(metrics, dict), f"{engine}: metrics must be an object")
        for key in ("p50_ms", "p95_ms", "p99_ms", "throughput_ops_s", "error_rate"):
            value = metrics.get(key)
            require(isinstance(value, (int, float)) and math.isfinite(value) and value >= 0,
                    f"{engine}: invalid metric {key}")
        require(metrics.get("error_rate", 1) <= 1, f"{engine}: error_rate must be at most 1")
        correctness = run.get("correctness")
        require(isinstance(correctness, dict) and correctness.get("fixture_digest_verified") is True,
                f"{engine}: fixture digest was not verified")
        require(correctness.get("provider_calls") == 0, f"{engine}: provider calls are not allowed")
        require(correctness.get("reference_transitions_executed_and_matched", 0) > 0,
                f"{engine}: no matched reference transitions recorded")
        require(correctness.get("engine_operation_capability_mappings_checked") == len(expected_operations),
                f"{engine}: capability mapping count differs from operation count")

        samples = operation_samples[engine]
        require(isinstance(samples, list), f"{engine}: operation samples must be a list")
        require([item.get("operation") for item in samples if isinstance(item, dict)] == expected_operations,
                f"{engine}: raw operation IDs differ from operation_ids")
        for item in samples:
            values = item.get("samples_ms")
            require(isinstance(values, list) and len(values) >= 5,
                    f"{engine}/{item.get('operation')}: expected at least five raw samples")
            require(all(isinstance(value, (int, float)) and math.isfinite(value) and value >= 0
                        for value in values), f"{engine}/{item.get('operation')}: invalid sample")
        flat = [value for item in samples for value in item["samples_ms"]]
        require(run.get("raw_samples") == flat, f"{engine}: raw_samples differ from operation samples")
        require(correctness["reference_transitions_executed_and_matched"] == len(flat),
                f"{engine}: matched transition count differs from raw sample count")


def main(argv: list[str]) -> int:
    try:
        input_path = parse_args(argv)
    except ValidationError as exc:
        print(f"usage error: {exc}", file=sys.stderr)
        return 2
    try:
        document = json.loads(input_path.read_text(encoding="utf-8"))
        validate_result(document)
    except (OSError, json.JSONDecodeError, ValidationError) as exc:
        print(f"invalid local result: {exc}", file=sys.stderr)
        return 1
    print("local result contains matched redb and Turso evidence with a verified corpus digest")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
