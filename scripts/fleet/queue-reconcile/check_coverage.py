#!/usr/bin/env python3
"""Standing issue<->queue coverage reconciliation (#7016).

The gap this closes: issues enter the fleet queue only when a
creator-class lane files them (file + enqueue in the same pass). Issues
filed any other way — a work lane filing a blocker mid-task, an audit
lane, by hand, or predating the queue (epics) — silently never get a
queue item, so no lane ever dispositions them. The 2026-11-07
reconciliation found 23 such issues across the four fleet repos; the
import that day was a one-off, and this script is the standing version.

The check, per Drew's doctrine (2026-11-07): **every open issue must
hold a LIVE task (queued/claimed/parked/blocked) in its own pilot
queue.** A done task is not coverage (done means triage or a slice, not
a resolved issue), and neither is a task in the shadow queue.

Two halves, deliberately split:

- **The diff** is done by the queue broker (`POST /coverage`), which
  owns the Turso connection. It is read-only and safe to run any time.
- **The import** is done here, through the sanctioned CLI enqueue path,
  so packets are validated and dedupe-keyed exactly like every other
  task. The dedupe key makes a re-run idempotent by construction.

Default is observe-only: the run reports what it *would* import and
writes nothing. Pass `--import` to actually enqueue.

Usage:
    check_coverage.py [--observe | --import] [--repo owner/name ...]
                      [--broker-url URL] [--token-file PATH]
                      [--state-dir DIR] [--now ISO8601]
                      [--fixture JSON]

`--fixture` replaces both the GitHub fetch and the broker call with a
JSON file shaped `{"issues": {"owner/repo": [...]}, "coverage":
{"owner/repo": {"uncovered_issues": [...]}}}`, so the whole decision
path runs offline in tests.

Exit codes: 0 = run completed (uncovered may be > 0 in observe mode),
2 = configuration/usage error, 3 = a fetch or broker call failed (the
report names which; a failure never masquerades as "0 uncovered").
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

# The four pilot queues and the repo each one covers. A repo outside this
# map is not reconciled — the pilot is scoped to these four by design
# (queue-worker-pilot-design-2026-10-25 v1.1).
PILOT_QUEUES: dict[str, str] = {
    "repo-health-code-pilot": "developing-today/code",
    "repo-health-opencode-pilot": "developing-today-forks/opencode",
    "repo-health-openchamber-pilot": "developing-today-forks/openchamber",
    "repo-health-nix-pilot": "dezren39/darwin-nix",
}

DEFAULT_STATE_DIR = Path.home() / ".local/state/queue-reconcile"
DEFAULT_TOKEN_FILE = Path.home() / ".config/queue-broker/token"
DEFAULT_BROKER_URL = "http://100.99.113.103:4801"
DEFAULT_BACKLOG = Path.home() / "workspace/code-backlog.md"


class FetchError(RuntimeError):
    """A GitHub or broker call failed; the run must not report success."""


def now_iso(override: str | None = None) -> str:
    if override:
        return override
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------------------
# GitHub: enumerate open issues (PRs excluded)
# ---------------------------------------------------------------------------


def fetch_open_issues(repo: str, gh: str = "gh") -> list[dict]:
    """All open issues in `repo`, newest first, PRs filtered out.

    Uses `gh api` with `--paginate` so the result is complete rather
    than the first page — a partial enumeration would read as coverage
    that does not exist.
    """
    cmd = [
        gh,
        "api",
        "--paginate",
        f"repos/{repo}/issues?state=open&per_page=100",
        "--jq",
        '.[] | select(has("pull_request") | not) | {number, title}',
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise FetchError(f"github enumeration failed for {repo}: {exc}") from exc
    if out.returncode != 0:
        raise FetchError(
            f"github enumeration failed for {repo} (exit {out.returncode}): "
            f"{out.stderr.strip()[:400]}"
        )
    issues: list[dict] = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        issues.append(json.loads(line))
    return issues


# ---------------------------------------------------------------------------
# Broker: the coverage diff
# ---------------------------------------------------------------------------


def broker_coverage(
    broker_url: str,
    token: str,
    queue: str,
    repo: str,
    issues: list[dict],
) -> dict:
    body = json.dumps({"queue": queue, "repo": repo, "issues": issues}).encode()
    req = urllib.request.Request(
        broker_url.rstrip("/") + "/coverage",
        data=body,
        headers={"X-Agent-Token": token, "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            payload = json.loads(resp.read())
    except (urllib.error.URLError, OSError, json.JSONDecodeError) as exc:
        raise FetchError(f"broker /coverage failed for {queue}: {exc}") from exc
    if not payload.get("ok"):
        raise FetchError(
            f"broker /coverage rejected {queue}: {payload.get('error', payload)}"
        )
    return payload


# ---------------------------------------------------------------------------
# Import: one triage packet per uncovered issue, dedupe-keyed
# ---------------------------------------------------------------------------


def triage_packet(repo: str, issue: dict, checked_at: str) -> dict:
    number = issue["number"]
    title = issue.get("title") or f"issue {number}"
    url = f"https://github.com/{repo}/issues/{number}"
    return {
        "title": f"Triage {repo}#{number}: {title}"[:180],
        "body": (
            f"Imported by the standing queue coverage reconciliation (#7016): "
            f"{url} is open with no live task in its pilot queue."
        ),
        "priority": 30,
        "dedupe_key": f"repo-health:v1:triage:{repo}:issue:{number}",
        "payload": {
            "schema": "repo-task/v1",
            "kind": "triage",
            "repo": repo,
            "queue": None,  # filled by the caller with the pilot queue
            "issue": {
                "number": number,
                "url": url,
                "title": title,
                "state_checked_at": checked_at,
            },
            "problem": (
                "No live queue task covers this open issue: it was filed outside a "
                "creator run (or predates the queue), so no lane would ever "
                "disposition it. Filed by the standing coverage reconciliation "
                "(#7016)."
            ),
            "evidence": [
                f"Coverage reconciliation {checked_at}: issue #{number} is open with no "
                "queued/claimed/parked/blocked task in its pilot queue",
                url,
            ],
            "acceptance": [
                "Issue triaged per the standing triage doctrine: annotate, validate, and "
                "either fix it in a small edit (merge with a closing keyword) or return it "
                "to the queue for real work",
            ],
            "tests": [
                "Triage verdict recorded on the issue; if a small fix, the merged PR "
                "carries a closing keyword",
            ],
            "scope": {
                "include": [f"Triage of {repo}#{number}"],
                "exclude": ["Secrets, credentials, purchases, or Drew-only decisions"],
            },
            "risk": {"level": "low", "flags": ["reconciliation-import"]},
            "priority_reason": (
                "Normal (30): imported by coverage reconciliation, not filed by a creator run."
            ),
            "timebox_minutes": 45,
            "max_attempts": 6,
            "context_refs": [url],
            "dedupe": {
                "checked_at": checked_at,
                "basis": "Coverage reconciliation (#7016): no live task held this issue at check time.",
            },
        },
    }


def enqueue_packet(
    packet: dict,
    queue: str,
    fleet: str,
    broker_url: str,
    token_file: str,
) -> tuple[bool, str]:
    """Enqueue via the sanctioned CLI path. Returns (ok, detail).

    The child gets the same broker URL and token file this run selected
    (`--broker-url` / `--token-file`), so it cannot enqueue into a
    different broker than the one whose coverage was read. The packet
    file is unique per enqueue: an ad-hoc run overlapping the scheduled
    one must never overwrite or delete the other's packet.
    """
    payload = packet["payload"]
    repo_slug = payload["repo"].replace("/", "-")
    number = payload["issue"]["number"]
    fd, tmp_name = tempfile.mkstemp(
        prefix=f"coverage-{repo_slug}-{number}-",
        suffix=".json",
        dir=os.environ.get("TMPDIR") or None,
    )
    tmp = Path(tmp_name)
    try:
        with os.fdopen(fd, "w") as fh:
            json.dump(packet, fh)
        env = os.environ.copy()
        env["REPO_QUEUE_BROKER_URL"] = broker_url
        env["REPO_QUEUE_TOKEN_FILE"] = str(token_file)
        try:
            out = subprocess.run(
                [fleet, "queue", "enqueue", str(tmp), "--queue", queue],
                capture_output=True,
                text=True,
                timeout=120,
                env=env,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            # A launch failure or timeout is an enqueue failure, not a
            # reason to abandon the remaining issues and the report.
            return False, f"could not run enqueue: {exc}"
    finally:
        tmp.unlink(missing_ok=True)
    if out.returncode != 0:
        return False, (out.stderr or out.stdout).strip()[:300]
    return True, out.stdout.strip()[:300]


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------


def render_report(
    checked_at: str,
    mode: str,
    results: list[dict],
    errors: list[str],
) -> str:
    lines = [
        f"# Queue coverage reconciliation — {checked_at}",
        "",
        f"Mode: **{mode}**. Doctrine: every open issue must hold a LIVE task "
        "(queued/claimed/parked/blocked) in its own pilot queue; a done task is not coverage.",
        "",
        "| repo | queue | open issues | covered | uncovered | imported |",
        "|---|---|---|---|---|---|",
    ]
    for r in results:
        lines.append(
            f"| {r['repo']} | {r['queue']} | {r['open_issues']} | {r['covered']} "
            f"| {r['uncovered']} | {r['imported']} |"
        )
    total_uncovered = sum(r["uncovered"] for r in results)
    total_imported = sum(r["imported"] for r in results)
    lines += [
        "",
        f"**Total uncovered: {total_uncovered}** · imported this run: {total_imported}",
        "",
    ]
    if errors:
        lines += ["## ERRORS", ""] + [f"- {e}" for e in errors] + [""]
    uncovered_any = [(r, i) for r in results for i in r.get("uncovered_issues", [])]
    if uncovered_any:
        lines += ["## Uncovered issues", ""]
        for r, i in uncovered_any:
            lines.append(f"- {r['repo']}#{i['number']} — {i.get('title', '')}")
        lines.append("")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--observe", action="store_true", help="report only (default)")
    mode.add_argument(
        "--import",
        dest="do_import",
        action="store_true",
        help="enqueue uncovered issues",
    )
    ap.add_argument(
        "--repo", action="append", help="restrict to these repos (repeatable)"
    )
    ap.add_argument(
        "--broker-url",
        default=os.environ.get("REPO_QUEUE_BROKER_URL", DEFAULT_BROKER_URL),
    )
    ap.add_argument("--token-file", default=str(DEFAULT_TOKEN_FILE))
    ap.add_argument("--state-dir", default=str(DEFAULT_STATE_DIR))
    ap.add_argument("--backlog", default=str(DEFAULT_BACKLOG))
    ap.add_argument("--fleet", default=os.environ.get("FLEET_BIN", "fleet"))
    ap.add_argument("--now", default=None)
    ap.add_argument("--fixture", default=None)
    args = ap.parse_args(argv)

    checked_at = now_iso(args.now)
    do_import = bool(args.do_import)
    mode_name = "import" if do_import else "observe"

    selected = list(PILOT_QUEUES.items())
    if args.repo:
        wanted = set(args.repo)
        selected = [(q, r) for q, r in selected if r in wanted]
        unknown = wanted - {r for _, r in selected}
        if unknown:
            print(f"error: no pilot queue covers {sorted(unknown)}", file=sys.stderr)
            return 2

    fixture = None
    if args.fixture:
        fixture = json.loads(Path(args.fixture).read_text())

    token = ""
    if not fixture:
        try:
            token = Path(args.token_file).read_text().strip()
        except OSError as exc:
            print(
                f"error: cannot read broker token {args.token_file}: {exc}",
                file=sys.stderr,
            )
            return 2
        if not token:
            print(
                f"error: broker token file {args.token_file} is empty", file=sys.stderr
            )
            return 2

    results: list[dict] = []
    errors: list[str] = []
    for queue, repo in selected:
        try:
            if fixture:
                issues = fixture["issues"][repo]
                cov = fixture["coverage"][repo]
            else:
                issues = fetch_open_issues(repo)
                cov = broker_coverage(args.broker_url, token, queue, repo, issues)
        except (FetchError, KeyError) as exc:
            errors.append(str(exc))
            continue

        uncovered = cov.get("uncovered_issues", [])
        imported = 0
        if do_import and uncovered:
            for issue in uncovered:
                packet = triage_packet(repo, issue, checked_at)
                ok, detail = enqueue_packet(
                    packet, queue, args.fleet, args.broker_url, args.token_file
                )
                if ok:
                    imported += 1
                else:
                    errors.append(
                        f"enqueue failed for {repo}#{issue['number']}: {detail}"
                    )

        results.append(
            {
                "repo": repo,
                "queue": queue,
                "open_issues": cov.get("open_issues", len(issues)),
                "covered": cov.get("covered", 0),
                "uncovered": len(uncovered),
                "imported": imported,
                "uncovered_issues": uncovered,
            }
        )

    report = render_report(checked_at, mode_name, results, errors)

    state_dir = Path(args.state_dir)
    state_dir.mkdir(parents=True, exist_ok=True)
    stamp = checked_at.replace(":", "").replace("-", "")
    report_path = state_dir / f"report-{stamp}.md"
    report_path.write_text(report)
    (state_dir / "latest.md").write_text(report)

    print(report)

    if errors:
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
