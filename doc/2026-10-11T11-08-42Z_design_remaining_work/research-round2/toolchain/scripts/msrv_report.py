import json
import sys
from functools import cmp_to_key

# Summarise the crates.io rust_version lookups: key crates, the highest rust_version in the union of both locks, and gaps.
results_path = sys.argv[1]
with open(results_path) as f:
    rows = json.load(f)

def parse(v):
    if not v:
        return None
    parts = v.split(".")
    try:
        return tuple(int(p) for p in parts + ["0"] * (3 - len(parts)))
    except ValueError:
        return None

TARGET = (1, 99, 0)
by_key = {(r["name"], r["version"]): r for r in rows}
KEY = [
    ("iroh", "1.3.0"), ("iroh-relay", "1.3.0"), ("iroh-blobs", "0.103.1"), ("iroh-tickets", "1.0.0"),
    ("iroh-docs", "0.101.0"), ("iroh-gossip", "0.101.0"), ("redb", "4.3.0"), ("redb", "3.1.3"),
    ("cbor-core", "0.10.1"), ("dcbor", "0.25.2"), ("ed25519-dalek", "2.2.0"), ("ed25519-dalek", "3.0.0"),
    ("libsql", "0.9.30"), ("tokio", "1.53.2"), ("distributed-topic-tracker", "0.3.5"),
]
print("KEY CRATES (crates.io rust_version, yanked):")
for name, ver in KEY:
    r = by_key.get((name, ver))
    if r is None:
        print(f"  {name} {ver}: NOT IN SCAN")
    else:
        print(f"  {name} {ver}: rust_version={r['rust_version']} yanked={r['yanked']} error={r['error']}")

vals = [(parse(r["rust_version"]), r) for r in rows if parse(r["rust_version"]) is not None]
unspecified = [r for r in rows if r["rust_version"] is None and r["error"] is None]
errors = [r for r in rows if r["error"] is not None]
yanked = [r for r in rows if r["yanked"]]
print(f"\nSCAN: pairs={len(rows)} with_rust_version={len(vals)} unspecified={len(unspecified)} errors={len(errors)} yanked={len(yanked)}")
mx = max(vals, key=lambda t: t[0])
print(f"HIGHEST rust_version in union: {mx[1]['name']} {mx[1]['version']} -> {mx[1]['rust_version']}")
above = [r for p, r in vals if p > TARGET]
print(f"crates needing > 1.99.0: {len(above)}")
for r in above:
    print(f"  {r['name']} {r['version']} needs {r['rust_version']}")
print("crates needing >= 1.96 (count):", sum(1 for p, _ in vals if p >= (1, 96, 0)))
print("yanked entries:", [f"{r['name']} {r['version']}" for r in yanked][:20])
print("error entries (first 10):", [f"{r['name']} {r['version']}: {r['error']}" for r in errors][:10])
print("unspecified rust_version (first 15):", [f"{r['name']} {r['version']}" for r in unspecified][:15])
