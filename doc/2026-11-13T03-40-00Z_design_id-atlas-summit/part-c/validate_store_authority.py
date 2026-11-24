#!/usr/bin/env python3
"""Validator for the C9135-01 store authority research deliverables.

Issue: https://github.com/developing-today/code/issues/10167 (slice C9135-01)

Command line (the whole contract, parsed strictly before any file I/O
or test execution; anything else is a usage error, exit 2):

    validate_store_authority.py --input PATH [--check-test]

--input PATH    the store-placement-research-report.json to check.
--check-test    additionally invoke the owned Cargo integration test
                (pkgs/id/tests/store_authority_contract.rs) through
                ~/tools/fleet-build-lock, as the slice oracle specifies.

Stdlib only. Without --check-test this checks, against the live tree:

* Report identity: slice C9135-01, issue #10167, statements S2.7-S2.9,
  and the design-basis file it cites.
* Plane/source assertions: exactly the blob/event/world/sync
  authorities, each mapped to its owning crate; every named source
  (and evidence) path exists and still carries the mechanism markers
  the report claims for it.
* Profile assertions: identity profiles are pairwise distinct, and
  every forbidden equivalence in the report's non-conflation list
  pairs profiles whose (algorithm, input) facts actually differ --
  a shared algorithm name never makes two profiles interchangeable.
* redb inventory: both locked versions are recorded, and the world
  and sync surfaces are recorded as using no redb.
* Atlas producer status: producer_unavailable, null producer, and
  interop claims recorded as forbidden (the 9110-R2 record).

Exit code 0 iff every check passes.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RESEARCH_MD = Path(__file__).resolve().with_name("store-placement-research.md")
EXPECTED_SURFACES = {"blob", "event", "world", "sync"}
EXPECTED_OWNERS = {
    "blob": "id-store",
    "event": "id-events",
    "world": "id-worlds",
    "sync": "id-sync",
}

FAILURES = []


def check(ok, label, detail=""):
    if ok:
        print(f"ok   {label}")
    else:
        FAILURES.append(label)
        print(f"FAIL {label} {detail}")


def parse_args(argv):
    """Strict parse of the declared command line, before any file I/O."""
    input_path = None
    check_test = False
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--input" and input_path is None and i + 1 < len(argv):
            input_path = argv[i + 1]
            i += 2
        elif arg == "--check-test" and not check_test:
            check_test = True
            i += 1
        else:
            print(
                "usage: validate_store_authority.py --input PATH [--check-test]",
                file=sys.stderr,
            )
            raise SystemExit(2)
    if input_path is None:
        print(
            "usage: validate_store_authority.py --input PATH [--check-test]",
            file=sys.stderr,
        )
        raise SystemExit(2)
    return Path(input_path), check_test


def profile_facts(report):
    """Map profile id -> (algorithm, input) across authorities and Atlas."""
    facts = {}
    for entry in report["authorities"]:
        ident = entry["identity"]
        facts[ident["profile"]] = (ident["algorithm"], ident["input"])
    atlas = report["atlas_content_hash"]
    facts[atlas["profile"]] = (atlas["algorithm"], atlas["input"])
    return facts


def main():
    input_path, check_test = parse_args(sys.argv[1:])

    try:
        report = json.loads(input_path.read_text())
    except (OSError, json.JSONDecodeError) as err:
        check(False, f"report loads from {input_path}", f"({err})")
        return 1

    check(report.get("slice") == "C9135-01", "report slice is C9135-01")
    check(report.get("issue") == 10167, "report issue is 10167")
    check(
        report.get("statements") == ["S2.7", "S2.8", "S2.9"],
        "report statements are S2.7, S2.8, S2.9",
    )
    basis = ROOT / report["design_basis"]["path"]
    check(basis.exists(), "design basis file exists", f"({basis})")

    authorities = report["authorities"]
    surfaces = [entry["surface"] for entry in authorities]
    check(
        set(surfaces) == EXPECTED_SURFACES and len(surfaces) == len(EXPECTED_SURFACES),
        "report maps exactly the blob/event/world/sync authorities",
        f"(got {surfaces})",
    )
    for entry in authorities:
        surface = entry["surface"]
        check(
            entry["owner_crate"] == EXPECTED_OWNERS[surface],
            f"{surface} authority owner is {EXPECTED_OWNERS[surface]}",
            f"(got {entry['owner_crate']})",
        )
        source = ROOT / entry["source_path"]
        check(source.exists(), f"{surface} source path exists", f"({source})")
        if source.exists():
            text = source.read_text(errors="replace")
            for marker in entry["source_markers"]:
                check(
                    marker in text,
                    f"{surface} source carries marker {marker!r}",
                )
        if "evidence_path" in entry:
            evidence = ROOT / entry["evidence_path"]
            check(
                evidence.exists(),
                f"{surface} evidence path exists",
                f"({evidence})",
            )
            if evidence.exists():
                text = evidence.read_text(errors="replace")
                for marker in entry["evidence_markers"]:
                    check(
                        marker in text,
                        f"{surface} evidence carries marker {marker!r}",
                    )

    facts = profile_facts(report)
    check(
        len(facts) == len(authorities) + 1,
        "identity profiles are pairwise distinct",
        f"(got {sorted(facts)})",
    )
    for left, right in report["non_conflation"]["forbidden_equivalences"]:
        check(
            left in facts and right in facts and facts[left] != facts[right],
            f"forbidden equivalence {left} != {right} rests on differing facts",
            f"(left={facts.get(left)}, right={facts.get(right)})",
        )

    redb = report["redb_inventory"]
    check(
        set(redb["locked_versions"]) == {"4.6.0", "3.1.3"},
        "redb inventory records both locked versions 4.6.0 and 3.1.3",
        f"(got {redb['locked_versions']})",
    )
    check(
        "4.6.0" in redb["manifest_pin"],
        "redb manifest pin names 4.6.0",
    )
    for consumer in ("id-worlds", "id-sync"):
        modes = [u["mode"] for u in redb["usage"] if u["consumer"] == consumer]
        check(
            any("no redb" in mode for mode in modes),
            f"redb inventory records {consumer} as using no redb",
        )

    atlas = report["atlas_content_hash"]
    check(
        atlas["status"] == "producer_unavailable",
        "Atlas content_hash status is producer_unavailable",
    )
    check(atlas["producer"] is None, "Atlas content_hash producer is null")
    check(
        atlas["interop_claims"] == "forbidden",
        "Atlas content_hash interop claims are forbidden",
    )
    check(
        (ROOT / atlas["authority_record"]).exists(),
        "Atlas authority record (9110-R2) exists",
    )

    check(RESEARCH_MD.exists(), "research record exists beside the report")
    if RESEARCH_MD.exists():
        md = RESEARCH_MD.read_text(errors="replace")
        for entry in authorities:
            check(
                entry["identity"]["profile"] in md,
                f"research record names profile {entry['identity']['profile']}",
            )
            check(
                entry["owner_crate"] in md,
                f"research record names owner {entry['owner_crate']}",
            )

    if check_test and not FAILURES:
        lock = Path.home() / "tools" / "fleet-build-lock"
        check(lock.exists(), "fleet-build-lock exists for --check-test", f"({lock})")
        if lock.exists():
            cmd = [
                str(lock),
                "cargo",
                "test",
                "--manifest-path",
                str(ROOT / "pkgs/id/Cargo.toml"),
                "--test",
                "store_authority_contract",
            ]
            print(f"run  {' '.join(cmd)}")
            result = subprocess.run(cmd, cwd=ROOT)
            check(
                result.returncode == 0,
                "owned Cargo contract test passes via fleet-build-lock",
                f"(exit {result.returncode})",
            )

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed")
        return 1
    print("\nall checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
