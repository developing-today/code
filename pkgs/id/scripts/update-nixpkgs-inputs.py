#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""update-nixpkgs-inputs.py

Discover NixOS/nixpkgs inputs from flake.lock and update them.
Parses flake.lock directly to find inputs by source URL (github:NixOS/nixpkgs).

Usage:
    ./scripts/update-nixpkgs-inputs.py [ref...]

Arguments:
    ref  - optional branch filter(s): "master", "nixos-unstable", or empty for all
           multiple refs can be passed to produce a combined summary

Examples:
    ./scripts/update-nixpkgs-inputs.py                         # all NixOS/nixpkgs inputs
    ./scripts/update-nixpkgs-inputs.py master                  # master (or no ref) inputs
    ./scripts/update-nixpkgs-inputs.py nixos-unstable           # nixos-unstable inputs
    ./scripts/update-nixpkgs-inputs.py master nixos-unstable    # both categories, combined summary
"""

import json
import re
import subprocess
import sys
from pathlib import Path


def load_flake_lock(repo_dir):
    """Load and parse flake.lock."""
    with open(repo_dir / "flake.lock") as f:
        return json.load(f)


def get_input_info(lock_data, input_name):
    """Extract rev, source URL, and ref for an input from parsed flake.lock.

    Returns: (short_rev, source_url, ref_display)
    """
    root_inputs = lock_data["nodes"]["root"]["inputs"]
    node_name = root_inputs.get(input_name)

    if not isinstance(node_name, str):
        return ("unknown", "unknown", "unknown")

    node = lock_data["nodes"][node_name]
    original = node.get("original", {})
    locked = node.get("locked", {})

    type_ = (original.get("type") or locked.get("type") or "").lower()
    full_rev = locked.get("rev", "unknown")
    rev = full_rev[:12] if len(full_rev) > 12 else full_rev
    branch = original.get("ref")
    ref_display = branch if branch else "(default)"

    if type_ == "github":
        owner = original.get("owner") or locked.get("owner") or "?"
        repo = original.get("repo") or locked.get("repo") or "?"
        slug = f"{owner}/{repo}"
        url = f"github:{slug}/{branch}" if branch else f"github:{slug}"
    elif type_ == "tarball":
        url = original.get("url") or locked.get("url") or "?"
    else:
        url = f"{type_}:{json.dumps(original or locked)}"

    return (rev, url, ref_display)


def discover_inputs(lock_data, ref):
    """Discover NixOS/nixpkgs inputs by parsing flake.lock directly.

    Walks root inputs, skips follows (array values), and matches by URL:
    original.type == "github", original.owner ~= NixOS (case-insensitive),
    original.repo == "nixpkgs".

    Filters by original.ref when ref is specified:
      ref=None  → all NixOS/nixpkgs inputs
      ref="master" → inputs with ref="master" or no ref (default branch)
      ref=<other>  → inputs with that exact ref
    """
    root_inputs = lock_data.get("nodes", {}).get("root", {}).get("inputs", {})
    results = []

    for name, node_ref in root_inputs.items():
        # Skip follows (array values like ["nixpkgs-master"])
        if not isinstance(node_ref, str):
            continue

        node = lock_data["nodes"].get(node_ref)
        if not node:
            continue

        original = node.get("original", {})

        # Match by URL: github:NixOS/nixpkgs
        if original.get("type", "").lower() != "github":
            continue
        if not re.match(r"[Nn]ix[Oo][Ss]", original.get("owner", "")):
            continue
        if original.get("repo", "") != "nixpkgs":
            continue

        # Filter by ref
        input_ref = original.get("ref")
        if ref is not None:
            if ref == "master":
                # master matches explicit "master" or no ref (github default)
                if input_ref is not None and input_ref != "master":
                    continue
            else:
                if input_ref != ref:
                    continue

        results.append(name)

    return sorted(results)


def update_input(input_name, repo_dir):
    """Run nix flake update for a single input. Returns True on success."""
    result = subprocess.run(
        ["nix", "flake", "update", input_name, "--refresh"],
        cwd=str(repo_dir),
    )
    return result.returncode == 0


def format_result_lines(results):
    """Format result detail lines grouped by source URL."""
    url_order = []
    url_groups = {}
    for r in results:
        url = r["url"]
        if url not in url_groups:
            url_order.append(url)
            url_groups[url] = []
        url_groups[url].append(r)

    lines = []
    for url in url_order:
        lines.append("")
        lines.append(f" {url}")
        lines.append(f" {'─' * len(url)}")
        for r in url_groups[url]:
            if r["status"] == "updated":
                lines.append(
                    f"   ✓ {r['name']}  {r['old_rev']} → {r['new_rev']}  [ref: {r['ref']}]"
                )
            elif r["status"] == "skipped":
                lines.append(
                    f"   - {r['name']}  {r['old_rev']}  [ref: {r['ref']}] (already up-to-date)"
                )
            elif r["status"] == "failed":
                lines.append(f"   ✗ {r['name']}  {r['old_rev']}  [ref: {r['ref']}]")
    return lines


def print_ref_summary(label, results, not_found_refs=None):
    """Print the summary for a single ref category."""
    updated = [r for r in results if r["status"] == "updated"]
    skipped = [r for r in results if r["status"] == "skipped"]
    failed = [r for r in results if r["status"] == "failed"]

    print()
    print("=" * 40)
    print(f" Summary: NixOS/nixpkgs ({label}) inputs")
    print("=" * 40)
    print(f" Total:   {len(results)}")
    print(f" Updated: {len(updated)}")
    print(f" Skipped: {len(skipped)}")
    print(f" Failed:  {len(failed)}")
    print("=" * 40)

    for line in format_result_lines(results):
        print(line)

    if not_found_refs:
        print()
        print(" No inputs found for:")
        for nf in not_found_refs:
            print(f"   · {nf}")

    print()
    print("=" * 40)


def print_combined_summary(all_ref_data):
    """Print a combined overall summary across all ref categories."""
    all_results = []
    not_found_refs = []
    ref_labels = []

    for rd in all_ref_data:
        ref_labels.append(rd["label"])
        all_results.extend(rd["results"])
        if rd.get("not_found"):
            not_found_refs.append(rd["label"])

    updated = [r for r in all_results if r["status"] == "updated"]
    skipped = [r for r in all_results if r["status"] == "skipped"]
    failed = [r for r in all_results if r["status"] == "failed"]

    sep = "═" * 48
    print()
    print(sep)
    print(f" Overall Summary")
    print(sep)
    print(f" Categories: {', '.join(ref_labels)}")
    print(f" Total:      {len(all_results)}")
    print(f" Updated:    {len(updated)}")
    print(f" Skipped:    {len(skipped)}")
    print(f" Failed:     {len(failed)}")
    print(sep)

    for line in format_result_lines(all_results):
        print(line)

    if not_found_refs:
        print()
        print(" No inputs found for:")
        for nf in not_found_refs:
            print(f"   · {nf}")

    print()
    print(sep)


def main():
    script_dir = Path(__file__).resolve().parent
    repo_dir = script_dir.parent

    refs = sys.argv[1:] if len(sys.argv) > 1 else [None]

    seen_inputs = set()
    all_ref_data = []

    for ref in refs:
        label = ref if ref else "all"
        ref_data = {"label": label, "results": [], "not_found": False}

        lock_data = load_flake_lock(repo_dir)
        inputs = discover_inputs(lock_data, ref)

        if not inputs:
            ref_data["not_found"] = True
            all_ref_data.append(ref_data)
            continue

        # Filter out already-seen inputs
        new_inputs = [i for i in inputs if i not in seen_inputs]
        seen_inputs.update(new_inputs)

        if not new_inputs:
            all_ref_data.append(ref_data)
            continue

        print(f"Updating NixOS/nixpkgs ({label}) inputs: {' '.join(new_inputs)}")

        for input_name in new_inputs:
            print(f"--- Updating {input_name} ---")

            lock_data = load_flake_lock(repo_dir)
            old_rev, source_url, input_ref = get_input_info(lock_data, input_name)

            if update_input(input_name, repo_dir):
                lock_data = load_flake_lock(repo_dir)
                new_rev, _, _ = get_input_info(lock_data, input_name)

                if old_rev != new_rev:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "updated",
                            "old_rev": old_rev,
                            "new_rev": new_rev,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
                else:
                    ref_data["results"].append(
                        {
                            "name": input_name,
                            "status": "skipped",
                            "old_rev": old_rev,
                            "new_rev": None,
                            "url": source_url,
                            "ref": input_ref,
                        }
                    )
            else:
                ref_data["results"].append(
                    {
                        "name": input_name,
                        "status": "failed",
                        "old_rev": old_rev,
                        "new_rev": None,
                        "url": source_url,
                        "ref": input_ref,
                    }
                )
                print(f"WARNING: failed to update {input_name}", file=sys.stderr)

        all_ref_data.append(ref_data)

        # Print per-ref summary as we go
        not_found_in_ref = [rd["label"] for rd in all_ref_data if rd["not_found"]]
        print_ref_summary(label, ref_data["results"], not_found_in_ref)

    # Print combined overall summary if multiple refs were requested
    if len(refs) > 1:
        print_combined_summary(all_ref_data)


if __name__ == "__main__":
    main()
