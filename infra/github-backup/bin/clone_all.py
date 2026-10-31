#!/usr/bin/env python3
"""clone_all.py — resumable parallel mirror-clone driver.

Reads plan.json from the backup tree (build it with reconcile.py --plan-only
or copy from a previous run). Mirrors land at <root>/<owner>/<repo>.git.
Repos with a `reference` wait for their anchor; if the anchor failed they
fall back to a full clone. Status is appended to clone-status.jsonl;
re-running skips verified mirrors.
"""
import json, os, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config, backup_root, mirror_path, valid_mirror, disk_kb

PAR = 4
TIMEOUT = 5400


def main():
    cfg = load_config()
    root = backup_root(cfg)
    plan = json.load(open(os.path.join(root, "plan.json")))
    status_path = os.path.join(root, "clone-status.jsonl")

    def log_status(rec):
        rec["ts"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        with open(status_path, "a") as f:
            f.write(json.dumps(rec) + "\n")

    ok_prev = set()
    if os.path.exists(status_path):
        for line in open(status_path):
            try:
                r = json.loads(line)
                if r.get("status") == "ok":
                    ok_prev.add(r["repo"])
            except Exception:
                pass

    def clone_one(item, ref_ok):
        repo = item["repo"]
        dest = mirror_path(root, repo)
        if valid_mirror(dest):
            return {"repo": repo, "status": "ok", "note": "already-present",
                    "disk_kb": disk_kb(dest)}
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        cmd = ["git", "clone", "--mirror"]
        ref = item.get("reference")
        if ref and ref_ok and valid_mirror(mirror_path(root, ref)):
            cmd += ["--reference", mirror_path(root, ref)]
        cmd += [f"https://github.com/{repo}.git", dest]
        t0 = time.time()
        env = dict(os.environ, GIT_TERMINAL_PROMPT="0")
        try:
            r = subprocess.run(cmd, capture_output=True, text=True,
                               timeout=TIMEOUT, env=env)
        except subprocess.TimeoutExpired:
            return {"repo": repo, "status": "fail", "error": "timeout",
                    "seconds": round(time.time() - t0)}
        dt = round(time.time() - t0)
        if r.returncode == 0 and valid_mirror(dest):
            return {"repo": repo, "status": "ok", "seconds": dt,
                    "disk_kb": disk_kb(dest),
                    "reference_used": ref if (ref and ref_ok) else None}
        return {"repo": repo, "status": "fail", "seconds": dt,
                "error": (r.stderr or r.stdout)[-400:]}

    state = {}
    for p in plan:
        if p["repo"] in ok_prev and valid_mirror(mirror_path(root, p["repo"])):
            state[p["repo"]] = "ok"
    pending = [p for p in plan if state.get(p["repo"]) != "ok"]
    print(f"plan={len(plan)} already-ok={len(plan)-len(pending)} pending={len(pending)}", flush=True)
    inflight = {}
    with ThreadPoolExecutor(max_workers=PAR) as ex:
        while pending or inflight:
            for p in list(pending):
                ref = p.get("reference")
                if ref and state.get(ref) is None and any(q["repo"] == ref for q in pending):
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
    print(f"DONE ok={ok} fail={len(state)-ok} of {len(plan)}", flush=True)
    return 0 if ok == len(plan) else 1


if __name__ == "__main__":
    sys.exit(main())
