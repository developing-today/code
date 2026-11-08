#!/usr/bin/env python3
"""ghb_common.py — shared helpers for the github-backup system.

Canonical home: developing-today/code, infra/github-backup/.
Config lives in ../config.json next to this file's parent directory.
The backup tree (default ~/backups/github) holds only data + generated
state (plan.json, MANIFEST.json, logs); all logic lives in the repo.
"""

import contextlib, json, os, shutil, subprocess, sys, tempfile

try:
    import fcntl
except ImportError:  # non-Unix: the lock helper degrades to a no-op
    fcntl = None

HERE = os.path.dirname(os.path.abspath(__file__))
GB_DIR = os.path.dirname(HERE)
CONFIG_PATH = os.path.join(GB_DIR, "config.json")


def load_config():
    with open(CONFIG_PATH) as f:
        return json.load(f)


def backup_root(cfg=None):
    cfg = cfg or load_config()
    return os.path.expanduser(cfg.get("backup_root", "~/backups/github"))


def mirror_path(root, repo):
    return os.path.join(root, repo + ".git")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def valid_mirror(path):
    """A mirror is valid only if it can prove it holds history (#3238).

    Design #3282 rule 1 — validity is resolvability: the destination
    must be a bare repository AND its HEAD must resolve to a commit
    (`rev-parse --verify -q HEAD`, the same call manifest.head_sha
    makes). Bareness alone is true from the moment `git clone
    --mirror` creates the destination skeleton, before any ref or
    object has transferred, so a clone killed in that window used to
    pass everywhere as a verified mirror while holding nothing.

    There is deliberately no emptiness exception: a planned repo is
    never legitimately empty — build_plan excludes zero-commit repos
    under the live config (skip_zero_commit_repos) — so an
    unresolvable HEAD inside the plan is always a skeleton, never a
    valid empty mirror.
    """
    if not os.path.isdir(path):
        return False
    r = run(["git", "-C", path, "rev-parse", "--is-bare-repository"])
    if r.returncode != 0 or r.stdout.strip() != "true":
        return False
    h = run(["git", "-C", path, "rev-parse", "--verify", "-q", "HEAD"])
    return h.returncode == 0 and bool(h.stdout.strip())


def is_skeleton_mirror(path):
    """True for a bare skeleton: bare, HEAD unresolvable, zero refs.

    This is the destination state a `clone --mirror` creates before
    transport and a killed clone leaves behind (#3238). By
    construction it holds no refs and no reachable objects, so
    removing it loses nothing — design #3282 rule 2's skeleton
    disposition (remove and clone). A bare repository WITH refs is
    never a skeleton, whatever its HEAD state: it may hold the only
    copy of transferred objects and must be preserved.
    """
    if not os.path.isdir(path):
        return False
    r = run(["git", "-C", path, "rev-parse", "--is-bare-repository"])
    if r.returncode != 0 or r.stdout.strip() != "true":
        return False
    h = run(["git", "-C", path, "rev-parse", "--verify", "-q", "HEAD"])
    if h.returncode == 0:
        return False
    refs = run(["git", "-C", path, "for-each-ref"])
    return refs.returncode == 0 and not refs.stdout.strip()


def disk_kb(path):
    r = run(["du", "-sk", path])
    try:
        return int(r.stdout.split()[0])
    except Exception:
        return -1


# ---------------------------------------------------------------- inventory
_GQL = """
query($login: String!, $after: String) {
  repositoryOwner(login: $login) {
    repositories(first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes {
        nameWithOwner isFork isArchived diskUsage pushedAt
        defaultBranchRef { name target { oid } }
        parent { nameWithOwner }
      }
    }
  }
}
"""


