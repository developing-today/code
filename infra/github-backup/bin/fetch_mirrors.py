#!/usr/bin/env python3
"""fetch_mirrors.py — daily fetch of every mirror in the tree.

- `git fetch --all --prune` (+ tags are part of the mirror refspec) per repo.
- Coalescing: mirrors fetched <12h ago are skipped (state.json).
- Failures are appended to fetch.log. A repo failing on 3 distinct days in
  a row makes the run exit non-zero (so systemd shows the failure).
- Repos named on the command line are fetched unconditionally (used by the
  webhook drainer); no args = the daily full pass.
"""
import json, os, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root, mirror_path, valid_mirror
import manifest as manifest_mod

COALESCE_S = 12 * 3600


def fetch_repo(root, repo, state):
    path = mirror_path(root, repo)
    if not valid_mirror(path):
        return "missing"
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    r = subprocess.run(["git", "-C", path, "fetch", "--all", "--prune"],
                       capture_output=True, text=True, timeout=3600, env=env)
    now = time.time()
    st = state.setdefault(repo, {})
    if r.returncode == 0:
        st["last_fetch"] = now
        st["fail_days"] = []
        return "ok"
    today = time.strftime("%Y-%m-%d")
    days = st.setdefault("fail_days", [])
    if today not in days:
        days.append(today)
    st["last_error"] = (r.stderr or r.stdout)[-300:]
    return "fail"


def iter_plan_repos(root):
    plan = json.load(open(os.path.join(root, "plan.json")))
    return [p["repo"] for p in plan]


def main(argv):
    cfg = load_config()
    root = backup_root(cfg)
    state_path = os.path.join(root, "state.json")
    state = json.load(open(state_path)) if os.path.exists(state_path) else {}
    log_path = os.path.join(root, "fetch.log")
    only = argv[1:]
    repos = only if only else iter_plan_repos(root)
    now = time.time()
    results = {"ok": 0, "fail": 0, "skip": 0, "missing": 0}
    failed = []
    for repo in repos:
        st = state.get(repo) or {}
        if not only and st.get("last_fetch") and now - st["last_fetch"] < COALESCE_S:
            results["skip"] += 1
            continue
        res = fetch_repo(root, repo, state)
        results[res] += 1
        if res != "ok":
            failed.append((repo, res))
            with open(log_path, "a") as f:
                f.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S%z')} {repo} {res} "
                        f"{(state.get(repo) or {}).get('last_error', '')}\n")
    with open(state_path, "w") as f:
        json.dump(state, f, indent=0)
    manifest_mod.build(root)
    chronic = [r for r in repos
               if len((state.get(r) or {}).get("fail_days", [])) >= 3]
    print(json.dumps({"results": results, "chronic_failures": chronic}))
    return 1 if chronic else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
