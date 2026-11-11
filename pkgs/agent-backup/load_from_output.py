#!/usr/bin/env python3
"""Feed a saved muse.db tool-output file into load_rows.py without the rows
passing through agent generation.

Usage: load_from_output.py <tool-output.json> <spec.json>
The tool output is the JSON the runtime saved for an oversized muse.db
result; rows are extracted from result.rows (or rows). The spec file is a
load_rows.py spec WITHOUT "rows" (rows are injected from the output file).
Prints load_rows.py's result; exit code propagates.
"""
import json
import subprocess
import sys
import os

LOADER = os.path.expanduser("~/tools/runtime-db-export/load_rows.py")


def extract_rows(path):
    with open(path) as f:
        data = json.load(f)
    if isinstance(data, dict):
        for key in ("result", "output", "data"):
            inner = data.get(key)
            if isinstance(inner, dict) and "rows" in inner:
                return inner["rows"]
        if "rows" in data:
            return data["rows"]
        # the saved file may be the raw exec envelope: search one level down
        for v in data.values():
            if isinstance(v, dict) and "rows" in v:
                return v["rows"]
            if isinstance(v, str):
                try:
                    inner = json.loads(v)
                except Exception:
                    continue
                if isinstance(inner, dict) and "rows" in inner:
                    return inner["rows"]
                if isinstance(inner, dict):
                    r = inner.get("result")
                    if isinstance(r, dict) and "rows" in r:
                        return r["rows"]
    raise SystemExit(f"no rows found in {path}")


def main():
    out_file, spec_file = sys.argv[1], sys.argv[2]
    rows = extract_rows(out_file)
    with open(spec_file) as f:
        spec = json.load(f)
    spec["rows"] = rows
    p = subprocess.run([sys.executable, LOADER], input=json.dumps(spec),
                       text=True)
    sys.exit(p.returncode)


if __name__ == "__main__":
    main()
