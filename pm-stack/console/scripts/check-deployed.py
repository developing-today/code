#!/usr/bin/env python3
"""Check the pm-console-edge /console/health answer against the deployed SHA.

Reads the health-check JSON from stdin, takes the expected full commit SHA
as argv[1], and prints the SHA when the Worker answers as that version with
a truthful deployed age (ageSeconds within 5s of now - deployedAt, and
deployedAt not in the future). Prints an empty string otherwise, so shell
callers can compare the output to the want value.

Extracted from .github/workflows/pm-console-edge-deploy.yml so the workflow
file stays valid YAML (a flush-left multiline python -c inside a run:| block
ends the block scalar and breaks parsing — that was the zero-job startup
failure of every pm-console-edge deploy run since 6b059084).
"""

from __future__ import annotations

import datetime
import json
import sys


def main() -> None:
    want = sys.argv[1] if len(sys.argv) > 1 else ""
    try:
        d = json.load(sys.stdin)
    except (ValueError, TypeError):
        print("")
        return
    if not (d.get("ok") and d.get("edge") and d.get("version") == want):
        print("")
        return
    try:
        deployed = datetime.datetime.fromisoformat(
            str(d.get("deployedAt")).replace("Z", "+00:00")
        )
        age = int(d.get("ageSeconds"))
        now = datetime.datetime.now(datetime.timezone.utc)
        expected = int((now - deployed).total_seconds())
        ok = 0 <= age and abs(age - expected) <= 5 and deployed <= now
        print(want if ok else "")
    except (ValueError, TypeError):
        print("")


if __name__ == "__main__":
    main()
