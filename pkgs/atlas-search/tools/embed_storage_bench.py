#!/usr/bin/env python3
"""Run a deterministic, content-free Atlas hydration benchmark smoke test.

Usage: python3 -I pkgs/atlas-search/tools/embed_storage_bench.py
       [--sample-size N] [--runs N] [--cache cold|hot|both]

This deliberately uses an in-memory synthetic corpus and object store. It checks
hash/reference behavior and exercises the report shape; it is not a live Turso
or Cloudflare R2 measurement. No content is written to disk or printed.

Live venue contract (not exercised here): a future live run must supply a
designated read-only Turso credential and a disposable, non-production R2
test bucket/prefix purely through the environment, keep the report shape
identical (hashes, byte counts, operation counts, timings only), and clean
the test prefix afterwards. This harness never reads credentials itself and
never prints them; it must never be pointed at Atlas production or backup
buckets.
"""

import argparse
import hashlib
import json
import math
import sys
from time import perf_counter_ns


CLASS_A_USD_PER_MILLION = 4.50
CLASS_B_USD_PER_MILLION = 0.36


class HydrationError(RuntimeError):
    """A reference could not be safely hydrated; message never contains bytes."""


def verify_reference(expected_hash, fetch):
    """Return the verified digest, never the referenced body."""
    try:
        body = fetch(expected_hash)
    except Exception:
        raise HydrationError("reference object unavailable") from None
    if body is None:
        raise HydrationError("reference object unavailable")
    if not isinstance(body, bytes) or hashlib.sha256(body).hexdigest() != expected_hash:
        raise HydrationError("reference object hash mismatch")
    return expected_hash


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def synthetic_sample(size):
    """Create repeatable synthetic payloads with intentional duplicate content."""
    unique = max(1, math.ceil(size * 0.75))
    bodies = []
    for index in range(size):
        source = index % unique
        length = 256 + (source % 17) * 64
        seed = f"atlas-benchmark-fixture-v1:{source}:".encode("ascii")
        body = (seed * ((length // len(seed)) + 1))[:length]
        bodies.append(body)
    return bodies


def measure(bodies, runs, cache_mode):
    hashes = [hashlib.sha256(body).hexdigest() for body in bodies]
    source_bytes = sum(map(len, bodies))
    objects = {digest: body for digest, body in zip(hashes, bodies)}
    reference_bytes = sum(map(len, objects.values()))
    if any(verify_reference(digest, objects.get) != digest for digest in hashes):
        raise HydrationError("source/reference hash mismatch")

    variants = ["cold", "hot"] if cache_mode == "both" else [cache_mode]
    output = {}
    for operation, width in (("get-chunk", 1), ("expand", 5), ("with-message", 5)):
        output[operation] = {}
        for variant in variants:
            turso_times = []
            reference_times = []
            r2_gets = 0
            cache = {}
            # Repeat one stable logical query: the hot variant can demonstrate
            # cache hits, while cold deliberately bypasses the same cache.
            indices = [index % len(bodies) for index in range(width)]
            for _run in range(runs):
                before = perf_counter_ns()
                _ = [bodies[index] for index in indices]
                turso_times.append((perf_counter_ns() - before) / 1_000_000)

                before = perf_counter_ns()
                for index in indices:
                    digest = hashes[index]
                    if variant == "hot" and digest in cache:
                        body = cache[digest]
                    else:
                        body = objects.get(digest)
                        r2_gets += 1
                        if variant == "hot":
                            cache[digest] = body
                    verified = verify_reference(digest, lambda _key, value=body: value)
                    if verified != digest:
                        raise HydrationError("source/reference hash mismatch")
                reference_times.append((perf_counter_ns() - before) / 1_000_000)
            output[operation][variant] = {
                "turso_ms": {
                    "p50": round(percentile(turso_times, 0.50), 6),
                    "p95": round(percentile(turso_times, 0.95), 6),
                },
                "reference_ms": {
                    "p50": round(percentile(reference_times, 0.50), 6),
                    "p95": round(percentile(reference_times, 0.95), 6),
                },
                "r2_class_b_gets": r2_gets,
            }

    return {
        "source_bytes": source_bytes,
        "reference_bytes": reference_bytes,
        "unique_objects": len(objects),
        "dedup_ratio": round(source_bytes / reference_bytes, 6)
        if reference_bytes
        else 1.0,
        "r2_class_a_puts": len(objects),
        "hydration": output,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--sample-size", type=int, default=128)
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument("--cache", choices=("cold", "hot", "both"), default="both")
    args = parser.parse_args(argv)
    if not 1 <= args.sample_size <= 100_000:
        parser.error("--sample-size must be between 1 and 100000")
    if not 1 <= args.runs <= 100_000:
        parser.error("--runs must be between 1 and 100000")

    report = {
        "format": "atlas-embed-storage-bench/v1",
        "venue": "local in-memory synthetic harness",
        "sample_size": args.sample_size,
        "runs_per_operation_variant": args.runs,
        "cache": args.cache,
        "measurement_scope": "process-local operation latency; no network, database, or deployed R2 path",
        "r2_cost_rates_usd_per_million": {
            "class_a": CLASS_A_USD_PER_MILLION,
            "class_b": CLASS_B_USD_PER_MILLION,
        },
        "result": measure(synthetic_sample(args.sample_size), args.runs, args.cache),
        "limitations": [
            "synthetic bytes are not a representative Atlas corpus sample",
            "in-memory adapters do not measure Turso or Cloudflare R2 latency",
            "request costs are arithmetic estimates from the checked-in R2 rates, not billed usage",
        ],
    }
    report["result"]["r2_request_cost_usd"] = round(
        report["result"]["r2_class_a_puts"] * CLASS_A_USD_PER_MILLION / 1_000_000
        + sum(
            variant["r2_class_b_gets"]
            for operation in report["result"]["hydration"].values()
            for variant in operation.values()
        )
        * CLASS_B_USD_PER_MILLION
        / 1_000_000,
        12,
    )
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
