#!/usr/bin/env python3
"""clone_all.py — resumable parallel mirror-clone driver.

Reads plan.json from the backup tree (build it with reconcile.py --plan-only
or copy from a previous run). Mirrors land at <root>/<owner>/<repo>.git.
Repos with a `reference` wait for their anchor; if the anchor failed they
fall back to a full clone. Status is appended to clone-status.jsonl;
re-running skips verified mirrors.
"""

import json, os, shutil, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root, mirror_path, valid_mirror, disk_kb, load_plan, report_problems, clone_status_record

PAR = 4
TIMEOUT = 5400


def clear_partial_dest(root, dest):
    """Remove a leftover partial clone destination (#3233).

    A clone killed mid-flight (driver TIMEOUT, interrupt, laptop sleep)
    leaves a non-empty destination that is not a valid mirror; git
    refuses to clone into it ('already exists and is not an empty
    directory'), so without this every rerun wedges on the same repo.

    Safety: only the driver-managed mirror path itself is removed —
    dest must resolve to <root>/<owner>/<repo>.git exactly as
    mirror_path builds it. Anything else is left untouched and the
    caller fails loudly. An empty directory is not partial (git clone
    accepts it), and a valid mirror never reaches this helper.
    Returns True when a partial destination was cleared.
    """
    if not os.path.lexists(dest):
        return False
    if os.path.isdir(dest) and not os.path.islink(dest) and not os.listdir(dest):
        return False
    root_real = os.path.realpath(root)
    dest_real = os.path.realpath(dest)
    if os.path.dirname(os.path.dirname(dest_real)) != root_real:
        return False
    if os.path.isdir(dest) and not os.path.islink(dest):
        shutil.rmtree(dest)
    else:
        os.remove(dest)
    return True


def dest_is_confirmed_partial(dest):
    """True only when dest is positively known not to be a repository.

    valid_mirror() returning False does not by itself prove a partial
    clone: git also refuses intact mirrors for policy reasons (dubious
    ownership after a restore, safe.bareRepository=explicit), and
    deleting an intact backup because inspection failed would be far
    worse than the wedge this fixes (Codex P1 on the #3233 PR). So:

    - a non-directory at the mirror path is a stray file — partial;
    - git answering at all (rc 0) means a real repository lives there
      — never partial, whatever valid_mirror made of it;
    - git saying 'not a git repository' is the killed-clone / junk
      shape — partial;
    - any other git failure is an inspection/policy error — NOT
      confirmed partial, and the caller preserves the destination.
    """
    if not os.path.isdir(dest):
        return True
    r = subprocess.run(
        ["git", "-C", dest, "rev-parse", "--is-bare-repository"],
        capture_output=True,
        text=True,
    )
    if r.returncode == 0:
        return False
    return "not a git repository" in (r.stderr or "")