def inventory_owner(owner):
    """All repos visible to the gh token under one owner, via GraphQL."""
    repos = []
    after = None
    while True:
        variables = {"login": owner}
        if after:
            variables["after"] = after
        r = run(
            ["gh", "api", "graphql", "-f", f"query={_GQL}", "-f", f"login={owner}"]
            + (["-f", f"after={after}"] if after else [])
        )
        if r.returncode != 0:
            raise RuntimeError(f"gh api graphql failed for {owner}: {r.stderr[:300]}")
        data = json.loads(r.stdout)
        # A successful gh exit can still carry a failed inventory: a
        # top-level GraphQL errors field, or a null repositoryOwner when
        # the owner is renamed / not visible to the token (e.g. SAML
        # enforcement). Neither may be read as "owner has no repos" —
        # that would silently un-plan the owner's mirrors in reconcile
        # (#3229). Raise so inventory() records an owner-level error.
        if data.get("errors"):
            raise RuntimeError(
                f"gh api graphql errors for {owner}: {json.dumps(data['errors'])[:300]}"
            )
        owner_obj = (data.get("data") or {}).get("repositoryOwner")
        if owner_obj is None:
            raise RuntimeError(
                f"gh api graphql for {owner}: repositoryOwner is null — "
                f"owner not visible to the token (renamed, SAML-blocked, "
                f"or no access); refusing to read as an empty owner"
            )
        conn = owner_obj.get("repositories")
        if conn is None:
            raise RuntimeError(
                f"gh api graphql for {owner}: repositories connection is "
                f"null; refusing to read as an empty owner"
            )
        if conn.get("nodes") is None or conn.get("pageInfo") is None:
            raise RuntimeError(
                f"gh api graphql for {owner}: malformed repositories "
                f"connection (nodes/pageInfo missing)"
            )
        for n in conn["nodes"]:
            repos.append(
                {
                    "repo": n["nameWithOwner"],
                    "fork": bool(n["isFork"]),
                    "archived": bool(n["isArchived"]),
                    "size_kb": n.get("diskUsage") or 0,
                    "parent": (n.get("parent") or {}).get("nameWithOwner"),
                    "branch": (n.get("defaultBranchRef") or {}).get("name"),
                    "head": ((n.get("defaultBranchRef") or {}).get("target") or {}).get(
                        "oid"
                    ),
                    "pushed_at": n.get("pushedAt"),
                    "empty": n.get("defaultBranchRef") is None,
                }
            )
        if conn["pageInfo"]["hasNextPage"]:
            after = conn["pageInfo"]["endCursor"]
        else:
            return repos


def inventory(owners):
    out = {}
    errors = {}
    for o in owners:
        try:
            for repo in inventory_owner(o):
                out[repo["repo"]] = repo
        except Exception as e:
            errors[o] = str(e)[:300]
    return out, errors


# ------------------------------------------- generated-state IO (#3240)
# Design #3282 rule 6: generated state (plan.json, state.json,
# MANIFEST.json, and the markdown reports built from them) is written
# atomically and loaded tolerantly, only through these helpers. A
# writer killed mid-dump (laptop sleep, reboot, driver timeout) must
# never leave a torn live file, and a pre-existing torn file must
# never wedge every reader at startup with a raw JSONDecodeError.

LAST_GOOD_SUFFIX = ".last-good"


def _atomic_replace(path, text):
    """Write `text` to `path` atomically: temp file in the SAME
    directory, fsync, os.replace, directory fsync (best effort). A
    reader only ever observes the previous complete file or the new
    complete file — never a partial one."""
    directory = os.path.dirname(os.path.abspath(path))
    fd, tmp = tempfile.mkstemp(
        dir=directory, prefix=".ghb-", suffix=".tmp"
    )
    try:
        with os.fdopen(fd, "w") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        with contextlib.suppress(OSError):
            os.unlink(tmp)
        raise
    with contextlib.suppress(OSError):
        dir_fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(dir_fd)
        finally:
            os.close(dir_fd)


def atomic_write_text(path, text):
    """Atomically replace `path` with `text`, keeping the previous
    complete version at `<path>.last-good` for the tolerant loader."""
    if os.path.exists(path):
        with contextlib.suppress(OSError):
            with open(path) as f:
                previous = f.read()
            _atomic_replace(path + LAST_GOOD_SUFFIX, previous)
    _atomic_replace(path, text)


def save_json(path, obj, **dump_kw):
    """Serialize + atomically write a generated-state JSON file."""
    dump_kw.setdefault("indent", 0)
    atomic_write_text(path, json.dumps(obj, **dump_kw))


def load_json_tolerant(path, default, source):
    """Load a generated-state JSON file tolerantly (#3240).

    Returns (value, problems). Missing file = (default, []). A torn
    or unreadable live file falls back to the `<path>.last-good`
    copy kept by atomic_write_text; if that also fails, (default,
    [problem]). Every fallback is a loud, named problem — corruption
    is surfaced by the callers' report_problems, never silent and
    never a startup traceback.
    """
    if not os.path.exists(path):
        return default, []
    try:
        with open(path) as f:
            return json.load(f), []
    except (OSError, ValueError) as e:
        problems = [
            f"{source}: cannot load {os.path.basename(path)} "
            f"({type(e).__name__}: {e}) — trying last-good copy"
        ]
    backup = path + LAST_GOOD_SUFFIX
    if os.path.exists(backup):
        try:
            with open(backup) as f:
                value = json.load(f)
            problems.append(
                f"{source}: {os.path.basename(path)} is torn; fell "
                f"back to the last-good copy — the live file needs "
                f"repair on the next successful write"
            )
            return value, problems
        except (OSError, ValueError) as e:
            problems.append(
                f"{source}: last-good copy also unusable "
                f"({type(e).__name__}: {e})"
            )
    problems.append(
        f"{source}: {os.path.basename(path)} treated as empty"
    )
    return default, problems


