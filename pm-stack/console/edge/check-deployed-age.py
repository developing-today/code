"""Decide whether the deployed pm-console-edge Worker answers as this commit.

Reads one /console/health JSON document on stdin. Prints the wanted commit
SHA (argv[1]) when the Worker is healthy, on the edge, serving that version,
and reporting a truthful deployed age; prints an empty line otherwise. Always
exits 0 so callers can capture the answer with $(... || true).

This lives in its own file because the caller is a `run: |` block in
.github/workflows/pm-console-edge-deploy.yml: YAML literal-block content must
stay indented, while Python needs its statements at column 0. Inlining the
multi-line program there dedents it out of the block and breaks the workflow
file (zero-job startup failure). See #16176.
"""

import datetime
import json
import sys


def main() -> None:
    want = sys.argv[1] if len(sys.argv) > 1 else ""
    try:
        d = json.load(sys.stdin)
    except ValueError:
        print("")
        return
    if not (isinstance(d, dict) and d.get("ok") and d.get("edge") and d.get("version") == want):
        print("")
        return
    try:
        deployed = datetime.datetime.fromisoformat(str(d.get("deployedAt")).replace("Z", "+00:00"))
        age = int(d.get("ageSeconds"))
        now = datetime.datetime.now(datetime.timezone.utc)
        expected = int((now - deployed).total_seconds())
        print(want if 0 <= age and abs(age - expected) <= 5 and deployed <= now else "")
    except (ValueError, TypeError):
        print("")


if __name__ == "__main__":
    main()
