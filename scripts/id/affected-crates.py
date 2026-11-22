#!/usr/bin/env python3
"""Print pkgs/id workspace packages affected by a unified diff or name-only diff.

Usage:
  git diff --name-only origin/main...HEAD | python3 scripts/id/affected-crates.py -
  python3 scripts/id/affected-crates.py change.patch
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
ID_ROOT = PurePosixPath("pkgs/id")
GLOBAL_WORKSPACE_FILES = {
    ID_ROOT / "Cargo.toml",
    ID_ROOT / "Cargo.lock",
    ID_ROOT / ".cargo" / "config",
    ID_ROOT / ".cargo" / "config.toml",
}


def _strip_side(path: str) -> str | None:
    path = path.strip()
    if path == "/dev/null":
        return None
    if path.startswith(("a/", "b/")):
        path = path[2:]
    return path or None


def diff_paths(diff_text: str) -> list[str]:
    """Read paths from unified patch headers or git diff --name-only output."""
    paths: set[str] = set()
    saw_patch_path = False
    for line in diff_text.splitlines():
        if line.startswith(("--- ", "+++ ")):
            saw_patch_path = True
            parsed = _strip_side(line[4:].split("\t", 1)[0])
            if parsed is not None:
                paths.add(parsed)
        elif line.startswith("diff --git "):
            # Also catch binary diffs, which have no ---/+++ file headers.
            import shlex

            try:
                sides = shlex.split(line[len("diff --git ") :])
            except ValueError:
                continue
            if len(sides) == 2:
                saw_patch_path = True
                for side in sides:
                    parsed = _strip_side(side)
                    if parsed is not None:
                        paths.add(parsed)
    if saw_patch_path:
        return sorted(paths)
    return sorted({line.strip() for line in diff_text.splitlines() if line.strip()})


def _workspace_packages(metadata: dict[str, Any]) -> list[dict[str, Any]]:
    member_ids = set(metadata.get("workspace_members", []))
    return [p for p in metadata.get("packages", []) if p.get("id") in member_ids]


def select_affected(diff_file_paths: list[str], metadata: dict[str, Any]) -> list[str]:
    """Return changed workspace packages plus their transitive dependents."""
    packages = _workspace_packages(metadata)
    by_name = {p["name"]: p for p in packages}
    roots: dict[str, PurePosixPath] = {}
    for package in packages:
        manifest = Path(package["manifest_path"])
        try:
            relative = manifest.parent.resolve().relative_to(REPO_ROOT.resolve())
        except ValueError:
            # Cargo workspaces should be rooted in the repository. Ignore an
            # unexpected external package rather than claiming it is a member.
            continue
        roots[package["name"]] = PurePosixPath(relative.as_posix())

    changed: set[str] = set()
    all_names = set(by_name)
    reverse: dict[str, set[str]] = {name: set() for name in by_name}
    for package in packages:
        depender = package["name"]
        for dep in package.get("dependencies", []):
            dependency = dep.get("name")
            if dep.get("source") is None and dependency in by_name:
                reverse[dependency].add(depender)

    for raw_path in diff_file_paths:
        path = PurePosixPath(raw_path.removeprefix("./"))
        if path in GLOBAL_WORKSPACE_FILES:
            return sorted(all_names)
        # A change to workspace-level Cargo configuration can affect every
        # member even though it is not owned by a package directory.
        if path == ID_ROOT / ".cargo" or str(path).startswith(
            str(ID_ROOT / ".cargo") + "/"
        ):
            return sorted(all_names)
        if path != ID_ROOT and not str(path).startswith(str(ID_ROOT) + "/"):
            continue
        matches = [
            (len(str(root)), name)
            for name, root in roots.items()
            if path == root or str(path).startswith(str(root) + "/")
        ]
        if not matches:
            # A pkgs/id file outside a member directory belongs to the root
            # package; this is also the conservative fallback for new files.
            owner = "id" if "id" in by_name else None
        else:
            owner = max(matches)[1]
        if owner is not None:
            changed.add(owner)

    affected = set(changed)
    pending = list(changed)
    while pending:
        dependency = pending.pop()
        for depender in reverse.get(dependency, set()):
            if depender not in affected:
                affected.add(depender)
                pending.append(depender)
    return sorted(affected)


def load_metadata() -> dict[str, Any]:
    manifest = REPO_ROOT / "pkgs/id/Cargo.toml"
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(manifest),
        ],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or f"cargo metadata exited {result.returncode}"
        raise RuntimeError(detail)
    return json.loads(result.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "diff",
        nargs="?",
        default="-",
        help="unified patch or git diff --name-only text file; '-' reads stdin",
    )
    args = parser.parse_args(argv)

    try:
        if args.diff == "-":
            diff_text = sys.stdin.read()
        else:
            diff_text = Path(args.diff).read_text(encoding="utf-8")
        paths = diff_paths(diff_text)
        if not paths:
            return 0
        affected = select_affected(paths, load_metadata())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"affected-crates: {error}", file=sys.stderr)
        return 1

    for name in affected:
        print(name)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
