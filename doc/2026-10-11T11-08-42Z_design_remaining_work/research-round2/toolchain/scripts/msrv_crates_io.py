import json
import sys
import time
import urllib.error
import urllib.request

# Look up rust_version and yanked for each (name, version) pair in the lock JSON files, one request per second.
# Input: JSON lists written by lock_packages.py. Output: JSON list of results written to the path in argv[1].
out_path = sys.argv[1]
in_paths = sys.argv[2:]
pairs = set()
for path in in_paths:
    with open(path) as f:
        for p in json.load(f):
            if p["source"].startswith("registry+"):
                pairs.add((p["name"], p["version"]))
results = []
UA = "research-round2-toolchain-msrv-check (read-only metadata lookup)"
for name, version in sorted(pairs):
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    rec = {"name": name, "version": version, "rust_version": None, "yanked": None, "error": None}
    for attempt in range(3):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=60) as resp:
                body = json.load(resp)
            rec["rust_version"] = body["version"].get("rust_version")
            rec["yanked"] = body["version"].get("yanked")
            break
        except urllib.error.HTTPError as e:
            rec["error"] = f"HTTP {e.code}"
            if e.code == 429:
                time.sleep(10)
                continue
            break
        except Exception as e:  # network or parse problem: record and move on
            rec["error"] = f"{type(e).__name__}: {e}"
            time.sleep(3)
    results.append(rec)
    time.sleep(1.0)
    if len(results) % 100 == 0:
        print(f"progress {len(results)}/{len(pairs)}", flush=True)
with open(out_path, "w") as f:
    json.dump(results, f, indent=1)
print(f"done {len(results)} pairs -> {out_path}", flush=True)