@contextlib.contextmanager
def state_lock(root):
    """Shared flock for state.json read-modify-write (#3240).

    The daily fetch and the 5-minute drainer both load state.json,
    modify it, and write it back; without a lock, one writer's
    read-modify-write window silently drops the other's updates.
    Hold this around the whole load -> modify -> save cycle. The
    lock lives on a dedicated lockfile (never replaced), and
    degrades to a no-op where fcntl is unavailable.
    """
    if fcntl is None:
        yield
        return
    os.makedirs(root, exist_ok=True)
    fd = os.open(os.path.join(root, "state.lock"), os.O_CREAT | os.O_RDWR, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        with contextlib.suppress(OSError):
            fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)


# ------------------------------------------------------- plan/state files
def _shape(value):
    return type(value).__name__


# Plan fields every consumer treats as a repo name / branch name (or
# null): clone_all hashes "reference" into its scheduler state and
# passes "branch" to manifest's head_sha string concatenation;
# manifest groups families by "reference", sorts the anchors, and
# copies "parent" into MANIFEST.json. All three must be strings or
# null — any other JSON shape is poison (#6834).
_PLAN_STRING_FIELDS = ("reference", "branch", "parent")


def validate_plan(plan, source="plan.json"):
    """Split a loaded plan.json into (valid entries, named problems).

    A plan entry is valid only as an object with a string "repo" —
    every consumer subscripts entry["repo"], so any other shape is a
    poison entry (#6637): it is reported by index and skipped, never
    allowed to crash the run for every other repo. A non-list top
    level invalidates the whole file as a single named problem.

    The consumed string fields ("reference", "branch", "parent")
    are validated per entry (#6834): a value that is neither a string
    nor null/absent is reported by entry and field and neutralized
    to None — a list/dict reference is unhashable in clone_all's
    scheduler and manifest's family grouping, a numeric reference
    crashes manifest's mixed-type anchor sort (or writes a bogus
    FAMILIES anchor), and a non-string branch crashes head_sha for a
    present mirror. The entry itself is kept: its repo is fine, only
    the poisoned field is dropped. Entries are copied, never mutated
    in place.
    """
    if not isinstance(plan, list):
        return [], [
            f"{source}: top level is {_shape(plan)}, expected a list of "
            f"objects — whole file treated as empty"
        ]
    entries, problems = [], []
    for i, item in enumerate(plan):
        if not (isinstance(item, dict) and isinstance(item.get("repo"), str)):
            problems.append(
                f"{source}: entry {i} is not an object with a string "
                f"'repo' (got {_shape(item)}) — skipped"
            )
            continue
        entry = dict(item)
        for field in _PLAN_STRING_FIELDS:
            value = entry.get(field)
            if value is not None and not isinstance(value, str):
                problems.append(
                    f"{source}: entry {i} ({entry['repo']}) field "
                    f"'{field}' is {_shape(value)}, expected a string "
                    f"or null — set to null"
                )
                entry[field] = None
        entries.append(entry)
    return entries, problems


def validate_state(state, source="state.json"):
    """Split a loaded state.json into (clean state, named problems).

    Per-repo entries must be objects; a wrong-shape entry is dropped
    by name (#6637) so one poison entry cannot wedge every repo, and
    a non-list "fail_days" inside an entry is reset to [] under the
    same rule. A non-object top level invalidates the whole file as
    a single named problem.

    Bookkeeping keys written by the webhook drainer are state too,
    under their own shapes: "drain_last::<repo>" is a numeric
    timestamp and "drain_fail::<repo>" is an object. They are
    validated under those shapes and preserved — dropping them
    would erase the drainer's coalescing window every daily fetch.
    """
    if not isinstance(state, dict):
        return {}, [
            f"{source}: top level is {_shape(state)}, expected an "
            f"object — whole file treated as empty"
        ]
    clean, problems = {}, []
    for repo, entry in state.items():
        if repo.startswith("drain_last::"):
            if isinstance(entry, (int, float)) and not isinstance(entry, bool):
                clean[repo] = entry
            else:
                problems.append(
                    f"{source}: entry for {repo!r} is {_shape(entry)}, "
                    f"expected a number — dropped"
                )
            continue
        if not isinstance(entry, dict):
            problems.append(
                f"{source}: entry for {repo!r} is {_shape(entry)}, "
                f"expected an object — dropped"
            )
            continue
        entry = dict(entry)
        days = entry.get("fail_days")
        if days is not None and not isinstance(days, list):
            problems.append(
                f"{source}: 'fail_days' for {repo!r} is {_shape(days)}, "
                f"expected a list — reset to []"
            )
            entry["fail_days"] = []
        clean[repo] = entry
    return clean, problems


