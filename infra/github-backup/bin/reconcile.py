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
guarantees nothing is missed. Exit code is non-zero on owner-level
inventory failure or a failed clone pass (#3300), so the timer unit
surfaces either.
"""

import json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import (
    load_config,
    backup_root,
    inventory,
    build_plan,
    load_plan,
    report_problems,
    valid_mirror,
    mirror_path,
    clone_status_record,
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


def _clone_status_entries(root, offset):
    """Parse clone-status.jsonl entries appended after byte offset.

    Per-line shape tolerance is the shared contract in
    ghb_common.clone_status_record (#6606): unparseable lines and
    valid-JSON non-object lines are both ignored, so a poison line
    can never crash the failed-clone consumer below.
    """
    path = os.path.join(root, "clone-status.jsonl")
    entries = []
    try:
        with open(path) as f:
            f.seek(offset)
            for line in f:
                record = clone_status_record(line)
                if record is not None:
                    entries.append(record)
    except OSError:
        pass
    return entries


def main():
    cfg = load_config()
    root = backup_root(cfg)
    os.makedirs(root, exist_ok=True)
    repos, errors = inventory(cfg["owners"])
    plan, excluded, gaps, pending, skipped_empty = build_plan(repos, cfg)
    today = time.strftime("%Y-%m-%d")
    old_plan_path = os.path.join(root, "plan.json")
    old_items, old_plan_problems = load_plan(root)
    report_problems(old_plan_problems)
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
    # Clone pass (#3300): run BEFORE the report is written so its
    # outcome lands in the dated report, and propagate a non-zero exit
    # instead of dropping clone_all's verdict on the floor.
    clone_rc = None
    clone_failures = []
    if missing:
        status_path = os.path.join(root, "clone-status.jsonl")
        status_offset = (
            os.path.getsize(status_path) if os.path.exists(status_path) else 0
        )
        r = subprocess.run(
            [
                sys.executable,
                os.path.join(
                    os.path.dirname(os.path.abspath(__file__)), "clone_all.py"
                ),
            ]
        )
        clone_rc = r.returncode
        if clone_rc != 0:
            clone_failures = sorted(
                {
                    e["repo"]
                    for e in _clone_status_entries(root, status_offset)
                    if e.get("status") == "fail"
                    and isinstance(e.get("repo"), str)
                    and e.get("repo")
                }
            )
    if clone_rc is None:
        lines.append("- clone pass: not run (no mirrors missing)")
    elif clone_rc == 0:
        lines.append(f"- clone pass: succeeded ({len(missing)} mirrors attempted)")
    else:
        lines.append(
            f"- clone pass: FAILED (clone_all exit {clone_rc}; "
            f"{len(missing)} mirrors attempted)"
        )
    lines.append("")
    if clone_rc not in (None, 0):
        lines.append("## clone pass failures")
        if clone_failures:
            lines += [f"- {r}" for r in clone_failures]
        else:
            lines.append(
                "- no per-repo failures recorded in clone-status.jsonl; "
                "see clone_all output"
            )
        lines.append("")
    report_path = os.path.join(root, f"reconcile-{today}.md")
    with open(report_path, "w") as f:
        f.write("\n".join(lines))
    print("\n".join(lines))
    if clone_rc not in (None, 0):
        print("clone pass had failures; see clone-status.jsonl")
    manifest_mod.build(root)
    return 1 if errors or clone_rc not in (None, 0) else 0


if __name__ == "__main__":
    sys.exit(main())
