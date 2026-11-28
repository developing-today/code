#!/usr/bin/env python3
"""test_run_timeout.py — regression tests for #10940.

ghb_common.run() must bound every helper subprocess. A command that
outlives its limit is killed and comes back as a failed
CompletedProcess (returncode 124), so callers keep their ordinary
failure handling instead of hanging the run:

  quick command            -> output passes through unchanged
  command past its limit   -> rc 124 with a 'timed out' stderr, returns promptly
  default limit            -> applied when the caller passes none
  is_skeleton_mirror        -> a timed-out inspection is never a skeleton
  disk_kb                  -> -1 when du times out
  inventory_owner          -> a timed-out gh call is an owner-level error

Run: python3 infra/github-backup/tests/test_run_timeout.py
"""

import os, subprocess, sys, tempfile, time

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import ghb_common

failures = []


def check(name, ok):
    print(("PASS " if ok else "FAIL ") + name)
    if not ok:
        failures.append(name)


# quick command passes through
r = ghb_common.run([sys.executable, "-c", "print('hi')"])
check("quick: rc 0", r.returncode == 0)
check("quick: stdout", r.stdout.strip() == "hi")

# a hung command is killed at its limit
start = time.monotonic()
r = ghb_common.run([sys.executable, "-c", "import time; time.sleep(30)"], timeout=0.5)
elapsed = time.monotonic() - start
check("hung: rc 124", r.returncode == ghb_common.RUN_TIMEOUT_RETURNCODE == 124)
check("hung: stderr says timed out", "timed out after 0.5 seconds" in r.stderr)
check("hung: returns promptly", elapsed < 10)

# the default limit applies when the caller passes none
seen = {}
real_run = subprocess.run


def spy(cmd, **kw):
    seen["timeout"] = kw.get("timeout")
    return real_run(cmd, **kw)


subprocess.run = spy
try:
    ghb_common.run([sys.executable, "-c", "pass"])
finally:
    subprocess.run = real_run
check("default: timeout passed", seen.get("timeout") == ghb_common.RUN_TIMEOUT_SECONDS)


# callers read a timeout as an ordinary failure
def timed_out(cmd, **kw):
    return subprocess.CompletedProcess(
        cmd, ghb_common.RUN_TIMEOUT_RETURNCODE, stdout="", stderr="timed out"
    )


real_helper = ghb_common.run
ghb_common.run = timed_out
try:
    with tempfile.TemporaryDirectory() as d:
        check(
            "skeleton: timeout is not a skeleton",
            ghb_common.is_skeleton_mirror(d) is False,
        )
        check("valid_mirror: timeout is not valid", ghb_common.valid_mirror(d) is False)
        check("disk_kb: -1 on timeout", ghb_common.disk_kb(d) == -1)
    try:
        ghb_common.inventory_owner("someone")
        check("inventory: timeout raises an owner error", False)
    except RuntimeError as e:
        check(
            "inventory: timeout raises an owner error", "failed for someone" in str(e)
        )
finally:
    ghb_common.run = real_helper

if failures:
    print(f"\n{len(failures)} failure(s)")
    sys.exit(1)
print("\nall passed")
