#!/usr/bin/env python3
"""Decide whether a deployed pm-console-edge Worker answers as this commit.

Reads the Worker's /console/health JSON object from stdin and the wanted
full commit SHA from argv[1]. Prints the SHA when the Worker reports
``ok`` + ``edge`` + that exact version *and* its ``deployedAt`` /
``ageSeconds`` pair is truthful (the reported age is within 5 seconds of
``now - deployedAt`` and ``deployedAt`` is not in the future); prints an
empty line otherwise. Exit status is 0 either way, 2 when argv[1] is
missing, so a shell ``$(... || true)`` substitution stays empty on misuse.

Extracted from the "Check the deployed Worker answers" step of
.github/workflows/pm-console-edge-deploy.yml: embedding this predicate as
a multi-line ``python3 -c`` string broke the workflow's ``run: |`` literal
block (column-zero lines end the block scalar), which fails every run at
startup with zero jobs. Keeping it a separate file keeps the workflow
one line and gives the predicate a unit test.
"""

import datetime
import json
import sys


def check(health, want):
    """Return ``want`` when *health* answers as *want* with a truthful age."""
    if not (
        isinstance(health, dict)
        and health.get("ok")
        and health.get("edge")
        and health.get("version") == want
    ):
        return ""
    try:
        deployed = datetime.datetime.fromisoformat(
            str(health.get("deployedAt")).replace("Z", "+00:00")
        )
        age = int(health.get("ageSeconds"))
        now = datetime.datetime.now(datetime.timezone.utc)
        expected = int((now - deployed).total_seconds())
        if 0 <= age and abs(age - expected) <= 5 and deployed <= now:
            return want
        return ""
    except (ValueError, TypeError):
        return ""


def main(argv):
    if len(argv) < 2:
        print("")
        return 2
    try:
        health = json.load(sys.stdin)
    except ValueError:
        print("")
        return 0
    print(check(health, argv[1]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
