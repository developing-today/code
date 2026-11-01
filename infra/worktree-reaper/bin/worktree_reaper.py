#!/usr/bin/env python3
"""worktree_reaper.py — retire dead fleet worktrees on the laptop.

Implements Drew's quiet-worktree rule (2026-11-01): a LINKED worktree is
reaped only when ALL of these gates hold at decision time:

  1. PUSHED    the tree's branch tip is proven on origin — equal to
               origin/<branch>, an ancestor of it, contained in origin's
               default branch, or patch-equivalent to it (`git cherry`).
               Detached HEADs are never reaped in v1.
  2. CLEAN     no tracked changes and no untracked files, except the
               generated-dirt allowlist (config: generated_dirt_prefixes,
               e.g. pkgs/mcpx/internal/cli/logs/, and generated_dirt_files,
               e.g. fleet-feed.json — matched on untracked entries only,
               re-verified in the same run just before removal).
  3. QUIET     last activity >= quiet_hours ago (absolute epoch). Activity
               = newest working-tree file mtime excluding the tree's root
               .git entry, max()ed with the HEAD reflog timestamp.
  4. UNOCCUPIED a fresh /proc cwd scan shows no process inside the tree
               (re-scanned immediately before each removal).
  5. NOT DENIED the tree is not on the config denylist, is not a primary,
               is not under ~/backups or ~/atlas, and does not contain
               this running script.

Reap = `git worktree remove` (plain; --force only when the sole blocker
is allowlisted generated dirt) + `git worktree prune` + `git branch -d`
(plain -d only; a refusal keeps the branch and is logged). Removals per
run are capped (config max_removals, default 100). Dirty or unpushed
trees are counted in the run summary, never auto-rescued, never
auto-issued.

Every decision is logged as JSONL to <state_dir>/runs/<ts>.jsonl plus a
summary object in <state_dir>/last-run.json. Kill switch: if
<state_dir>/DISABLED exists the reaper exits 0 doing nothing.
--dry-run evaluates everything and reports the would-reap list without
removing anything.

Config resolution: --config PATH, else $WORKTREE_REAPER_CONFIG, else
config.json next to this script's parent dir (infra/worktree-reaper/),
else config.json next to the script. An optional overlay at
<state_dir>/config.local.json is merged on top (deny lists union,
scalars override) — install-units.sh uses it to denylist the install
tree when the units were installed from a linked worktree.

Standard library only. Run the tests:
  python3 tests/test_worktree_reaper.py
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

GIT_TIMEOUT = 300
FETCH_TIMEOUT = 900

DEFAULT_CONFIG = {
    "git_root": "~/git",
    "state_dir": "~/.local/state/worktree-reaper",
    "quiet_hours": 6,
    "max_removals": 100,
    "primaries": [],  # empty = discover: dirs under git_root whose .git is a directory
    "deny_paths": [],
    "deny_prefixes": [],
    "deny_names": [],
    "generated_dirt_prefixes": ["pkgs/mcpx/internal/cli/logs/"],
    "generated_dirt_files": ["fleet-feed.json"],
    "fetch": True,
}

HARD_DENY_PREFIXES = ("~/backups", "~/atlas")


# ---------------------------------------------------------------- utils

def expand(p):
    return os.path.abspath(os.path.expanduser(p))


def run_git(repo, *args, timeout=GIT_TIMEOUT):
    """Run git -C repo <args>; return (rc, stdout_bytes, stderr_text)."""
    try:
        proc = subprocess.run(
            ["git", "-C", repo, *args],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
        return proc.returncode, proc.stdout, proc.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return 124, b"", "git timeout"
    except OSError as exc:
        return 127, b"", str(exc)


def load_config(script_dir, args):
    cfg = dict(DEFAULT_CONFIG)
    cfg_path = None
    if args.config:
        cfg_path = args.config
    elif os.environ.get("WORKTREE_REAPER_CONFIG"):
        cfg_path = os.environ["WORKTREE_REAPER_CONFIG"]
    else:
        for cand in (
            os.path.join(script_dir, "..", "config.json"),
            os.path.join(script_dir, "config.json"),
        ):
            if os.path.isfile(cand):
                cfg_path = cand
                break
    if cfg_path:
        with open(cfg_path) as fh:
            cfg.update(json.load(fh))
        cfg["_config_path"] = os.path.abspath(cfg_path)
    state_dir = expand(args.state_dir or cfg["state_dir"])
    overlay = os.path.join(state_dir, "config.local.json")
    if os.path.isfile(overlay):
        with open(overlay) as fh:
            local = json.load(fh)
        for key, val in local.items():
            if key in ("deny_paths", "deny_prefixes", "deny_names") and isinstance(val, list):
                cfg[key] = list(cfg.get(key, [])) + val
            else:
                cfg[key] = val
        cfg["_overlay_path"] = overlay
    if args.quiet_hours is not None:
        cfg["quiet_hours"] = args.quiet_hours
    if args.max_removals is not None:
        cfg["max_removals"] = args.max_removals
    if args.no_fetch:
        cfg["fetch"] = False
    cfg["state_dir"] = state_dir
    return cfg


# ------------------------------------------------------- repo inventory

def discover_primaries(cfg):
    if cfg.get("primaries"):
        return [expand(p) for p in cfg["primaries"]]
    root = expand(cfg["git_root"])
    found = []
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return found
    for name in names:
        path = os.path.join(root, name)
        # A primary checkout has .git as a *directory*; a linked worktree
        # has .git as a file. That is the structural distinction.
        if os.path.isdir(path) and os.path.isdir(os.path.join(path, ".git")):
            found.append(path)
    return found


def parse_worktree_porcelain(raw):
    """Parse `git worktree list --porcelain -z` output (bytes).

    Returns a list of dicts with keys: path, head, branch (short name or
    None), detached, bare, prunable, locked.
    """
    entries = []
    cur = None

    def flush():
        nonlocal cur
        if cur is not None and cur.get("path"):
            entries.append(cur)
        cur = None

    for token in raw.split(b"\0"):
        line = token.decode("utf-8", "surrogateescape")
        if line == "":
            flush()
            continue
        if line.startswith("worktree "):
            flush()
            cur = {"path": line[len("worktree "):], "head": None, "branch": None,
                   "detached": False, "bare": False, "prunable": False,
                   "locked": False}
        elif cur is None:
            continue
        elif line.startswith("HEAD "):
            cur["head"] = line[len("HEAD "):]
        elif line.startswith("branch refs/heads/"):
            cur["branch"] = line[len("branch refs/heads/"):]
        elif line == "detached":
            cur["detached"] = True
        elif line == "bare":
            cur["bare"] = True
        elif line == "prunable":
            cur["prunable"] = True
        elif line == "locked" or line.startswith("locked "):
            cur["locked"] = True
    flush()
    return entries


def list_worktrees(primary):
    rc, out, err = run_git(primary, "worktree", "list", "--porcelain", "-z")
    if rc != 0:
        raise RuntimeError(f"worktree list failed in {primary}: {err.strip()}")
    return parse_worktree_porcelain(out)


def fetch_primary(primary):
    rc, _, err = run_git(
        primary, "fetch", "--quiet", "--prune", "origin",
        "+refs/heads/*:refs/remotes/origin/*", timeout=FETCH_TIMEOUT,
    )
    return rc == 0, err.strip()


# ---------------------------------------------------------------- gates

def is_denied(tree, cfg, script_real):
    rp = os.path.realpath(tree)
    deny_paths = [os.path.realpath(expand(p)) for p in cfg.get("deny_paths", [])]
    if rp in deny_paths:
        return "deny-path"
    for prefix in [expand(p) for p in cfg.get("deny_prefixes", [])] + \
                  [expand(p) for p in HARD_DENY_PREFIXES]:
        pp = os.path.realpath(prefix)
        if rp == pp or rp.startswith(pp + os.sep):
            return "deny-prefix"
    if os.path.basename(rp) in cfg.get("deny_names", []):
        return "deny-name"
    # Never reap the tree this script is running from (install tree when
    # the units were installed out of a linked worktree pre-merge).
    if script_real == rp or script_real.startswith(rp + os.sep):
        return "install-tree"
    return None


def ref_sha(primary, ref):
    rc, out, _ = run_git(primary, "rev-parse", "--verify", "--quiet", ref + "^{commit}")
    if rc != 0:
        return None
    return out.decode().strip()


def is_ancestor(primary, anc, desc_ref):
    rc, _, _ = run_git(primary, "merge-base", "--is-ancestor", anc, desc_ref)
    return rc == 0


def cherry_equivalent(primary, upstream_ref, tip):
    rc, out, _ = run_git(primary, "cherry", upstream_ref, tip)
    if rc != 0:
        return False
    for line in out.decode("utf-8", "replace").splitlines():
        if line.startswith("+"):
            return False
    return True


def pushed_proof(primary, tip, branch):
    """Return (ok, method). Any one proof suffices (tonight's method)."""
    if not tip:
        return False, "no-tip"
    default_ref = None
    rc, out, _ = run_git(primary, "symbolic-ref", "--quiet", "refs/remotes/origin/HEAD")
    if rc == 0:
        default_ref = out.decode().strip()  # e.g. refs/remotes/origin/main
    if default_ref and ref_sha(primary, default_ref) is None:
        default_ref = None
    up_ref = f"refs/remotes/origin/{branch}" if branch else None
    if up_ref and ref_sha(primary, up_ref) is None:
        up_ref = None
    if up_ref:
        if ref_sha(primary, up_ref) == tip:
            return True, "upstream-equal"
        if is_ancestor(primary, tip, up_ref):
            return True, "ancestor-of-upstream"
    if default_ref:
        if is_ancestor(primary, tip, default_ref):
            return True, "contained-in-default"
        if cherry_equivalent(primary, default_ref, tip):
            return True, "patch-equivalent-default"
    if up_ref and cherry_equivalent(primary, up_ref, tip):
        return True, "patch-equivalent-upstream"
    return False, "no-proof"


def status_entries(tree):
    """Return (tracked_dirty, untracked) repo-relative path lists."""
    rc, out, err = run_git(
        tree, "status", "--porcelain=v1", "-z", "--no-renames",
        "--untracked-files=all",
    )
    if rc != 0:
        raise RuntimeError(f"status failed in {tree}: {err.strip()}")
    tracked, untracked = [], []
    for token in out.split(b"\0"):
        if len(token) < 4:
            continue
        xy = token[:2].decode("ascii", "replace")
        path = token[3:].decode("utf-8", "surrogateescape")
        if xy == "??":
            untracked.append(path)
        else:
            tracked.append(path)
    return tracked, untracked


def dirt_allowlisted(untracked, cfg):
    """Split untracked paths into (allowlisted, other) per the config."""
    prefixes = [p if p.endswith("/") else p + "/" for p in cfg.get("generated_dirt_prefixes", [])]
    names = set(cfg.get("generated_dirt_files", []))
    allow, other = [], []
    for path in untracked:
        if any(path.startswith(p) for p in prefixes) or os.path.basename(path) in names:
            allow.append(path)
        else:
            other.append(path)
    return allow, other


_REFLOG_TS = re.compile(r"\s(\d{9,})\s[+-]\d{4}\t")


def head_reflog_ts(tree):
    rc, out, _ = run_git(tree, "rev-parse", "--absolute-git-dir")
    if rc != 0:
        return 0
    log = os.path.join(out.decode().strip(), "logs", "HEAD")
    try:
        with open(log, "rb") as fh:
            lines = [ln for ln in fh.read().splitlines() if ln.strip()]
    except OSError:
        return 0
    if not lines:
        return 0
    m = _REFLOG_TS.search(lines[-1].decode("utf-8", "replace"))
    return int(m.group(1)) if m else 0


def walk_activity(tree, cutoff):
    """Newest file mtime in the working tree (root .git entry excluded).

    Returns (max_mtime, total_file_bytes, complete). Stops early with
    complete=False as soon as a file newer than cutoff is seen (the tree
    is then certainly not quiet; a full walk would prove nothing more).
    Raises OSError on traversal failure — callers fail closed.
    """
    max_mtime = 0.0
    total = 0
    try:
        root_stat = os.lstat(tree)
        max_mtime = root_stat.st_mtime
        if max_mtime > cutoff:
            return max_mtime, 0, False
    except OSError:
        raise
    stack = [tree]
    while stack:
        current = stack.pop()
        with os.scandir(current) as it:
            for entry in it:
                if current == tree and entry.name == ".git":
                    continue
                st = entry.stat(follow_symlinks=False)
                if st.st_mtime > max_mtime:
                    max_mtime = st.st_mtime
                    if max_mtime > cutoff:
                        return max_mtime, total, False
                if entry.is_dir(follow_symlinks=False):
                    stack.append(entry.path)
                elif entry.is_file(follow_symlinks=False):
                    total += st.st_size
    return max_mtime, total, True


def proc_cwds():
    cwds = []
    me = str(os.getpid())
    try:
        pids = os.listdir("/proc")
    except OSError:
        return cwds
    for pid in pids:
        if not pid.isdigit() or pid == me:
            continue
        try:
            target = os.readlink(os.path.join("/proc", pid, "cwd"))
        except OSError:
            continue
        if target.endswith(" (deleted)"):
            target = target[: -len(" (deleted)")]
        cwds.append(target)
    return cwds


def tree_occupied(tree, cwds=None):
    rp = os.path.realpath(tree)
    for cwd in (proc_cwds() if cwds is None else cwds):
        c = os.path.realpath(cwd)
        if c == rp or c.startswith(rp + os.sep):
            return True
    return False


# ------------------------------------------------------------------ run

def evaluate_tree(primary, wt, cfg, now, cwds):
    """Evaluate all gates for one linked worktree. Returns a record."""
    tree = wt["path"]
    rec = {
        "tree": tree, "primary": primary, "branch": wt["branch"],
        "tip": wt["head"], "gates": {}, "action": "keep",
    }
    gates = rec["gates"]
    if not os.path.isdir(tree):
        rec["action"] = "stale-registration"
        gates["exists"] = False
        return rec
    deny = is_denied(tree, cfg, os.path.realpath(__file__))
    gates["denied"] = deny
    if deny:
        rec["reason"] = f"denied:{deny}"
        return rec
    if wt["locked"]:
        rec["reason"] = "locked"
        return rec
    if wt["detached"] or not wt["branch"]:
        gates["pushed"] = {"ok": False, "method": "detached-v1"}
        rec["reason"] = "detached"
        return rec
    ok, method = pushed_proof(primary, wt["head"], wt["branch"])
    gates["pushed"] = {"ok": ok, "method": method}
    if not ok:
        rec["reason"] = "unpushed"
        return rec
    try:
        tracked, untracked = status_entries(tree)
    except RuntimeError as exc:
        gates["clean"] = {"ok": False, "error": str(exc)}
        rec["reason"] = "status-error"
        return rec
    allow, other = dirt_allowlisted(untracked, cfg)
    # Clean = no tracked changes and no untracked files beyond the
    # allowlisted generated dirt (allowlisted-only dirt reaps via --force).
    gates["clean"] = {
        "ok": not tracked and not other,
        "tracked": tracked[:20], "untracked_other": other[:20],
        "allowlisted": allow[:20],
        "allowlisted_only": bool(allow) and not tracked and not other,
    }
    if not gates["clean"]["ok"]:
        rec["reason"] = "dirty"
        return rec
    cutoff = now - cfg["quiet_hours"] * 3600
    try:
        mtime, size, _complete = walk_activity(tree, cutoff)
    except OSError as exc:
        gates["quiet"] = {"ok": False, "error": str(exc)}
        rec["reason"] = "walk-error"
        return rec
    activity = max(mtime, float(head_reflog_ts(tree)))
    age_h = (now - activity) / 3600.0
    gates["quiet"] = {"ok": activity <= cutoff, "last_activity": int(activity),
                      "age_hours": round(age_h, 2), "size_bytes": size}
    rec["size_bytes"] = size
    if not gates["quiet"]["ok"]:
        rec["reason"] = "active"
        return rec
    occupied = tree_occupied(tree, cwds)
    gates["occupied"] = {"ok": not occupied}
    if occupied:
        rec["reason"] = "occupied"
        return rec
    rec["action"] = "candidate"
    return rec


def reap_tree(primary, rec, cfg):
    """Execute the removal for a candidate record (mutates rec)."""
    tree, branch = rec["tree"], rec["branch"]
    # Re-verify in the same run, immediately before the irreversible step:
    # fresh /proc scan and a fresh dirt check (spec: --force only when the
    # sole blocker is allowlisted dirt, re-verified in the same run).
    if tree_occupied(tree):
        rec["action"] = "keep"
        rec["reason"] = "occupied-at-removal"
        return False
    try:
        tracked, untracked = status_entries(tree)
    except RuntimeError as exc:
        rec["action"] = "keep"
        rec["reason"] = f"status-error-at-removal: {exc}"
        return False
    allow, other = dirt_allowlisted(untracked, cfg)
    if tracked or other:
        rec["action"] = "keep"
        rec["reason"] = "dirty-at-removal"
        return False
    force = bool(allow)
    args = ["worktree", "remove"] + (["--force"] if force else []) + [tree]
    rc, _, err = run_git(primary, *args)
    if rc != 0:
        rec["action"] = "remove-failed"
        rec["reason"] = err.strip()[:300]
        return False
    rec["action"] = "reaped"
    rec["forced"] = force
    rc, _, err = run_git(primary, "branch", "-d", branch)
    if rc == 0:
        rec["branch_deleted"] = True
    else:
        rec["branch_deleted"] = False
        rec["branch_keep_reason"] = err.strip()[:300]
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description="Retire dead fleet worktrees (Drew's 6-hour rule).")
    ap.add_argument("--dry-run", action="store_true",
                    help="evaluate and print the would-reap list; remove nothing")
    ap.add_argument("--config", help="path to config.json")
    ap.add_argument("--state-dir", help="override state dir (logs, kill switch)")
    ap.add_argument("--primary", action="append", default=[],
                    help="restrict to this primary (repeatable)")
    ap.add_argument("--no-fetch", action="store_true",
                    help="skip the origin fetch (tests; pushed gate then uses stale refs)")
    ap.add_argument("--quiet-hours", type=float, default=None)
    ap.add_argument("--max-removals", type=int, default=None)
    args = ap.parse_args(argv)

    script_dir = os.path.dirname(os.path.abspath(__file__))
    cfg = load_config(script_dir, args)
    state_dir = cfg["state_dir"]

    if os.path.exists(os.path.join(state_dir, "DISABLED")):
        print("worktree-reaper: DISABLED kill switch present; doing nothing.")
        return 0

    os.makedirs(os.path.join(state_dir, "runs"), exist_ok=True)
    now = time.time()
    ts = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(int(now)))
    records = []
    summary = {
        "run": ts, "dry_run": bool(args.dry_run), "evaluated": 0, "candidates": 0,
        "reaped": 0, "reaped_bytes": 0, "branch_deleted": 0, "branch_kept": 0,
        "kept": {}, "errors": [],
    }

    primaries = [expand(p) for p in args.primary] or discover_primaries(cfg)
    cwds = proc_cwds()
    reaped_count = 0
    cap = int(cfg["max_removals"])

    for primary in primaries:
        if not os.path.isdir(os.path.join(primary, ".git")):
            summary["errors"].append(f"primary missing or not a checkout: {primary}")
            continue
        if cfg.get("fetch"):
            ok, err = fetch_primary(primary)
            if not ok:
                summary["errors"].append(f"fetch failed in {primary}: {err}")
                continue  # fail closed: stale origin refs prove nothing
        try:
            worktrees = list_worktrees(primary)
        except RuntimeError as exc:
            summary["errors"].append(str(exc))
            continue
        primary_real = os.path.realpath(primary)
        removed_any = False
        for wt in worktrees:
            if wt["bare"] or os.path.realpath(wt["path"]) == primary_real:
                continue  # primaries are never reaped
            summary["evaluated"] += 1
            rec = evaluate_tree(primary, wt, cfg, now, cwds)
            if rec["action"] == "candidate":
                summary["candidates"] += 1
                if args.dry_run:
                    rec["action"] = "would-reap"
                elif reaped_count >= cap:
                    rec["action"] = "deferred-cap"
                elif reap_tree(primary, rec, cfg):
                    reaped_count += 1
                    removed_any = True
                    summary["reaped"] += 1
                    summary["reaped_bytes"] += rec.get("size_bytes", 0)
                    if rec.get("branch_deleted"):
                        summary["branch_deleted"] += 1
                    else:
                        summary["branch_kept"] += 1
            if rec["action"] not in ("reaped", "would-reap"):
                reason = rec.get("reason", rec["action"])
                summary["kept"][reason] = summary["kept"].get(reason, 0) + 1
            records.append(rec)
            verb = {"reaped": "REAP", "would-reap": "WOULD-REAP"}.get(rec["action"])
            if verb:
                print(f"{verb} {rec['tree']} branch={rec['branch']} "
                      f"size={rec.get('size_bytes', 0)} pushed={rec['gates'].get('pushed', {}).get('method')}")
        if removed_any and not args.dry_run:
            run_git(primary, "worktree", "prune")

    summary["kept_total"] = sum(summary["kept"].values())
    log_path = os.path.join(state_dir, "runs", f"run-{ts}-{os.getpid()}.jsonl")
    try:
        with open(log_path, "w") as fh:
            for rec in records:
                fh.write(json.dumps({"type": "decision", **rec}) + "\n")
            fh.write(json.dumps({"type": "summary", **summary}) + "\n")
        with open(os.path.join(state_dir, "last-run.json"), "w") as fh:
            json.dump(summary, fh, indent=2)
    except OSError as exc:
        print(f"worktree-reaper: could not write log: {exc}", file=sys.stderr)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