def load_plan(root):
    """Load + validate plan.json under `root` (missing file = empty).

    Tolerant (#3240): a torn plan.json falls back to its last-good
    copy (or empty) with named problems, never a JSONDecodeError at
    startup."""
    path = os.path.join(root, "plan.json")
    raw, problems = load_json_tolerant(path, [], "plan.json")
    entries, shape_problems = validate_plan(raw)
    return entries, problems + shape_problems


def load_state(root):
    """Load + validate state.json under `root` (missing file = empty).

    Tolerant (#3240), same contract as load_plan."""
    path = os.path.join(root, "state.json")
    raw, problems = load_json_tolerant(path, {}, "state.json")
    clean, shape_problems = validate_state(raw)
    return clean, problems + shape_problems


def clone_status_record(line):
    """The clone-status.jsonl line contract — the ONE place it is stated
    (#6606). Both readers of clone-status.jsonl (clone_all.py and
    reconcile.py) go through this helper, so they cannot disagree:

    each line is a JSON *object* (a status record like
    {"repo": ..., "status": ...}). A line that does not parse as JSON,
    and a line that parses as valid JSON of any other shape (number,
    string, list, null, bool — a torn/interleaved append, a hand edit,
    a future writer change), is NOT a record: this returns None and
    the reader ignores the line exactly as it ignores unparseable
    lines. Records are returned as-is; missing keys are the caller's
    existing concern, and a caller that puts a field value into a set
    must require the field's expected type (a record can still carry
    a malformed field value, e.g. a list "repo" — both current
    consumers require a string repo).
    """
    try:
        value = json.loads(line)
    except Exception:
        return None
    return value if isinstance(value, dict) else None


def report_problems(problems):
    """Name every shape problem on stderr; returns the count."""
    for problem in problems:
        print(f"github-backup: {problem}", file=sys.stderr)
    return len(problems)


# ---------------------------------------------------------------- plan
def build_plan(repos, cfg):
    """Apply config tier rules. Returns (plan, excluded, gaps, pending_review).

    plan: ordered list of {repo, reference|null, size_kb, fork, parent,
    branch, head} — dependency order (anchors/parents before members).
    """
    rules = cfg["tier_rules"]
    cap_kb = int(rules.get("fork_max_size_mb", 500)) * 1024
    force = set(rules.get("force_include", []))
    excl = {e["repo"]: e["reason"] for e in cfg.get("exclusions", [])}
    gaps = {g["repo"]: g["reason"] for g in cfg.get("gaps", [])}
    included, excluded, gap, pending, skipped_empty = {}, [], [], [], []
    for name, v in sorted(repos.items()):
        if name in gaps or (name.split("/")[0] + "/*") in gaps:
            gap.append(
                {
                    "repo": name,
                    "reason": gaps.get(name) or gaps.get(name.split("/")[0] + "/*"),
                }
            )
            continue
        if name in excl:
            excluded.append({"repo": name, "reason": excl[name]})
            continue
        if rules.get("skip_zero_commit_repos") and v.get("empty"):
            skipped_empty.append({"repo": name, "reason": "zero commits (empty repo)"})
            continue
        if (
            (not v["fork"] and rules.get("include_nonforks"))
            or name in force
            or v["size_kb"] < cap_kb
        ):
            included[name] = v
        else:
            pending.append(
                {
                    "repo": name,
                    "reason": f"fork {v['size_kb'] / 1024:.0f} MB >= cap "
                    f"{rules.get('fork_max_size_mb')} MB — pending Drew review",
                }
            )
    incset = set(included)

    def depth(k):
        n, cur, guard = 0, k, set()
        while True:
            p = included.get(cur, {}).get("parent")
            if p and p in incset and p not in guard:
                guard.add(p)
                n += 1
                cur = p
            else:
                return n

    plan, anchor_for = [], {}
    for k in sorted(included, key=lambda k: (depth(k), k)):
        v = included[k]
        ref = None
        if v["fork"] and v.get("parent"):
            p = v["parent"]
            if p in incset:
                ref = p
            elif p in anchor_for:
                ref = anchor_for[p]
            else:
                anchor_for[p] = k
        plan.append(
            {
                "repo": k,
                "reference": ref,
                "size_kb": v["size_kb"],
                "fork": v["fork"],
                "parent": v.get("parent"),
                "branch": v.get("branch"),
                "head": v.get("head"),
                "private": None,
            }
        )
    return plan, excluded, gap, pending, skipped_empty
