#!/usr/bin/env python3
"""reconcile.py — weekly reconciliation sweep (the correctness floor).

1. Fresh inventory of every owner in config.json via the GitHub GraphQL API.
2. Build the plan with the config tier rules; diff against the tree:
   - new repos matching the rules  -> cloned (clone_all logic via plan.json)
   - new giant forks               -> MANIFEST 'pending-drew-review', not cloned
   - repos gone from GitHub        -> reported, mirror kept
   - inventory errors per owner    -> reported (e.g. SAML-blocked orgs)
3. Write reconcile-YYYY-MM-DD.md into the backup tree and refresh
   plan.json + MANIFEST.json.

Webhooks (the App + Worker) are the freshness layer; this sweep is what
guarantees nothing is missed. Exit code is non-zero only on owner-level
inventory failure, so the timer unit surfaces it.
"""

import json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import (
    load_config,
    backup_root,
    inventory,
    build_plan,
    valid_mirror,
    mirror_path,
)
import manifest as manifest_mod


def _dependency_order(plan):
    """Order a merged plan the way build_plan does: parents (anchors)
    before their members, then by repo name."""
    by_repo = {p["repo"]: p for p in plan}

    def depth(item):
        n, cur, guard = 0, item, set()
        while True:
            p = cur.get("parent")
            if p and p in by_repo and p not in guard:
                guard.add(p)
                n += 1
                cur = by_repo[p]
            else:
                return n

    return sorted(plan, key=lambda it: (depth(it), it["repo"]))


def main():
    cfg = load_config()
    root = backup_root(cfg)
    os.makedirs(root, exist_ok=True)
    repos, errors = inventory(cfg["owners"])
    plan, excluded, gaps, pending, skipped_empty = build_plan(repos, cfg)
    today = time.strftime("%Y-%m-%d")
    old_plan_path = os.path.join(root, "plan.json")
    old_items = []
    if os.path.exists(old_plan_path):
        old_items = json.load(open(old_plan_path))
    old = [p["repo"] for p in old_items]
    # Last-known-good (#3229): an owner whose inventory FAILED (null
    # repositoryOwner, SAML-blocked, gh error, ...) keeps its previously
    # planned entries in the new plan. Without this, one invisible owner
    # would silently drop out of plan.json and MANIFEST.json while the
    # run still exits on the error path alone. Owners that inventoried
    # successfully are unaffected: their drops are real.
    retained = []
    if errors:
        planned_repos = {p["repo"] for p in plan}
        for item in old_items:
            if (
                item["repo"].split("/")[0] in errors
                and item["repo"] not in planned_repos
            ):
                plan.append(item)
                retained.append(item["repo"])
        if retained:
            plan = _dependency_order(plan)
    new = [p["repo"] for p in plan]
    added = sorted(set(new) - set(old))
    removed = sorted(set(old) - set(new))
    missing = [r for r in new if not valid_mirror(mirror_path(root, r))]
    with open(old_plan_path, "w") as f:
        json.dump(plan, f, indent=0)
    lines = [
        f"# github-backup reconcile {today}",
        "",
        f"- inventory: {len(repos)} repos across {len(cfg['owners'])} owners",
        f"- plan: {len(plan)} mirrors ({len(added)} new, {len(removed)} no longer planned)",
        f"- excluded by config: {len(excluded)}; gaps: {len(gaps)}; "
        f"pending Drew review: {len(pending)}; zero-commit skipped: {len(skipped_empty)}",
        f"- mirrors missing on disk before clone pass: {len(missing)}",
        "",
    ]
    if errors:
        lines.append("## inventory errors")
        for o, e in errors.items():
            lines.append(f"- {o}: {e}")
        lines.append("")
    if retained:
        lines.append("## plan entries retained for failed owners (last-known-good)")
        lines.append(
            f"- {len(retained)} previously planned repos kept in "
            f"plan.json + MANIFEST.json because their owner's "
            f"inventory failed; their mirrors stay covered until "
            f"the owner inventories cleanly again:"
        )
        lines += [f"- {r}" for r in sorted(retained)] + [""]
    if added:
        lines.append("## newly planned")
        lines += [f"- {r}" for r in added] + [""]
    if removed:
        lines.append("## no longer planned (mirrors kept on disk)")
        lines += [f"- {r}" for r in removed] + [""]
    if pending:
        lines.append("## pending Drew review (giant forks, NOT cloned)")
        lines += [f"- {p['repo']} — {p['reason']}" for p in pending] + [""]
    report_path = os.path.join(root, f"reconcile-{today}.md")
    with open(report_path, "w") as f:
        f.write("\n".join(lines))
    print("\n".join(lines))
    if missing:
        r = subprocess.run(
            [
                sys.executable,
                os.path.join(
                    os.path.dirname(os.path.abspath(__file__)), "clone_all.py"
                ),
            ]
        )
        if r.returncode != 0:
            print("clone pass had failures; see clone-status.jsonl")
    manifest_mod.build(root)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
