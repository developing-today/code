#!/usr/bin/env python3
"""Tip-hygiene census for one repo (code#3205).

The standing task in code#3205 re-runs a per-repo hygiene census every
quarter (or sooner when a trigger fires): tracked-blob count and total
bytes at the tip, the largest blobs, and any junk classes at the tip.
The 2026-10-31 census and its follow-up passes were manual `git
ls-tree` readings recorded in issue comments; this harness makes one
pass one command, with the classification stated once, here, instead
of re-derived per pass.

Junk classes (from code#3205: "build outputs, binaries, logs,
scratch/dump dirs, committed databases"). Every tracked blob lands in
exactly one class; first match wins, in this order:

- scratch:      any path component is exactly `scratch`, `dump`,
                `dumps` or `tmp`
- build-output: any path component is exactly `target`, or the
                extension is one of .a .class .o .obj .pyc .rlib
- database:     extension .db .sqlite .sqlite3
- log:          extension .log
- binary:       extension .bin .dll .dylib .exe .rh .so
- content:      everything else

A directory named `build` is deliberately NOT a junk signal: source
trees use it for modules (pkgs/roc/build in this repo is Roc source).
Build outputs are caught by the `target` component and by extension.

The census is observe-only: it prints one JSON document and writes
nothing. A junk finding is data for the quarterly pass, not a failure
of this tool; exit 0 means the census ran.

Argument contract (#3352): the whole command line is declared below
and parsed strictly before any subprocess or file read. Unknown flags,
stray positionals and valueless options are usage errors (exit 2);
exactly one of --fixture / --repo-dir must be given. `--opt=value`
works identically to `--opt value` (argparse semantics).

Usage:
    check_tip.py --repo-dir DIR [--ref REF] [--top N]
                 [--large-blob-mib MIB]
    check_tip.py --fixture JSON [--top N] [--large-blob-mib MIB]

`--fixture` replaces the git read with a JSON file shaped
{"entries": [{"path": ..., "size": ...}, ...]}, so the whole census
runs offline in tests. Live mode runs
`git -C DIR ls-tree -r -l REF` and counts blob entries only.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys

SCRATCH_COMPONENTS = {"scratch", "dump", "dumps", "tmp"}
BUILD_OUTPUT_COMPONENTS = {"target"}
BUILD_OUTPUT_EXTENSIONS = {".a", ".class", ".o", ".obj", ".pyc", ".rlib"}
DATABASE_EXTENSIONS = {".db", ".sqlite", ".sqlite3"}
LOG_EXTENSIONS = {".log"}
BINARY_EXTENSIONS = {".bin", ".dll", ".dylib", ".exe", ".rh", ".so"}

CLASS_ORDER = ["content", "build-output", "binary", "log", "database", "scratch"]
DEFAULT_TOP = 20
DEFAULT_LARGE_BLOB_MIB = 10.0


def classify(path: str) -> str:
    parts = path.split("/")
    components = parts[:-1]
    name = parts[-1]
    ext = "." + name.rsplit(".", 1)[1].lower() if "." in name else ""
    if any(c in SCRATCH_COMPONENTS for c in components):
        return "scratch"
    if any(c in BUILD_OUTPUT_COMPONENTS for c in components) or ext in BUILD_OUTPUT_EXTENSIONS:
        return "build-output"
    if ext in DATABASE_EXTENSIONS:
        return "database"
    if ext in LOG_EXTENSIONS:
        return "log"
    if ext in BINARY_EXTENSIONS:
        return "binary"
    return "content"


def census(entries: list[dict], top: int = DEFAULT_TOP,
           large_blob_mib: float = DEFAULT_LARGE_BLOB_MIB) -> dict:
    classes = {name: {"count": 0, "bytes": 0} for name in CLASS_ORDER}
    total_bytes = 0
    for entry in entries:
        cls = classify(entry["path"])
        classes[cls]["count"] += 1
        classes[cls]["bytes"] += entry["size"]
        total_bytes += entry["size"]
    junk_count = sum(
        c["count"] for name, c in classes.items() if name != "content"
    )
    junk_bytes = sum(
        c["bytes"] for name, c in classes.items() if name != "content"
    )
    threshold = int(large_blob_mib * 1024 * 1024)
    large = sorted(
        (e for e in entries if e["size"] >= threshold),
        key=lambda e: e["size"],
        reverse=True,
    )[:top]
    return {
        "blob_count": len(entries),
        "total_bytes": total_bytes,
        "classes": classes,
        "junk": {"count": junk_count, "bytes": junk_bytes},
        "large_blobs": [
            {"path": e["path"], "size": e["size"]} for e in large
        ],
    }


def entries_from_fixture(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)
    return [
        {"path": str(e["path"]), "size": int(e["size"])}
        for e in doc["entries"]
    ]


def entries_from_git(repo_dir: str, ref: str) -> list[dict]:
    out = subprocess.run(
        ["git", "-C", repo_dir, "ls-tree", "-r", "-l", ref],
        capture_output=True, text=True, check=True,
    ).stdout
    entries = []
    for line in out.splitlines():
        meta, _, path = line.partition("\t")
        fields = meta.split()
        if len(fields) < 4 or fields[1] != "blob" or fields[3] == "-":
            continue
        entries.append({"path": path, "size": int(fields[3])})
    return entries


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Tip-hygiene census for one repo (code#3205)."
    )
    parser.add_argument("--repo-dir", help="git checkout to census (live mode)")
    parser.add_argument("--ref", default="HEAD",
                        help="ref whose tip tree is censused (default HEAD)")
    parser.add_argument("--fixture",
                        help="JSON entry list replacing the git read (offline mode)")
    parser.add_argument("--top", type=int, default=DEFAULT_TOP,
                        help="max large blobs to report (default 20)")
    parser.add_argument("--large-blob-mib", type=float,
                        default=DEFAULT_LARGE_BLOB_MIB,
                        help="large-blob threshold in MiB (default 10)")
    args = parser.parse_args(argv)
    if bool(args.fixture) == bool(args.repo_dir):
        parser.error("exactly one of --fixture / --repo-dir is required")
    if args.fixture:
        entries = entries_from_fixture(args.fixture)
        source = {"fixture": args.fixture}
    else:
        entries = entries_from_git(args.repo_dir, args.ref)
        source = {"repo_dir": args.repo_dir, "ref": args.ref}
    report = census(entries, top=args.top, large_blob_mib=args.large_blob_mib)
    report["source"] = source
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
