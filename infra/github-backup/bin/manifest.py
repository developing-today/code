#!/usr/bin/env python3
"""manifest.py — regenerate MANIFEST.json + FAMILIES.md in the backup tree.

MANIFEST.json is the disaster-recovery source: config.json (in the repo)
says WHAT should exist; MANIFEST.json records what DOES exist, where, and
its state. Together they rebuild the whole tree (see docs/RUNBOOK.md § DR).
"""
import json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root, mirror_path, valid_mirror, disk_kb


def head_sha(path, branch=None):
    refs = ["refs/heads/" + branch] if branch else ["HEAD"]
    for ref in refs:
        r = subprocess.run(["git", "-C", path, "rev-parse", "--verify", "-q", ref],
                           capture_output=True, text=True)
        if r.returncode == 0:
            return r.stdout.strip()
    return None


def last_fetch(path, state):
    return state.get("last_fetch")


def build(root=None):
    cfg = load_config()
    root = root or backup_root(cfg)
    plan_path = os.path.join(root, "plan.json")
    plan = json.load(open(plan_path)) if os.path.exists(plan_path) else []
    state_path = os.path.join(root, "state.json")
    state = json.load(open(state_path)) if os.path.exists(state_path) else {}
    entries = {}
    families = {}
    for item in plan:
        repo = item["repo"]
        path = mirror_path(root, repo)
        present = valid_mirror(path)
        e = {
            "status": "mirrored" if present else "missing",
            "disk_kb": disk_kb(path) if present else 0,
            "github_size_kb": item.get("size_kb"),
            "last_fetch": (state.get(repo) or {}).get("last_fetch"),
            "head": head_sha(path, item.get("branch")) if present else None,
            "default_branch": item.get("branch"),
            "reference": item.get("reference"),
            "fork": item.get("fork"),
            "parent": item.get("parent"),
        }
        entries[repo] = e
        if item.get("reference"):
            families.setdefault(item["reference"], []).append(repo)
    manifest = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "config_version": cfg.get("version"),
        "mirrors": entries,
        "excluded": cfg.get("exclusions", []),
        "gaps": cfg.get("gaps", []),
        "totals": {
            "mirrored": sum(1 for e in entries.values() if e["status"] == "mirrored"),
            "missing": sum(1 for e in entries.values() if e["status"] != "mirrored"),
            "disk_kb": sum(e["disk_kb"] for e in entries.values()),
        },
    }
    with open(os.path.join(root, "MANIFEST.json"), "w") as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
    lines = ["# Fork families (git alternates pool)", "",
             "Members below are NOT self-contained mirrors: their object stores",
             "alternate into the anchor's objects. The backup TREE is the unit.",
             "Cold exports must use `git bundle` (materializes all objects).", ""]
    for anchor in sorted(families):
        lines.append(f"## anchor: {anchor}")
        for m in sorted(families[anchor]):
            lines.append(f"- {m}")
        lines.append("")
    with open(os.path.join(root, "FAMILIES.md"), "w") as f:
        f.write("\n".join(lines))
    return manifest


if __name__ == "__main__":
    m = build()
    print(json.dumps(m["totals"]))
