"""Print per-unit durations from a cargo --timings HTML report (UNIT_DATA array)."""
import json
import re
import sys

html = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r"UNIT_DATA = (\[.*?\]);", html, re.S)
units = json.loads(m.group(1))
rows = []
for u in units:
    rows.append((u["duration"], u["name"], u.get("version", ""), u.get("mode", "")))
rows.sort(reverse=True)
total = sum(r[0] for r in rows)
print(f"units={len(rows)} sum_of_unit_durations_s={total:.2f}")
for d, name, version, mode in rows:
    print(f"{d:8.2f}s  {name} {version} [{mode}]")
