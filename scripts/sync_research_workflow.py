#!/usr/bin/env python3
# Copyright developing.today LLC & contributors to the project.
# Licensed under the MIT and Apache-2.0 licenses in the repository root.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Copy the reviewed, portable Cairn workflow; keep repository profiles local."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


SKILLS = (
    "research-workflow",
    "request-intake",
    "research-prepare",
    "concept-research",
    "notes-reconcile",
    "issue-handoff",
    "design-orchestrate",
    "feature-plan",
    "implementation-orchestrate",
)
MANIFEST = ".agents/research-workflow.upstream.json"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inside(root, relative):
    if Path(relative).is_absolute() or ".." in Path(relative).parts:
        raise ValueError(f"path must be a repository-relative path: {relative}")
    candidate = root / relative
    resolved = candidate.resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ValueError(f"path escapes repository: {relative}")
    return candidate


def source_files(source, target):
    profile = json.loads((target / ".agents/research-workflow.json").read_text())
    source_profile = json.loads((source / ".agents/research-workflow.json").read_text())
    skills = source_profile.get("lifecycle_skills", SKILLS)
    mappings = [(Path(source_profile["workflow_docs"]), Path(profile["workflow_docs"]))]
    mappings += [
        (Path(".agents/skills") / name, Path(".agents/skills") / name)
        for name in skills
    ]
    result = {}
    for source_dir, target_dir in mappings:
        directory = inside(source, source_dir)
        if not directory.is_dir():
            raise ValueError(f"missing source directory: {source_dir}")
        for path in sorted(directory.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"source symlink needs review: {path}")
            if path.is_dir() or "__pycache__" in path.parts:
                continue
            destination = target_dir / path.relative_to(directory)
            inside(target, destination)
            result[destination.as_posix()] = (path, path.read_bytes())
    for relative in (
        "scripts/research_workflow.py",
        "scripts/tests/test_research_workflow.py",
    ):
        path = inside(source, relative)
        if not path.is_file():
            raise ValueError(f"missing portable tool: {relative}")
        result[relative] = (path, path.read_bytes())
    return result


def sync(source, target, write=False, require_clean=False):
    source, target = source.resolve(), target.resolve()
    if require_clean:
        dirty = subprocess.check_output(
            [
                "git",
                "-C",
                str(source),
                "status",
                "--porcelain",
                "--untracked-files=all",
            ],
            text=True,
        ).strip()
        if dirty:
            raise ValueError(
                "source checkout has uncommitted or untracked files; commit or clean it first:\n"
                + dirty
            )
    files = source_files(source, target)
    manifest_path = inside(target, MANIFEST)
    old = (
        json.loads(manifest_path.read_text())
        if manifest_path.exists()
        else {"files": {}}
    )
    issues, changed = [], []
    for relative, (_, content) in files.items():
        path = inside(target, relative)
        if path.exists():
            current = path.read_bytes()
            if current == content:
                continue
            if write and digest(current) != old["files"].get(relative):
                issues.append(f"local edits need reconciliation: {relative}")
        changed.append(relative)
    for relative in old["files"].keys() - files.keys():
        issues.append(f"upstream removed {relative}; review removal in a PR")
    if issues:
        raise ValueError("\n".join(issues))
    if not write:
        if changed or not manifest_path.exists():
            raise ValueError(
                "workflow mirror differs: " + ", ".join(changed or [MANIFEST])
            )
        for relative, (_, content) in files.items():
            if old["files"].get(relative) != digest(content):
                raise ValueError(f"provenance digest differs: {relative}")
        return len(files), 0
    # Validate the complete copy before creating or updating any destination.
    commit = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    manifest = {
        "version": 1,
        "source_repository": "dezren39/cairn",
        "source_commit": commit,
        "local_adapter": ".agents/research-workflow.json",
        "files": {
            relative: digest(data) for relative, (_, data) in sorted(files.items())
        },
    }
    for relative in changed:
        path = inside(target, relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(files[relative][1])
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    return len(files), len(changed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source", required=True, type=Path, help="local Cairn checkout"
    )
    parser.add_argument(
        "--repo-root", type=Path, default=Path(__file__).resolve().parents[1]
    )
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="allow syncing from uncommitted/dirty source",
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="compare without writing")
    mode.add_argument(
        "--write",
        action="store_true",
        help="copy reviewed changes, protecting local edits",
    )
    args = parser.parse_args()
    try:
        count, changed = sync(
            args.source, args.repo_root, args.write, require_clean=not args.allow_dirty
        )
        print(f"Workflow mirror: {count} files verified; {changed} updated")
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
