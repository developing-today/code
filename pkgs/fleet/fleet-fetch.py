#!/usr/bin/env python3
"""fleet-fetch.py — deterministic repo snapshot fetcher (Phase B part 1).

For each fleet repo: pull current state via `gh api` and upsert a row
into agent_repo_snapshots in the fleet-queue Turso DB. One events row
per run (worker_id 'fleet-fetcher', kind 'snapshot_refresh') plus
watermarks in agent_watermarks (same DB). Zero agent tokens: pure
script, run by fleet-fetch.timer.

NO LOCAL CLONES (Drew, 2026-11-02): the sandbox holds no repo copies.
The former clone-warming `git fetch` step was removed; the snapshot
payload always came from `gh api`, and the nix upstream watermark now
reads the nixpkgs-unstable head from `gh api` too.

Writes go through the libSQL HTTP pipeline to fleet-queue (token by
name: turso.fleet-queue-rw-token). Failure of one repo never fails
the others; exit is non-zero only if ALL repos fail.

History: snapshots/watermarks lived in Supabase muse-dev
(agent.repo_snapshots / agent.watermarks) until 2026-11-07, when the
#5693 ci re-home moved them to fleet-queue alongside the github-ci
Worker (which advances head_sha in the same table on push). The
snapshot_refresh event's sink (#5690, AGENT_EVENTS_SINK) had already
flipped to turso; the PG event leg is retired with this repoint —
the event now always lands in the fleet-queue events table.
Cutover: doc/fleet/agent-events-turso.md in developing-today/code.
"""
import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

# Anchor on the real home, NOT $HOME: under systemd HOME=/root, which broke
# gh auth lookup and every path below (first timer fires failed silently;
# interactive runs masked it). /home/hatch is the durable home on this box.
HOME = "/home/hatch"
LOG_FILE = os.path.join(HOME, "logs", "fleet-fetch.log")
SQL_TMPDIR = os.path.join(HOME, "tmp")  # /tmp is a small tmpfs on this box

# Fleet repo slugs. Snapshots are gh-api only — no local clones.
REPOS = [
    "developing-today/code",
    "developing-today-forks/opencode",
    "developing-today-forks/openchamber",
    "dezren39/darwin-nix",
]
NIX_REPO = "dezren39/darwin-nix"
UPSTREAM_WATERMARK_KEY = "upstream:nixpkgs-unstable"
CAP = 100


def now_iso():
    return datetime.now(timezone.utc).isoformat()


def q(value):
    """SQL string literal with '' escaping (NULL for None)."""
    if value is None:
        return "NULL"
    return "'" + str(value).replace("'", "''") + "'"


def jb(obj):
    """SQL jsonb literal."""
    return q(json.dumps(obj)) + "::jsonb"


EVENTS_TURSO_URL = "https://fleet-queue-dezren39.aws-us-east-1.turso.io"


def events_sink():
    """AGENT_EVENTS_SINK for the snapshot_refresh row: pg|dual|turso."""
    return os.environ.get("AGENT_EVENTS_SINK", "pg").strip().lower()


def queue_token():
    """fleet-queue RW token, resolved by name via the secrets CLI,
    never stored here."""
    tok = run(["fleet", "secrets", "get", "turso.fleet-queue-rw-token"],
              timeout=60)
    token = tok.stdout.strip()
    if tok.returncode != 0 or not token:
        raise RuntimeError(
            f"secrets get turso.fleet-queue-rw-token failed: "
            f"{tok.stderr.strip()[:200]}")
    return token


