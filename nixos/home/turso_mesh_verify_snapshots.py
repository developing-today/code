#!/usr/bin/env python3
"""Read-only verifier for the R2, laptop, and Drive restic mesh repositories.

Command line contract: no arguments are accepted; ``--help`` is provided by
argparse. Repositories come from TURSO_MESH_R2_REPOSITORY,
TURSO_MESH_LAPTOP_REPOSITORY, and TURSO_MESH_DRIVE_REPOSITORY. All arguments
are parsed and all environment configuration is validated before restic runs.
The shared restic password and backend credentials are supplied by the caller.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys


class VerificationError(Exception):
    pass


REPOSITORY_VARIABLES = {
    "r2": "TURSO_MESH_R2_REPOSITORY",
    "laptop": "TURSO_MESH_LAPTOP_REPOSITORY",
    "drive": "TURSO_MESH_DRIVE_REPOSITORY",
}


def parse_arguments(argv):
    parser = argparse.ArgumentParser(
        description="Compare the full restic snapshot-ID sets in the backup mesh."
    )
    parser.allow_abbrev = False
    return parser.parse_args(argv)


def repositories_from_environment(environ):
    repositories = {}
    for label, variable in REPOSITORY_VARIABLES.items():
        value = environ.get(variable, "")
        if not value:
            raise VerificationError(
                f"required environment variable {variable} is empty"
            )
        repositories[label] = value
    if len(set(repositories.values())) != len(REPOSITORY_VARIABLES):
        raise VerificationError(
            "R2, laptop, and Drive must name three distinct repositories"
        )
    return repositories


def _snapshot_ids(restic, label, repository, runner):
    completed = runner(
        [restic, "snapshots", "--json", "--repo", repository],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise VerificationError(
            f"{label}: restic snapshots failed (exit {completed.returncode})"
        )
    try:
        snapshots = json.loads(completed.stdout)
    except (TypeError, json.JSONDecodeError) as exc:
        raise VerificationError(f"{label}: restic returned invalid JSON") from exc
    if not isinstance(snapshots, list):
        raise VerificationError(f"{label}: restic JSON must be a snapshot list")

    ids = set()
    for index, snapshot in enumerate(snapshots):
        snapshot_id = snapshot.get("id") if isinstance(snapshot, dict) else None
        if not isinstance(snapshot_id, str) or not snapshot_id:
            raise VerificationError(f"{label}: snapshot {index} has no non-empty id")
        if snapshot_id in ids:
            raise VerificationError(f"{label}: duplicate snapshot id at entry {index}")
        ids.add(snapshot_id)
    return ids


def verify_snapshot_sets(restic, repositories, runner=None):
    if runner is None:
        runner = subprocess.run
    expected_labels = set(REPOSITORY_VARIABLES)
    if set(repositories) != expected_labels:
        raise VerificationError(
            "repository map must contain exactly r2, laptop, and drive"
        )
    if any(not isinstance(value, str) or not value for value in repositories.values()):
        raise VerificationError("repository names must be non-empty strings")
    if len(set(repositories.values())) != len(REPOSITORY_VARIABLES):
        raise VerificationError(
            "R2, laptop, and Drive must name three distinct repositories"
        )

    snapshot_sets = {
        label: _snapshot_ids(restic, label, repositories[label], runner)
        for label in ("r2", "laptop", "drive")
    }
    source = snapshot_sets["r2"]
    divergence = {}
    for label in ("laptop", "drive"):
        target = snapshot_sets[label]
        missing = sorted(source - target)
        extra = sorted(target - source)
        if missing or extra:
            divergence[label] = {"missing": missing, "extra": extra}
    return {
        "status": "equal" if not divergence else "diverged",
        "source": "r2",
        "snapshots": len(source),
        "divergence": divergence,
    }


def main(
    argv=None, environ=None, runner=subprocess.run, out=sys.stdout, err=sys.stderr
):
    args = parse_arguments(sys.argv[1:] if argv is None else argv)
    del args  # argparse performs the strict no-argument check before any I/O.
    environment = os.environ if environ is None else environ
    try:
        repositories = repositories_from_environment(environment)
        restic = shutil.which("restic", path=environment.get("PATH"))
        if not restic:
            raise VerificationError("restic is not available on PATH")
        result = verify_snapshot_sets(restic, repositories, runner=runner)
    except VerificationError as exc:
        print(f"turso-mesh-verify-snapshots: FAILED {exc}", file=err)
        return 1

    print(json.dumps(result, sort_keys=True), file=out)
    return 0 if result["status"] == "equal" else 1


if __name__ == "__main__":
    raise SystemExit(main())
