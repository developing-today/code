#!/usr/bin/env python3
"""Check that the committed many-worlds benchmark records latency components."""
from pathlib import Path

BASELINES = Path(__file__).resolve().parents[1] / "pkgs/id/benches/BASELINES.md"
PROFILE_FIELDS = (
    "wake_p50_ms", "wake_p95_ms", "wake_p99_ms",
    "input_ack_p50_ms", "input_ack_p95_ms", "input_ack_p99_ms",
)


def main() -> int:
    lines = [line for line in BASELINES.read_text().splitlines()
             if line.startswith("bench many_worlds ")]
    if not lines:
        raise SystemExit("no many_worlds measurement found in BASELINES.md")
    latest = lines[-1]
    fields = dict(token.split("=", 1) for token in latest.split()[2:] if "=" in token)
    missing = [name for name in PROFILE_FIELDS if name not in fields]
    if missing:
        raise SystemExit(f"latest many_worlds result lacks latency profile fields: {', '.join(missing)}")
    for prefix in ("wake", "input_ack"):
        values = [float(fields[f"{prefix}_p{percentile}_ms"]) for percentile in (50, 95, 99)]
        if values != sorted(values):
            raise SystemExit(f"{prefix} percentiles are not monotonic: {values}")
        if values[0] < 0:
            raise SystemExit(f"{prefix} latency is negative: {values}")
    if int(fields.get("commands", "0")) != 20_000:
        raise SystemExit("latest profile is not the full 20,000-command workload")
    print("many_worlds profile record valid: 20,000 commands; wake and input-ack p50/p95/p99 present")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
