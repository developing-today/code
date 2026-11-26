#!/usr/bin/env python3
"""PM orchestration contract conformance report (CEN-07, #9437).

Reads the fixture manifest
(pkgs/fleet/crates/fleet-queue/tests/fixtures/pmo-conformance/manifest.json)
and the conformance venue's outputs, and says per family whether it is
landed and passing, landed and failing, or gated.

  check-list MANIFEST LIST_FILE
      LIST_FILE is `cargo test ... -- --list --format terse` output.
      Fails when a test a landed family or the harness names is not in
      the target: a filter that matches nothing is a failure, never a
      silent pass.

  summary MANIFEST RUST_LOG RUST_RC PROJECTOR_RC
      RUST_LOG is the `cargo test` output of the target; the two exit
      codes come from the Rust and projector steps. Writes a markdown
      table to $GITHUB_STEP_SUMMARY (stdout when unset) and fails when
      any landed family fails, a named test did not run, the harness
      test failed, or either step exited non-zero. Gated families are
      reported with their issue and never counted as passing or failing.
"""
import json
import os
import re
import sys

RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)\s*$")


def manifest(path):
    with open(path) as handle:
        return json.load(handle)


def named_tests(m):
    names = list(m.get("harness_tests", []))
    for family in m["families"]:
        if family["status"] == "landed":
            names.extend(family["tests"])
    return names


def check_list(manifest_path, list_path):
    m = manifest(manifest_path)
    with open(list_path) as handle:
        listed = {
            line[: -len(": test")]
            for line in handle.read().splitlines()
            if line.endswith(": test")
        }
    missing = [name for name in named_tests(m) if name not in listed]
    if not listed:
        print("FAIL: the conformance target lists no tests")
        return 1
    for name in missing:
        print(f"FAIL: manifest names {name}, which the target does not contain")
    if missing:
        return 1
    print(f"OK: all {len(named_tests(m))} named tests are in the target")
    return 0


def summary(manifest_path, log_path, rust_rc, projector_rc):
    m = manifest(manifest_path)
    results = {}
    with open(log_path, errors="replace") as handle:
        log = handle.read()
    for line in log.splitlines():
        match = RESULT.match(line.strip())
        if match:
            results[match.group(1)] = match.group(2)
    not_run_projector = "NOT RUN: projector source" in log

    rows = []
    failed = False
    for family in m["families"]:
        fid = family["id"]
        if family["status"] == "gated":
            rows.append((fid, "gated", f"waiting on #{family['gating_issue']}"))
            continue
        problems = []
        for test in family["tests"]:
            outcome = results.get(test)
            if outcome != "ok":
                problems.append(f"{test}: {outcome or 'did not run'}")
        if family.get("projector_tests"):
            if projector_rc != 0:
                problems.append(f"projector tests exited {projector_rc}")
            if not_run_projector:
                problems.append("projector source check did not run")
        if problems:
            failed = True
            rows.append((fid, "landed, failing", "; ".join(problems)))
        else:
            count = len(family["tests"])
            rows.append((fid, "landed, passing", f"{count} test{'' if count == 1 else 's'}"))

    harness = [t for t in m.get("harness_tests", []) if results.get(t) != "ok"]
    notes = []
    if harness:
        failed = True
        notes.append("Harness test not passing: " + ", ".join(harness))
    if rust_rc != 0:
        failed = True
        notes.append(f"The Rust conformance step exited {rust_rc}.")
    if projector_rc != 0:
        failed = True
        notes.append(f"The projector step exited {projector_rc}.")

    out = ["## PM orchestration contract conformance", "",
           "| Family | State | Detail |", "| --- | --- | --- |"]
    out += [f"| `{fid}` | {state} | {detail} |" for fid, state, detail in rows]
    out.append("")
    if failed:
        out.append("No family may be claimed as conforming from this run.")
    else:
        out.append("Every landed family passed. Gated families are not claimed.")
    out += [""] + notes
    text = "\n".join(out) + "\n"
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a") as handle:
            handle.write(text)
    print(text, end="")
    return 1 if failed else 0


def main(argv):
    if len(argv) == 3 and argv[0] == "check-list":
        return check_list(argv[1], argv[2])
    if len(argv) == 5 and argv[0] == "summary":
        return summary(argv[1], argv[2], int(argv[3]), int(argv[4]))
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
