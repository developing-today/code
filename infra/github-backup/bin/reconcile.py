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
from ghb_common import load_config, backup_root, inventory, build_plan, valid_mirror, mirror_path
import manifest as manifest_mod


def main():
    cfg = load_config()
    root = backup_root(cfg)
    os.makedirs(root, exist_ok=True)
    repos, errors = inventory(cfg["owners"])
    plan, excluded, gaps, pending, skipped_empty = build_plan(repos, cfg)
    today = time.strftime("%Y-%m-%d")
    old_plan_path = os.path.join(root, "plan.json")
    old = []
    if os.path.exists(old_plan_path):
        old = [p["repo"] for p in json.load(open(old_plan_path))]
    new = [p["repo"] for p in plan]
    added = sorted(set(new) - set(old))
    removed = sorted(set(old) - set(new))
    missing = [r for r in new if not valid_mirror(mirror_path(root, r))]
    with open(old_plan_path, "w") as f:
        json.dump(plan, f, indent=0)
    lines = [f"# github-backup reconcile {today}", "",
             f"- inventory: {len(repos)} repos across {len(cfg['owners'])} owners",
             f"- plan: {len(plan)} mirrors ({len(added)} new, {len(removed)} no longer planned)",
             f"- excluded by config: {len(excluded)}; gaps: {len(gaps)}; "
             f"pending Drew review: {len(pending)}; zero-commit skipped: {len(skipped_empty)}",
             f"- mirrors missing on disk before clone pass: {len(missing)}", ""]
    if errors:
        lines.append("## inventory errors")
        for o, e in errors.items():
            lines.append(f"- {o}: {e}")
        lines.append("")
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
        r = subprocess.run([sys.executable,
                            os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                         "clone_all.py")])
        if r.returncode != 0:
            print("clone pass had failures; see clone-status.jsonl")
    manifest_mod.build(root)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
