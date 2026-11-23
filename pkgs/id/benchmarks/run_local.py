#!/usr/bin/env python3
"""Run the approved local redb/Turso fixture with matched corpus bytes.

CLI contract: --engines redb,turso --corpus PATH --corpus-digest sha256:HEX
--result PATH. The entire argv is validated, including duplicates and unknown
arguments, before file reads, subprocesses, temp files, or output writes.
Results are local evidence and must not be compared as cloud performance.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
FIXTURE = ROOT / "fixtures" / "synthetic-workload.json"
RUNNER_VERSION = "id-benchmarks-local/1"
REPETITIONS = 5
OPTIONS = {"--engines", "--corpus", "--corpus-digest", "--result"}


class UsageError(ValueError):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    values: dict[str, str] = {}
    index = 0
    while index < len(argv):
        token = argv[index]
        if token.startswith("--") and "=" in token:
            option, value = token.split("=", 1)
        else:
            option, value = token, None
        if option not in OPTIONS:
            raise UsageError(f"unknown argument: {token}")
        if option in values:
            raise UsageError(f"duplicate option: {option}")
        if value is None:
            index += 1
            if index >= len(argv) or argv[index].startswith("--"):
                raise UsageError(f"{option} requires a value")
            value = argv[index]
        if not value:
            raise UsageError(f"{option} requires a non-empty value")
        values[option] = value
        index += 1
    missing = OPTIONS - values.keys()
    if missing:
        raise UsageError("missing required option(s): " + ", ".join(sorted(missing)))
    if values["--engines"] != "redb,turso":
        raise UsageError("--engines must be exactly redb,turso")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", values["--corpus-digest"]):
        raise UsageError("--corpus-digest must be sha256: followed by 64 lowercase hex digits")
    return argparse.Namespace(
        engines=values["--engines"], corpus=Path(values["--corpus"]),
        corpus_digest=values["--corpus-digest"], result=Path(values["--result"]),
    )


def corpus_payload(corpus: bytes, fixture: dict[str, Any], declared_digest: str) -> str:
    actual = "sha256:" + hashlib.sha256(corpus).hexdigest()
    if actual != declared_digest:
        raise ValueError(f"corpus digest mismatch: declared {declared_digest}, computed {actual}")
    first_put = next(
        op for op in fixture["operations"]
        if op["id"] == "blob_put_if_absent" and "bytes_hex" in op["input"]
    )
    expected = bytes.fromhex(first_put["input"]["bytes_hex"])
    if corpus != expected:
        raise ValueError("corpus bytes must match the approved synthetic workload payload")
    return actual


RUST_HARNESS = r'''use id_benchmarks::{ReferenceStore, adapters::{redb::RedbStore, turso::TursoStore}, support_level};
use serde_json::{json, Value};
use std::{env, fs, time::Instant};

fn normalized(mut observed: Value, expected: &Value) -> Value {
    if expected.get("content_id").is_none() {
        observed.as_object_mut().expect("observed object").remove("content_id");
    }
    observed
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let fixture: Value = serde_json::from_slice(&fs::read(&args[1]).expect("read fixture")).expect("parse fixture");
    let plan: Value = serde_json::from_slice(&fs::read(&args[2]).expect("read plan")).expect("parse plan");
    let operations = fixture["operations"].as_array().expect("operations array");
    let plan_operations = plan["logical_operations"].as_array().expect("plan operations array");
    let mut result = serde_json::Map::new();
    let mut matched = serde_json::Map::new();
    let mut sync = serde_json::Map::new();
    for engine in ["redb", "turso"] {
        let mut samples = vec![Vec::<f64>::new(); operations.len()];
        let mut sync_samples = Vec::<Value>::new();
        let mut matched_count = 0usize;
        let mut capability_count = 0usize;
        for repetition in 0..5 {
            let mut reference = ReferenceStore::new();
            match engine {
                "redb" => {
                    let mut store = RedbStore::temp();
                    for (index, op) in operations.iter().enumerate() {
                        let id = op["id"].as_str().expect("operation id");
                        let cell = plan_operations.iter().find(|row| row["id"] == id).expect("plan operation")[engine].as_str().expect("plan engine cell");
                        assert_eq!(support_level(id, engine, cell), op["engine_support"][engine].as_str().expect("fixture capability"), "capability mapping at {id}");
                        if repetition == 0 { capability_count += 1; }
                        let args = &op["input"];
                        let expected = &op["expected"];
                        let reference_row = normalized(reference.execute(id, args, &fixture), expected);
                        assert_eq!(&reference_row, expected, "reference fixture oracle at {id}");
                        let start = Instant::now();
                        let actual = store.execute(id, args, &fixture);
                        samples[index].push(start.elapsed().as_secs_f64() * 1000.0);
                        assert_eq!(normalized(actual, expected), reference_row, "redb/reference mismatch at {id}");
                        matched_count += 1;
                    }
                }
                "turso" => {
                    let mut store = TursoStore::temp();
                    for (index, op) in operations.iter().enumerate() {
                        let id = op["id"].as_str().expect("operation id");
                        let cell = plan_operations.iter().find(|row| row["id"] == id).expect("plan operation")[engine].as_str().expect("plan engine cell");
                        assert_eq!(support_level(id, engine, cell), op["engine_support"][engine].as_str().expect("fixture capability"), "capability mapping at {id}");
                        if repetition == 0 { capability_count += 1; }
                        let args = &op["input"];
                        let expected = &op["expected"];
                        let reference_row = normalized(reference.execute(id, args, &fixture), expected);
                        assert_eq!(&reference_row, expected, "reference fixture oracle at {id}");
                        let start = Instant::now();
                        let actual = store.execute(id, args, &fixture);
                        samples[index].push(start.elapsed().as_secs_f64() * 1000.0);
                        assert_eq!(normalized(actual.transition, expected), reference_row, "Turso/reference mismatch at {id}");
                        sync_samples.push(json!({
                            "operation": id,
                            "local_revision": actual.sync.local_revision,
                            "pushed_revision": actual.sync.pushed_revision,
                            "pulled_revision": actual.sync.pulled_revision,
                            "pending_push_revisions": actual.sync.pending_push_revisions,
                            "network": actual.sync.network,
                        }));
                        matched_count += 1;
                    }
                }
                _ => unreachable!(),
            }
        }
        let per_operation: Vec<Value> = operations.iter().enumerate().map(|(i, op)| json!({
            "operation": op["id"], "samples_ms": samples[i]
        })).collect();
        result.insert(engine.to_string(), Value::Array(per_operation));
        if engine == "turso" {
            sync.insert(engine.to_string(), json!({
                "network": "local-only", "sync_samples": sync_samples
            }));
        } else {
            sync.insert(engine.to_string(), json!({"network": "local-only"}));
        }
        matched.insert(engine.to_string(), json!({
            "reference_transitions_executed_and_matched": matched_count,
            "engine_operation_capability_mappings_checked": capability_count / 5,
        }));
    }
    println!("{}", json!({"raw_operation_samples": result, "matched_transitions": matched, "engine_state": sync}));
}
'''


def read_memory_bytes() -> int:
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError):
        pass
    raise RuntimeError("could not determine total memory from /proc/meminfo")


def command_output(*args: str) -> str:
    completed = subprocess.run(args, check=True, capture_output=True, text=True)
    return completed.stdout.strip()


def environment(digest: str, started: str) -> dict[str, Any]:
    hardware = "unknown"
    for path in (Path("/sys/devices/virtual/dmi/id/product_name"), Path("/sys/firmware/devicetree/base/model")):
        try:
            candidate = path.read_text(encoding="utf-8").strip("\x00\n ")
            if candidate:
                hardware = candidate
                break
        except OSError:
            continue
    return {
        "runner_version": RUNNER_VERSION,
        "git_revision": command_output("git", "rev-parse", "HEAD"),
        "hardware_model": hardware,
        "cpu_count": os.cpu_count() or 1,
        "memory_bytes": read_memory_bytes(),
        "os_kernel": platform.platform(),
        "runtime_versions": {
            "python": platform.python_version(),
            "rustc": command_output("rustc", "--version"),
            "cargo": command_output("cargo", "--version"),
        },
        "start_time_utc": started,
        "cache_mode": "cold",
        "storage_shard_count": 1,
        "dataset_digest": digest,
    }


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[max(0, min(len(ordered) - 1, int((len(ordered) - 1) * fraction)))]


def build_result(corpus: bytes, digest: str, fixture: dict[str, Any], fixture_bytes: bytes,
                 harness_result: dict[str, Any], started: str) -> dict[str, Any]:
    env = environment(digest, started)
    per_engine = harness_result["raw_operation_samples"]
    runs = []
    operation_ids = [op["id"] for op in fixture["operations"]]
    for engine in ("redb", "turso"):
        operation_rows = per_engine[engine]
        flat = [sample for row in operation_rows for sample in row["samples_ms"]]
        total_ms = sum(flat)
        runs.append({
            "schema_version": 1,
            "workload_id": "C9133-LOCAL-synthetic-workload-v3",
            "engine": engine,
            "dataset_digest": digest,
            "environment": dict(env),
            "metrics": {
                "p50_ms": percentile(flat, 0.50),
                "p95_ms": percentile(flat, 0.95),
                "p99_ms": percentile(flat, 0.99),
                "throughput_ops_s": (len(flat) * 1000.0 / total_ms) if total_ms else 0.0,
                "error_rate": 0.0,
            },
            "raw_samples": flat,
            "correctness": {
                "fixture_digest_verified": True,
                "reference_transitions_executed_and_matched": harness_result["matched_transitions"][engine]["reference_transitions_executed_and_matched"],
                "engine_operation_capability_mappings_checked": harness_result["matched_transitions"][engine]["engine_operation_capability_mappings_checked"],
                "timings_measured": True,
                "provider_calls": 0,
            },
        })
    return {
        "format": "id-atlas-local-result/v1",
        "schema_version": 1,
        "workload_id": "C9133-LOCAL-synthetic-workload-v3",
        "comparison_scope": "local-only",
        "notice": "These local measurements are not cloud performance comparisons or shard recommendations.",
        "dataset_digest": digest,
        "corpus_bytes_base64": base64.b64encode(corpus).decode("ascii"),
        "fixture_digest": "sha256:" + hashlib.sha256(fixture_bytes).hexdigest(),
        "fixture_bytes_base64": base64.b64encode(fixture_bytes).decode("ascii"),
        "operation_ids": operation_ids,
        "runs": runs,
        "raw_operation_samples": per_engine,
        "engine_state": harness_result["engine_state"],
    }


def run(argv: list[str]) -> int:
    try:
        args = parse_args(argv)
    except UsageError as exc:
        print(f"usage error: {exc}", file=sys.stderr)
        return 2
    try:
        corpus = args.corpus.read_bytes()
        fixture_bytes = FIXTURE.read_bytes()
        fixture = json.loads(fixture_bytes)
        digest = corpus_payload(corpus, fixture, args.corpus_digest)
        started = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        with tempfile.TemporaryDirectory(prefix="id-benchmarks-local-") as temp_name:
            temp = Path(temp_name)
            (temp / "src").mkdir()
            manifest = "\n".join([
                "[package]", 'name = "id-benchmarks-local-runner"', 'version = "0.1.0"',
                'edition = "2021"', "", "[workspace]", "", "[dependencies]",
                f"id-benchmarks = {{ path = {json.dumps(str(ROOT))} }}",
                'serde_json = "1.0"', "",
            ])
            (temp / "Cargo.toml").write_text(manifest, encoding="utf-8")
            (temp / "src" / "main.rs").write_text(RUST_HARNESS, encoding="utf-8")
            cargo = [
                "cargo", "run", "--quiet", "--manifest-path", str(temp / "Cargo.toml"),
                "--", str(FIXTURE),
            ]
            broker = shutil.which("cargo-target-broker")
            home_broker = Path.home() / "tools" / "cargo-target-broker"
            if broker is None and home_broker.is_file() and os.access(home_broker, os.X_OK):
                broker = str(home_broker)
            if broker:
                repo_root = command_output("git", "rev-parse", "--show-toplevel")
                cargo = [
                    broker, "exec", "--repo", repo_root,
                    "--profile", "test", "--", *cargo,
                ]
            else:
                repo_root = command_output("git", "rev-parse", "--show-toplevel")
            plan_path = Path(repo_root) / "doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/benchmark-plan.json"
            cargo.extend([str(plan_path)])
            completed = subprocess.run(
                cargo,
                check=True, capture_output=True, text=True,
            )
        harness_result = json.loads(completed.stdout)
        result = build_result(corpus, digest, fixture, fixture_bytes, harness_result, started)
        args.result.parent.mkdir(parents=True, exist_ok=True)
        tmp_output = args.result.with_name(args.result.name + ".tmp")
        tmp_output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        tmp_output.replace(args.result)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, json.JSONDecodeError, RuntimeError) as exc:
        detail = getattr(exc, "stderr", None) or str(exc)
        print(f"local benchmark failed: {detail}", file=sys.stderr)
        return 1
    print(f"wrote matched redb/Turso local evidence: {args.result} ({digest})")
    return 0


if __name__ == "__main__":
    raise SystemExit(run(sys.argv[1:]))
