#!/usr/bin/env python3
"""Load one saved muse.db page via load_from_output.py and print the cursor
for the next page: {"count": n, "last": {key: value, ...}}.
Usage: page_load.py <output-file> <spec-file> <key1[,key2,...]>
"""
import json
import subprocess
import sys
import os

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from load_from_output import extract_rows


def main():
    out_file, spec_file, keys = sys.argv[1], sys.argv[2], sys.argv[3].split(",")
    rows = extract_rows(out_file)
    p = subprocess.run([sys.executable, os.path.join(HERE, "load_from_output.py"),
                        out_file, spec_file])
    if p.returncode != 0:
        sys.exit(p.returncode)
    last = {k: rows[-1].get(k) for k in keys} if rows else {}
    print(json.dumps({"count": len(rows), "last": last}))


if __name__ == "__main__":
    main()
