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
               Ignored files are gated too (#6734): the PUSHED proof
               cannot cover content git ignores, so any ignored path
               beyond the ignored-dirt allowlist (config:
               ignored_dirt_prefixes, e.g. target/ and node_modules/
               build output, and ignored_dirt_files) blocks the reap —
               fail closed, at this gate and at the pre-removal
               re-verification. Every ignored path present in a reaped
               tree is recorded by path in the run record
               (ignored_paths) before removal: a reap never silently
               deletes unlisted content.
  3. QUIET     last activity >= quiet_hours ago (absolute epoch). Activity
               = newest working-tree file mtime excluding the tree's root
               .git entry, max()ed with the HEAD reflog timestamp.
  4. UNOCCUPIED a fresh /proc cwd scan shows no process inside the tree
               (re-scanned immediately before each removal).
  5. NOT DENIED the tree is not on the config denylist, is not a primary,
               is not under ~/backups or ~/atlas, and does not contain
               this running script.

Runs are single-instance (#6899): main() takes an flock on
<state_dir>/reaper.lock before evaluating anything; a second
concurrent run exits without evaluating or removing anything.
Immediately before a removal, reap_tree() re-verifies ALL of the
gates that can change after evaluation — occupancy, CLEAN, QUIET,
and PUSHED — so a commit landing during the run (clean tree, tip
now unpushed and active) retains the tree.

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
import fcntl
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
    "ignored_dirt_prefixes": ["target/", "node_modules/"],
    "ignored_dirt_files": [],
    "fetch": True,
}

HARD_DENY_PREFIXES = ("~/backups", "~/atlas")

LOCK_FILENAME = "reaper.lock"


def acquire_instance_lock(state_dir):
    """Take the single-instance flock in the state dir (#6899).

    Returns (fd, path) on success — the caller keeps fd open for the
    whole run; the flock releases when fd closes. Returns (None, path)
    when another live instance already holds the lock: overlapping
    runs (a manual live run racing a timer fire) would otherwise
    evaluate the same trees concurrently and race the state writes
    and removals. A stale lock FILE is harmless: flock is released
    when the holder exits, however it exits.
    """
    path = os.path.join(state_dir, LOCK_FILENAME)
    fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        return None, path
    os.ftruncate(fd, 0)
    os.write(fd, f"{os.getpid()}\n".encode())
    return fd, path


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


class ConfigError(Exception):
    """A config file is unreadable, malformed, or fails validation.

    The message always names the file. `main` turns this into the
    clean `worktree-reaper: cannot load config ...` error instead of
    a traceback (#6394).
    """


def _load_json_object(path):
    """Read `path` as JSON and require a top-level object (#6394)."""
    try:
        with open(path) as fh:
            data = json.load(fh)
    except OSError as exc:
        raise ConfigError(f"{path}: cannot read file: {exc}") from exc
    except json.JSONDecodeError as exc:
        raise ConfigError(f"{path}: invalid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise ConfigError(
            f"{path}: top level must be a JSON object, "
            f"got {type(data).__name__}"
        )
    return data


def validate_config(data, path):
    """Validate one config file's contents against DEFAULT_CONFIG (#6394).

    Unknown keys are refused: a typo'd `deny_*` key must never load
    silently, because for a deletion tool that turns an explicit
    "never reap this" into no protection at all. Types are taken
    from the defaults: strings stay strings, the list keys stay
    lists of strings, `fetch` stays a bool, and the numeric keys
    stay numbers (never bools, never strings that explode mid-run).
    """
    for key, val in data.items():
        if key not in DEFAULT_CONFIG:
            raise ConfigError(
                f"{path}: unknown key {key!r} "
                f"(valid keys: {', '.join(sorted(DEFAULT_CONFIG))})"
            )
        default = DEFAULT_CONFIG[key]
        if isinstance(default, bool):
            ok = isinstance(val, bool)
            want = "a boolean"
        elif isinstance(default, list):
            ok = isinstance(val, list) and all(
                isinstance(item, str) for item in val
            )
            want = "a list of strings"
        elif isinstance(default, str):
            ok = isinstance(val, str)
            want = "a string"
        else:  # numeric defaults: quiet_hours, max_removals
            ok = isinstance(val, (int, float)) and not isinstance(val, bool)
            want = "a number"
        if not ok:
            raise ConfigError(
                f"{path}: key {key!r} must be {want}, got {val!r}"
            )
        if key in ("quiet_hours", "max_removals") and val < 0:
            raise ConfigError(f"{path}: key {key!r} must be >= 0, got {val!r}")


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
        data = _load_json_object(cfg_path)
        validate_config(data, cfg_path)
        cfg.update(data)
        cfg["_config_path"] = os.path.abspath(cfg_path)
    state_dir = expand(args.state_dir or cfg["state_dir"])
    overlay = os.path.join(state_dir, "config.local.json")
    if os.path.isfile(overlay):
        local = _load_json_object(overlay)
        validate_config(local, overlay)
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
    """Return (tracked_dirty, untracked, ignored) repo-relative path lists.

    Ignored entries (porcelain `!!`, via --ignored=matching) are
    enumerated explicitly (#6734): without the flag they are invisible
    to every gate, yet `git worktree remove` deletes them with the
    tree. With =matching, git lists an ignored directory itself (path
    ending in /) when the directory matches an ignore rule, and
    individual ignored files otherwise.
    """
    rc, out, err = run_git(
        tree, "status", "--porcelain=v1", "-z", "--no-renames",
        "--untracked-files=all", "--ignored=matching",
    )
    if rc != 0:
        raise RuntimeError(f"status failed in {tree}: {err.strip()}")
    tracked, untracked, ignored = [], [], []
    for token in out.split(b"\0"):
        if len(token) < 4:
            continue
        xy = token[:2].decode("ascii", "replace")
        path = token[3:].decode("utf-8", "surrogateescape")
        if xy == "??":
            untracked.append(path)
        elif xy == "!!":
            ignored.append(path)
        else:
            tracked.append(path)
    return tracked, untracked, ignored


def ignored_allowlisted(ignored, cfg):
    """Split ignored paths into (allowlisted, other) per the config's
    ignored-dirt lists. Prefixes match on whole path segments (target/
    matches target/ and pkgs/id/target/ alike): build output lives at
    crate roots, not only at the tree root. Files match on basename,
    like the generated-dirt allowlist."""
    prefixes = [p.strip("/") for p in cfg.get("ignored_dirt_prefixes", [])]
    names = set(cfg.get("ignored_dirt_files", []))
    allow, other = [], []
    for path in ignored:
        segments = [seg for seg in path.split("/") if seg]
        if any(seg in prefixes for seg in segments) or os.path.basename(path.rstrip("/")) in names:
            allow.append(path)
        else:
            other.append(path)
    return allow, other


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


class ProcScanError(OSError):
    """The /proc scan itself failed (#5740).

    Distinct from a successful scan that finds no processes: occupancy
    is UNKNOWN, and every caller must fail closed (retain the tree).
    Subclasses OSError so generic OS-error handlers still see it as
    an inspection failure, never as an empty result.
    """


def _strip_deleted(target):
    if target.endswith(" (deleted)"):
        target = target[: -len(" (deleted)")]
    return target


def proc_refs():
    """All filesystem paths any process is tied to, from /proc (#6898).

    For each pid: its cwd, its exe, every open fd target, and every
    absolute-path token in its cmdline. The cwd-only scan this replaces
    could not see a process holding an open file in a tree (or running
    a binary from it) with its cwd elsewhere, so the reaper could
    delete a tree in active use. Cost is bounded by the number of
    processes plus their open fds (one readlink per fd), the same
    order as any /proc scan. Per-pid errors (the process exited
    mid-scan, or its entries are not ours to inspect) skip that entry,
    exactly as the cwd scan did; only failing to list /proc itself
    raises ProcScanError (fail closed, #5740).
    """
    refs = []
    me = str(os.getpid())
    try:
        pids = os.listdir("/proc")
    except OSError as exc:
        raise ProcScanError(f"cannot list /proc: {exc}") from exc
    for pid in pids:
        if not pid.isdigit() or pid == me:
            continue
        base = os.path.join("/proc", pid)
        for name in ("cwd", "exe"):
            try:
                target = os.readlink(os.path.join(base, name))
            except OSError:
                continue
            target = _strip_deleted(target)
            if target.startswith("/"):
                refs.append(target)
        try:
            fds = os.listdir(os.path.join(base, "fd"))
        except OSError:
            fds = []
        for fd in fds:
            try:
                target = os.readlink(os.path.join(base, "fd", fd))
            except OSError:
                continue
            target = _strip_deleted(target)
            if target.startswith("/"):
                refs.append(target)
        try:
            with open(os.path.join(base, "cmdline"), "rb") as fh:
                tokens = fh.read().split(b"\0")
        except OSError:
            tokens = []
        for token in tokens:
            path = token.decode("utf-8", "surrogateescape")
            if path.startswith("/"):
                refs.append(path)
    return refs


def proc_cwds():
    """Backward-compatible name: the scan now returns every /proc path
    reference (cwd, exe, fd targets, cmdline paths), not only cwds."""
    return proc_refs()


def tree_occupied(tree, cwds=None):
    rp = os.path.realpath(tree)
    for ref in (proc_refs() if cwds is None else cwds):
        c = os.path.realpath(ref)
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
        tracked, untracked, ignored = status_entries(tree)
    except RuntimeError as exc:
        gates["clean"] = {"ok": False, "error": str(exc)}
        rec["reason"] = "status-error"
        return rec
    allow, other = dirt_allowlisted(untracked, cfg)
    ig_allow, ig_other = ignored_allowlisted(ignored, cfg)
    # Clean = no tracked changes and no untracked files beyond the
    # allowlisted generated dirt (allowlisted-only dirt reaps via --force),
    # and no ignored files beyond the ignored-dirt allowlist (#6734:
    # ignored content is exactly what the PUSHED proof cannot cover).
    gates["clean"] = {
        "ok": not tracked and not other and not ig_other,
        "tracked": tracked[:20], "untracked_other": other[:20],
        "allowlisted": allow[:20],
        "ignored_other": ig_other[:20], "ignored_allowlisted": ig_allow[:20],
        "ignored": ignored,
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
    try:
        occupied_now = tree_occupied(tree)
    except ProcScanError as exc:
        # Fail closed (#5740): occupancy unknown — retain the tree and
        # say so; never let a scan error read as "unoccupied".
        rec["action"] = "keep"
        rec["reason"] = "occupied-unknown-at-removal"
        rec["error"] = str(exc)
        print(f"worktree-reaper: WARNING: /proc scan failed before "
              f"removal of {tree}; retaining it ({exc})")
        return False
    if occupied_now:
        rec["action"] = "keep"
        rec["reason"] = "occupied-at-removal"
        return False
    try:
        tracked, untracked, ignored = status_entries(tree)
    except RuntimeError as exc:
        rec["action"] = "keep"
        rec["reason"] = f"status-error-at-removal: {exc}"
        return False
    allow, other = dirt_allowlisted(untracked, cfg)
    _ig_allow, ig_other = ignored_allowlisted(ignored, cfg)
    if tracked or other or ig_other:
        rec["action"] = "keep"
        rec["reason"] = "dirty-at-removal"
        return False
    # Re-verify PUSHED and QUIET (#6899): both can change after
    # evaluation while the tree stays clean — a commit landing
    # during the run moves the tip (now unpushed) and refreshes
    # activity. Retain the tree if either gate no longer holds.
    rc, out, _err = run_git(tree, "rev-parse", "HEAD")
    current_tip = out.decode().strip() if rc == 0 else None
    if not current_tip:
        rec["action"] = "keep"
        rec["reason"] = "pushed-unknown-at-removal"
        return False
    ok, method = pushed_proof(primary, current_tip, branch)
    rec.setdefault("gates", {})["pushed_at_removal"] = {"ok": ok, "method": method}
    if not ok:
        rec["action"] = "keep"
        rec["reason"] = "unpushed-at-removal"
        return False
    cutoff = time.time() - float(cfg.get("quiet_hours", DEFAULT_CONFIG["quiet_hours"])) * 3600
    try:
        mtime, _size, _complete = walk_activity(tree, cutoff)
    except OSError as exc:
        rec["action"] = "keep"
        rec["reason"] = "quiet-unknown-at-removal"
        rec["error"] = str(exc)
        return False
    activity = max(mtime, float(head_reflog_ts(tree)))
    rec["gates"]["quiet_at_removal"] = {"ok": activity <= cutoff, "last_activity": int(activity)}
    if activity > cutoff:
        rec["action"] = "keep"
        rec["reason"] = "active-at-removal"
        return False
    # Record every ignored path in this tree before the irreversible
    # step (#6734): only allowlisted ignored paths can be present
    # here, and the record is the audit trail for their deletion.
    rec["ignored_paths"] = ignored
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
    try:
        cfg = load_config(script_dir, args)
    except ConfigError as exc:
        print(f"worktree-reaper: cannot load config: {exc}")
        return 2
    state_dir = cfg["state_dir"]

    if os.path.exists(os.path.join(state_dir, "DISABLED")):
        print("worktree-reaper: DISABLED kill switch present; doing nothing.")
        return 0

    os.makedirs(os.path.join(state_dir, "runs"), exist_ok=True)
    lock_fd, lock_path = acquire_instance_lock(state_dir)
    if lock_fd is None:
        print(f"worktree-reaper: another instance already holds the lock "
              f"({lock_path}); exiting without evaluating or removing anything.")
        return 0
    now = time.time()
    ts = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime(int(now)))
    records = []
    summary = {
        "run": ts, "dry_run": bool(args.dry_run), "evaluated": 0, "candidates": 0,
        "reaped": 0, "reaped_bytes": 0, "branch_deleted": 0, "branch_kept": 0,
        "stale_registrations": 0, "pruned": 0,
        "kept": {}, "errors": [],
    }

    primaries = [expand(p) for p in args.primary] or discover_primaries(cfg)
    try:
        cwds = proc_refs()
    except ProcScanError as exc:
        # Fail closed (#5740): with no trustworthy occupancy scan,
        # nothing in this run may be removed — stop before evaluating.
        print(f"worktree-reaper: ERROR: /proc scan failed ({exc}); "
              "failing closed, nothing will be removed this run.")
        summary["errors"].append(f"proc scan failed: {exc}")
        print(json.dumps(summary, indent=2))
        os.close(lock_fd)
        return 1
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
        stale_any = False
        for wt in worktrees:
            if wt["bare"] or os.path.realpath(wt["path"]) == primary_real:
                continue  # primaries are never reaped
            summary["evaluated"] += 1
            rec = evaluate_tree(primary, wt, cfg, now, cwds)
            if rec["action"] == "stale-registration":
                # A registration whose directory is gone is not a kept
                # tree: it is prune work for this primary (#6547). Count
                # it as such, and in a dry run say would-prune instead
                # of folding it into the kept counts.
                stale_any = True
                summary["stale_registrations"] += 1
                if args.dry_run:
                    rec["action"] = "would-prune"
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
            if rec["action"] not in ("reaped", "would-reap", "would-prune",
                                     "stale-registration"):
                reason = rec.get("reason", rec["action"])
                summary["kept"][reason] = summary["kept"].get(reason, 0) + 1
            records.append(rec)
            verb = {"reaped": "REAP", "would-reap": "WOULD-REAP",
                    "would-prune": "WOULD-PRUNE"}.get(rec["action"])
            if verb:
                print(f"{verb} {rec['tree']} branch={rec['branch']} "
                      f"size={rec.get('size_bytes', 0)} pushed={rec['gates'].get('pushed', {}).get('method')}")
        # Prune whenever this run left the primary's registrations
        # stale: a tree was reaped, OR a stale registration was found
        # (#6547 — previously prune ran only as a side effect of a
        # same-run reap, so a deleted-directory-only primary was never
        # pruned). Fail-safe: this block is reached only for primaries
        # successfully inventoried above, and a prune failure is
        # surfaced in the summary, never swallowed.
        if (removed_any or stale_any) and not args.dry_run:
            rc, _, err = run_git(primary, "worktree", "prune")
            if rc != 0:
                summary["errors"].append(
                    f"prune failed in {primary}: {err.strip()}")
            else:
                summary["pruned"] += 1

    summary["kept_total"] = sum(summary["kept"].values())
    # The log name must be unique per run: ts has only second resolution,
    # so same-second runs from one process (the gate suite drives main()
    # in-process) would otherwise share a path and the later run would
    # truncate the earlier run's audit log (#5716). Add the microsecond
    # fraction and create exclusively, suffixing on any residual clash.
    log_stamp = f"{ts}-{int((now - int(now)) * 1_000_000):06d}"
    log_path = os.path.join(state_dir, "runs", f"run-{log_stamp}-{os.getpid()}.jsonl")
    suffix = 2
    while os.path.exists(log_path):
        log_path = os.path.join(
            state_dir, "runs", f"run-{log_stamp}-{os.getpid()}-{suffix}.jsonl")
        suffix += 1
    try:
        with open(log_path, "x") as fh:
            for rec in records:
                fh.write(json.dumps({"type": "decision", **rec}) + "\n")
            fh.write(json.dumps({"type": "summary", **summary}) + "\n")
        with open(os.path.join(state_dir, "last-run.json"), "w") as fh:
            json.dump(summary, fh, indent=2)
    except OSError as exc:
        print(f"worktree-reaper: could not write log: {exc}", file=sys.stderr)
    print(json.dumps(summary, indent=2))
    os.close(lock_fd)
    return 0


if __name__ == "__main__":
    sys.exit(main())