def turso_pipeline(statements):
    """Run SQL statements against fleet-queue via the libSQL HTTP
    pipeline. Raises on any error result."""
    import urllib.request
    body = json.dumps({"requests": [
        *[{"type": "execute", "stmt": {"sql": s}} for s in statements],
        {"type": "close"}]}).encode()
    req = urllib.request.Request(
        EVENTS_TURSO_URL + "/v2/pipeline", data=body, method="POST",
        headers={"Authorization": f"Bearer {queue_token()}",
                 "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        out = json.loads(resp.read())
    failed = [r for r in out.get("results", []) if r.get("type") == "error"]
    if failed:
        raise RuntimeError(
            f"turso pipeline failed: "
            f"{failed[0].get('error', {}).get('message', 'pipeline error')}"[:200])


def turso_events_append(kind, detail, at_iso):
    """Append one row to the fleet-queue events table. Raises on
    failure; the caller decides fatality. Timestamp normalized to
    the queue backend's format (whole seconds, Z) so lexicographic
    order stays chronological."""
    created = at_iso.replace("+00:00", "Z")
    if "." in created:
        created = created.split(".")[0] + "Z"
    sql = ("insert into events (worker_id, task_id, kind, detail,"
           " created_at) values (" + ", ".join([
               q("fleet-fetcher"), "NULL", q(kind),
               q(json.dumps(detail)), q(created)]) + ")")
    turso_pipeline([sql])


def run(cmd, timeout=120, env_extra=None):
    env = dict(os.environ)
    env["TMPDIR"] = SQL_TMPDIR  # keep children off the small /tmp tmpfs
    if env_extra:
        env.update(env_extra)
    return subprocess.run(cmd, capture_output=True, text=True,
                          timeout=timeout, env=env)


def gh_api(path):
    last_err = None
    for attempt in range(2):  # one retry: the egress proxy flakes transiently
        try:
            out = run(["gh", "api", path], timeout=120)
            if out.returncode == 0:
                return json.loads(out.stdout)
            last_err = out.stderr.strip()[:300]
        except Exception as exc:
            last_err = str(exc)[:300]
        if attempt == 0:
            import time
            time.sleep(5)
    raise RuntimeError(f"gh api {path}: {last_err}")


def fetch_repo(repo):
    """Gather one repo's snapshot. Returns (snapshot dict, notes list).
    Raises on hard failure of the gh side."""
    notes = ["gh-api only (no local clone fetch — Drew, 2026-11-02)"]

    meta = gh_api(f"repos/{repo}")
    default_branch = meta.get("default_branch")

    commit = gh_api(f"repos/{repo}/commits/{default_branch}")
    head_sha = commit.get("sha")

    prs_raw = gh_api(f"repos/{repo}/pulls?state=open&per_page={CAP}")
    open_prs = [{
        "number": p["number"],
        "title": p.get("title"),
        "author": (p.get("user") or {}).get("login"),
        "head_ref": (p.get("head") or {}).get("ref"),
        "updated_at": p.get("updated_at"),
        "is_draft": bool(p.get("draft")),
    } for p in prs_raw]
    pr_count = gh_api(
        f"search/issues?q=repo:{repo}+type:pr+state:open&per_page=1"
    )["total_count"]

    issues_raw = gh_api(f"repos/{repo}/issues?state=open&per_page={CAP}")
    open_issues = [{
        "number": i["number"],
        "title": i.get("title"),
        "labels": [l.get("name") for l in i.get("labels", [])],
        "updated_at": i.get("updated_at"),
    } for i in issues_raw if "pull_request" not in i]
    issue_count = gh_api(
        f"search/issues?q=repo:{repo}+type:issue+state:open&per_page=1"
    )["total_count"]

    runs_raw = gh_api(f"repos/{repo}/actions/runs?per_page=10")
    ci_summary = {"runs": [{
        "name": r.get("name"),
        "status": r.get("status"),
        "conclusion": r.get("conclusion"),
        "updated_at": r.get("updated_at"),
    } for r in runs_raw.get("workflow_runs", [])]}

    snapshot = {
        "repo": repo,
        "head_sha": head_sha,
        "default_branch": default_branch,
        "open_prs": open_prs,
        "open_issues": open_issues,
        "open_pr_count": pr_count,
        "open_issue_count": issue_count,
        "ci_summary": ci_summary,
    }

    if repo == NIX_REPO:
        try:
            upstream = gh_api("repos/NixOS/nixpkgs/commits/nixpkgs-unstable")
            if upstream.get("sha"):
                snapshot["_upstream_sha"] = upstream["sha"]
        except Exception as exc:
            notes.append(f"upstream sha read failed: {str(exc)[:120]}")

    return snapshot, notes


def write_sql(statements):
    """Kept name, new home (#5693): statements run against the
    fleet-queue Turso DB (agent_repo_snapshots / agent_watermarks /
    events), not Supabase."""
    turso_pipeline(statements)


def snapshot_statements(repo, snap, started):
    fetched = q(started)
    stmts = [
        "insert into agent_repo_snapshots (repo, head_sha, default_branch,"
        " open_prs, open_issues, open_pr_count, open_issue_count,"
        " ci_summary, fetched_at, fetch_source) values ("
        f"{q(repo)}, {q(snap['head_sha'])}, {q(snap['default_branch'])}, "
        f"{q(json.dumps(snap['open_prs']))}, {q(json.dumps(snap['open_issues']))}, "
        f"{snap['open_pr_count']}, {snap['open_issue_count']}, "
        f"{q(json.dumps(snap['ci_summary']))}, {fetched}, 'fetcher')"
        " on conflict (repo) do update set"
        " head_sha = excluded.head_sha,"
        " default_branch = excluded.default_branch,"
        " open_prs = excluded.open_prs,"
        " open_issues = excluded.open_issues,"
        " open_pr_count = excluded.open_pr_count,"
        " open_issue_count = excluded.open_issue_count,"
        " ci_summary = excluded.ci_summary,"
        f" fetched_at = {fetched}, fetch_source = 'fetcher';"
    ]
    if snap.get("_upstream_sha"):
        stmts.append(
            "insert into agent_watermarks (key, value, updated_at) values ("
            f"{q(UPSTREAM_WATERMARK_KEY)}, "
            f"{q(json.dumps({'sha': snap['_upstream_sha'], 'repo': repo, 'fetched_at': started}))}, "
            f"{fetched}) on conflict (key) do update set"
            f" value = excluded.value, updated_at = {fetched};"
        )
    return stmts


def main():
    from concurrent.futures import ThreadPoolExecutor

    started = now_iso()
    results = {}   # repo -> {"ok": bool, ...}
    snapshots = {}

    # Fetch all repos in parallel (subprocess-bound work); one repo's
    # failure never fails the others.
    with ThreadPoolExecutor(max_workers=len(REPOS)) as pool:
        futures = {pool.submit(fetch_repo, repo): repo for repo in REPOS}
        for fut, repo in futures.items():
            try:
                snap, notes = fut.result()
                snapshots[repo] = snap
                results[repo] = {
                    "ok": True,
                    "open_prs": snap["open_pr_count"],
                    "open_issues": snap["open_issue_count"],
                    "head_sha": snap["head_sha"],
                    "notes": notes,
                }
            except Exception as exc:
                results[repo] = {"ok": False, "error": str(exc)[:300]}

    # Write each snapshot in its own supabase-exec call so one bad write
    # cannot sink the repos that fetched fine.
    write_errors = []
    for repo, snap in snapshots.items():
        try:
            write_sql(snapshot_statements(repo, snap, started))
            results[repo]["write_ok"] = True
        except Exception as exc:
            results[repo]["write_ok"] = False
            write_errors.append(f"{repo}: {str(exc)[:200]}")

    ok_repos = [r for r, v in results.items() if v["ok"]]
    sink = events_sink()
    ev_detail = {'at': started, 'repos': results}
    final_stmts = [
        "insert into agent_watermarks (key, value, updated_at) values ("
        f"'fetcher:last_run', {q(json.dumps({'at': started, 'repos_ok': ok_repos, 'repos_failed': [r for r, v in results.items() if not v['ok']]}))}, "
        f"{q(started)}) on conflict (key) do update set"
        f" value = excluded.value, updated_at = {q(started)};",
    ]
    sink_notes = []
    if sink == "pg":
        # The PG event leg retired with the #5693 repoint; the
        # snapshot_refresh event's home is the fleet-queue sink.
        sink_notes.append("sink=pg ignored: PG event leg retired (#5693)")
    try:
        write_sql(final_stmts)
        db_ok = not write_errors
    except Exception as exc:
        db_ok = False
        write_errors.append(f"final: {str(exc)[:200]}")
    try:
        turso_events_append("snapshot_refresh", ev_detail, started)
    except Exception as exc:
        db_ok = False
        write_errors.append(f"turso-events: {str(exc)[:200]}")

    summary = (f"{started} fleet-fetch: "
               + " ".join(
                   f"{r}=ok(prs={v['open_prs']},issues={v['open_issues']})"
                   if v.get("ok") else f"{r}=FAIL({v.get('error', '?')[:80]})"
                   for r, v in results.items())
               + f" db_write={'ok' if db_ok else 'FAIL'}")
    if sink != "pg":
        summary += f" events_sink={sink}"
    if sink_notes:
        summary += " sink_notes=" + " | ".join(sink_notes)
    if write_errors:
        summary += " write_errors=" + " | ".join(write_errors)
    os.makedirs(os.path.dirname(LOG_FILE), exist_ok=True)
    with open(LOG_FILE, "a") as f:
        f.write(summary + "\n")
    print(summary)

    if not ok_repos:
        return 1
    return 0 if db_ok else 1


if __name__ == "__main__":
    sys.exit(main())
