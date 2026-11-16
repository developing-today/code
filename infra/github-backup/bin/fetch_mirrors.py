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
from datetime import date, timedelta

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import (
    load_config,
    backup_root,
    mirror_path,
    valid_mirror,
    load_plan,
    load_state,
    report_problems,
    save_json,
    state_lock,
)
import manifest as manifest_mod

COALESCE_S = 12 * 3600


def record_failure_day(days, today):
    """Keep only the trailing consecutive calendar-day failure streak.

    A non-list `days` is a malformed state entry (#6637): fail as a
    single named error instead of escaping as a raw TypeError/KeyError
    from the streak walk or the in-place writes below.
    """
    if not isinstance(days, list):
        raise ValueError(
            f"fail_days must be a list of ISO dates, got {type(days).__name__}"
        )
    today_date = date.fromisoformat(today)
    start = len(days) - 1
    try:
        date.fromisoformat(days[start])
        while start > 0:
            current = date.fromisoformat(days[start])
            previous = date.fromisoformat(days[start - 1])
            if previous + timedelta(days=1) != current:
                break
            start -= 1
        streak = days[start:]
    except (TypeError, ValueError, IndexError):
        streak = []

    if streak:
        last_date = date.fromisoformat(streak[-1])
        if last_date == today_date:
            days[:] = streak
            return
        if last_date + timedelta(days=1) == today_date:
            days[:] = streak + [today]
            return

    days[:] = [today]


def _repo_state(state, repo):
    """The per-repo state dict, sanitized (#6637).

    A pre-existing non-dict entry (or a non-list fail_days) is poison
    from a wrong-shape state.json: report it, replace it with a clean
    entry, and let the fetch for this repo — and every other repo —
    proceed.
    """
    st = state.get(repo)
    if not isinstance(st, dict):
        if repo in state:
            print(
                f"github-backup: state entry for {repo!r} is "
                f"{type(st).__name__}, expected an object — replaced",
                file=sys.stderr,
            )
        st = {}
        state[repo] = st
    days = st.get("fail_days")
    if days is not None and not isinstance(days, list):
        print(
            f"github-backup: 'fail_days' for {repo!r} is "
            f"{type(days).__name__}, expected a list — reset to []",
            file=sys.stderr,
        )
        st["fail_days"] = []
    return st


def fetch_repo(root, repo, state):
    path = mirror_path(root, repo)
    if not valid_mirror(path):
        # #3270 / design #3282 rule 5: a missing mirror is an
        # unavailable day, recorded exactly like a failure day. The
        # missing path used to return before any state was touched,
        # so a repo with NO backup at all could never trip the
        # chronic alarm (3 consecutive days -> exit 1) that a merely
        # failing fetch trips. Success clears the record via the
        # same fail_days reset as the failure path.
        st = _repo_state(state, repo)
        today = time.strftime("%Y-%m-%d")
        days = st.setdefault("fail_days", [])
        record_failure_day(days, today)
        st["last_error"] = "mirror missing or invalid on disk"
        return "missing"
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
    try:
        r = subprocess.run(
            ["git", "-C", path, "fetch", "--all", "--prune"],
            capture_output=True,
            text=True,
            timeout=3600,
            env=env,
        )
    except subprocess.TimeoutExpired as e:
        st = _repo_state(state, repo)
        today = time.strftime("%Y-%m-%d")
        days = st.setdefault("fail_days", [])
        record_failure_day(days, today)
        st["last_error"] = f"fetch timed out after {e.timeout} seconds"
        return "fail"
    now = time.time()
    st = _repo_state(state, repo)
    if r.returncode == 0:
        st["last_fetch"] = now
        st["fail_days"] = []
        return "ok"
    today = time.strftime("%Y-%m-%d")
    days = st.setdefault("fail_days", [])
    record_failure_day(days, today)
    st["last_error"] = (r.stderr or r.stdout)[-300:]
    return "fail"


def iter_plan_repos(root):
    plan, problems = load_plan(root)
    report_problems(problems)
    return [p["repo"] for p in plan]


def main(argv):
    cfg = load_config()
    root = backup_root(cfg)
    # #3240: the whole load -> modify -> save cycle runs under the
    # shared state lock so the daily fetch and the 5-minute drainer
    # cannot interleave a lost update into state.json.
    with state_lock(root):
        return _main_locked(argv, root)


def _main_locked(argv, root):
    state_path = os.path.join(root, "state.json")
    state, problems = load_state(root)
    report_problems(problems)
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
                f.write(
                    f"{time.strftime('%Y-%m-%dT%H:%M:%S%z')} {repo} {res} "
                    f"{(state.get(repo) or {}).get('last_error', '')}\n"
                )
    save_json(state_path, state)
    manifest_mod.build(root)
    chronic = [
        r
        for r in repos
        if isinstance((state.get(r) or {}).get("fail_days"), list)
        and len(state[r]["fail_days"]) >= 3
    ]
    print(json.dumps({"results": results, "chronic_failures": chronic}))
    return 1 if chronic else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
