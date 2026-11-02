#!/usr/bin/env python3
"""ghb_common.py — shared helpers for the github-backup system.

Canonical home: developing-today/code, infra/github-backup/.
Config lives in ../config.json next to this file's parent directory.
The backup tree (default ~/backups/github) holds only data + generated
state (plan.json, MANIFEST.json, logs); all logic lives in the repo.
"""

import json, os, subprocess, sys

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
    if not os.path.isdir(path):
        return False
    r = run(["git", "-C", path, "rev-parse", "--is-bare-repository"])
    return r.returncode == 0 and r.stdout.strip() == "true"


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