def main():
    cfg = load_config()
    root = backup_root(cfg)
    plan, problems = load_plan(root)
    report_problems(problems)
    status_path = os.path.join(root, "clone-status.jsonl")

    def log_status(rec):
        rec["ts"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        with open(status_path, "a") as f:
            f.write(json.dumps(rec) + "\n")

    ok_prev = set()
    if os.path.exists(status_path):
        for line in open(status_path):
            # Shared line contract: ghb_common.clone_status_record
            # (#6606) — non-record lines are ignored, never crash.
            r = clone_status_record(line)
            if (
                r is not None
                and r.get("status") == "ok"
                and isinstance(r.get("repo"), str)
                and r["repo"]
            ):
                ok_prev.add(r["repo"])

    def clone_one(item, ref_ok):
        repo = item["repo"]
        dest = mirror_path(root, repo)
        if valid_mirror(dest):
            return {
                "repo": repo,
                "status": "ok",
                "note": "already-present",
                "disk_kb": disk_kb(dest),
            }
        # #3233: a destination that exists but is not a valid mirror may
        # be the leftover of a killed clone — clear it and retry, or the
        # clone below fails on 'already exists' forever. But only a
        # CONFIRMED partial is cleared: if git can open the destination
        # as a repository, or cannot inspect it for policy reasons
        # (dubious ownership, safe.bareRepository), it may be an intact
        # backup — preserve it and fail loudly instead. The status
        # record carries cleared_partial so a cleared-partial retry is
        # distinguishable from an ordinary failure in clone-status.jsonl.
        cleared_partial = False
        if os.path.lexists(dest) and not (os.path.isdir(dest) and not os.listdir(dest)):
            if not dest_is_confirmed_partial(dest):
                return {
                    "repo": repo,
                    "status": "fail",
                    "error": "destination exists and is not a valid "
                    "mirror, but is not a confirmed partial clone (git "
                    "can open it, or inspection failed for policy "
                    f"reasons); preserving it: {dest}",
                }
            cleared_partial = clear_partial_dest(root, dest)
            if os.path.lexists(dest):
                return {
                    "repo": repo,
                    "status": "fail",
                    "error": "partial destination is outside the "
                    "driver-managed mirror tree; refusing to remove "
                    f"it: {dest}",
                }
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        cmd = ["git", "clone", "--mirror"]
        ref = item.get("reference")
        if ref and ref_ok and valid_mirror(mirror_path(root, ref)):
            cmd += ["--reference", mirror_path(root, ref)]
        cmd += [f"https://github.com/{repo}.git", dest]
        t0 = time.time()
        env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
        try:
            r = subprocess.run(
                cmd, capture_output=True, text=True, timeout=TIMEOUT, env=env
            )
        except subprocess.TimeoutExpired:
            rec = {
                "repo": repo,
                "status": "fail",
                "error": "timeout",
                "seconds": round(time.time() - t0),
            }
            if cleared_partial:
                rec["cleared_partial"] = True
            return rec
        dt = round(time.time() - t0)
        if r.returncode == 0 and valid_mirror(dest):
            rec = {
                "repo": repo,
                "status": "ok",
                "seconds": dt,
                "disk_kb": disk_kb(dest),
                "reference_used": ref if (ref and ref_ok) else None,
            }
            if cleared_partial:
                rec["cleared_partial"] = True
            return rec
        rec = {
            "repo": repo,
            "status": "fail",
            "seconds": dt,
            "error": (r.stderr or r.stdout)[-400:],
        }
        if cleared_partial:
            rec["cleared_partial"] = True
        return rec

    state = {}
    for p in plan:
        if p["repo"] in ok_prev and valid_mirror(mirror_path(root, p["repo"])):
            state[p["repo"]] = "ok"
    pending = [p for p in plan if state.get(p["repo"]) != "ok"]
    print(
        f"plan={len(plan)} already-ok={len(plan) - len(pending)} pending={len(pending)}",
        flush=True,
    )
    inflight = {}
    with ThreadPoolExecutor(max_workers=PAR) as ex:
        while pending or inflight:
            for p in list(pending):
                ref = p.get("reference")
                if (
                    ref
                    and state.get(ref) is None
                    and (
                        any(q["repo"] == ref for q in pending)
                        or any(q["repo"] == ref for q in inflight.values())
                    )
                ):
                    continue
                if len(inflight) >= PAR:
                    break
                inflight[ex.submit(clone_one, p, state.get(ref) == "ok")] = p
                pending.remove(p)
            if not inflight:
                if pending:
                    p = pending.pop(0)
                    inflight[ex.submit(clone_one, p, False)] = p
                else:
                    break
            done = [f for f in list(inflight) if f.done()]
            if not done:
                time.sleep(2)
                continue
            for f in done:
                p = inflight.pop(f)
                try:
                    rec = f.result()
                except Exception as e:
                    rec = {"repo": p["repo"], "status": "fail", "error": str(e)[:300]}
                state[p["repo"]] = "ok" if rec["status"] == "ok" else "fail"
                log_status(rec)
                print(rec["repo"], rec["status"], rec.get("seconds", ""), flush=True)
    ok = sum(1 for s in state.values() if s == "ok")
    print(f"DONE ok={ok} fail={len(state) - ok} of {len(plan)}", flush=True)
    return 0 if ok == len(plan) else 1


if __name__ == "__main__":
    sys.exit(main())
