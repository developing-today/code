#!/usr/bin/env python3
"""Scheduled five-copy freshness probe for #8584 slice 6 backups.

Verifies each of the five backup copies, rewrites the JSON status doc
that freshness_readout.py judges, prints the readout, and (with
--file-issues) keeps one alert issue in developing-today/code in sync
with the verdict: opened or updated while any copy is behind target,
commented and closed on recovery. This is the producer half of the
#8584 freshness leg; freshness_readout.py is the judge.

What "verified" means per copy (a copy is stamped only on proof):

- turso_export / d1_copy: the D1 `_fleet_queue_backup_meta` row of the
  `fleet-queue-backup` database. The export succeeds only after its
  D1 read-back counts match the source, so that one record is both
  copies' success record; both are stamped with its `exported_at`.
- plain_mirror: for each fleet repo, the `developing-today/<repo>-mirror`
  HEAD equals the local backup mirror's HEAD. Stamped with the probe
  time: the proof is "in sync now", not a sync timestamp.
- r2_bundles: for each fleet repo, the newest content-addressed bundle
  object in R2 names the local backup mirror's HEAD. Stamped with the
  probe time, same reasoning as the mirror.
- cloudflare_rehost: for each fleet repo, the HEAD oid in the
  rehost Worker's smart-HTTP advertisement (git.security.cab, the
  Workers + R2 surface Drew picked 2026-11-24) equals the local
  backup mirror's HEAD. Stamped with the probe time, same reasoning
  as the mirror.

A copy whose check fails or cannot run keeps its previous
last_success from the status doc, so it ages out against its target
instead of being silently refreshed or silently dropped. Checker
errors are printed, never swallowed.

Command line: [--status-json PATH] [--root PATH] [--bucket NAME]
[--now ISO] [--format human|json] [--rehost-base URL] [--file-issues].
Arguments are parsed strictly before any file or network access;
anything unaccounted for is a usage error (exit 2, #3352).

Exit codes: 0 every copy fresh, 1 any copy stale/invalid, 2 usage
error. Credentials arrive via the environment only (the systemd
wrapper fetches them by vault name at runtime); this module never
reads credential files and never prints a credential.
"""

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import freshness_readout
from ghb_common import atomic_write_text, backup_root, mirror_path
from plain_mirror import FLEET_REPOS

# Daily copies on daily timers (01:30 R2, 02:15 mirror, 02:15 queue
# export, each plus up to 30m randomized delay and real runtime): a
# 26h target absorbs the jitter and still catches a fully missed day
# the same morning.
TARGET_HOURS = {
    "turso_export": 26,
    "d1_copy": 26,
    "cloudflare_rehost": 26,
    "plain_mirror": 26,
    "r2_bundles": 26,
}

ALERT_REPO = "developing-today/code"
ALERT_TITLE_PREFIX = "Backup freshness:"
ALERT_MARKER = "backup-freshness-probe:v1"
DEFAULT_R2_BUCKET = "supabase-backups"
R2_PREFIX = "github-backup/cold-bundles"
SUBPROCESS_TIMEOUT_SECONDS = 300
# The rehost Worker (infra/github-backup/rehost), Drew's 2026-11-24
# surface pick. --rehost-base overrides it (tests, future moves).
DEFAULT_REHOST_BASE = "https://git.security.cab"
REHOST_FETCH_TIMEOUT_SECONDS = 60


def iso(when):
    return when.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def outcome(ok, detail, verified_at=None):
    return {"ok": ok, "detail": detail, "verified_at": verified_at}


# ------------------------------------------------------------ checkers
def judge_d1_meta(meta, now):
    """The export meta record is the success proof for both D1 copies."""
    if meta is None:
        return outcome(False, "D1 copy has no export meta record")
    raw = meta.get("exported_at")
    when = freshness_readout.parse_time(raw, "D1 meta exported_at") if raw else None
    if when is None:
        return outcome(False, f"D1 export meta has no usable exported_at: {raw!r}")
    if when > now + timedelta(minutes=5):
        return outcome(False, f"D1 export meta exported_at is in the future: {raw}")
    return outcome(True, f"last verified export at {raw}", verified_at=when)


def judge_head_pairs(pairs):
    """pairs: (label, expected_head, actual_head). All must match."""
    problems = []
    for label, expected, actual in pairs:
        if not expected or not actual:
            problems.append(f"{label}: missing head (expected {expected!r}, got {actual!r})")
        elif expected != actual:
            problems.append(f"{label}: head {actual} != expected {expected}")
    if problems:
        return outcome(False, "; ".join(problems))
    return outcome(True, f"all {len(pairs)} repos verified in sync")


def parse_advertised_head(body):
    """HEAD oid from a smart-HTTP info/refs advertisement, or None.

    The advertisement is pkt-lines: the `# service=git-upload-pack`
    line, a flush, then `<oid> HEAD` as the first ref line. Anything
    else — an error page, a truncated body, a non-hex oid — is not
    proof of a copy and parses to None."""
    if not isinstance(body, (bytes, bytearray)):
        return None
    lines = []
    i = 0
    while i + 4 <= len(body):
        try:
            length = int(body[i : i + 4], 16)
        except ValueError:
            return None
        if length == 0:
            lines.append(None)
            i += 4
            continue
        if length < 4 or i + length > len(body):
            return None
        lines.append(bytes(body[i + 4 : i + length]))
        i += length
    if not lines or lines[0] != b"# service=git-upload-pack\n":
        return None
    for line in lines[1:]:
        if line is None:
            continue
        fields = line.rstrip(b"\n").split(b" ", 1)
        if len(fields) == 2 and fields[1].split(b"\0")[0] == b"HEAD":
            oid = fields[0].decode("ascii", "replace")
            if len(oid) == 40 and all(c in "0123456789abcdef" for c in oid):
                return oid
            return None
    return None


def parse_bundle_head(name):
    """'<head>-<sha256>.bundle' -> head, or None when unparseable.

    bundle_r2.py content-addresses objects by full git SHA (40 hex)
    and full SHA-256 (64 hex); anything else in the prefix is not a
    bundle this system published and does not count as a copy."""
    if not name.endswith(".bundle"):
        return None
    stem = name[: -len(".bundle")]
    head, sep, digest = stem.rpartition("-")
    if not sep or len(head) != 40 or len(digest) != 64:
        return None
    if any(c not in "0123456789abcdef" for c in head + digest):
        return None
    return head


def newest_bundle_head(entries):
    """entries: rclone lsjson dicts for one repo prefix. Newest by ModTime."""
    best = None
    for entry in entries:
        if entry.get("IsDir"):
            continue
        head = parse_bundle_head(entry.get("Name", ""))
        if head is None:
            continue
        mod = freshness_readout.parse_time(entry.get("ModTime", ""), "rclone ModTime")
        if mod is None:
            continue
        if best is None or mod > best[0]:
            best = (mod, head)
    return best[1] if best else None


# ------------------------------------------------------------ fetchers
def _run(argv, timeout=SUBPROCESS_TIMEOUT_SECONDS):
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"{argv[0]} timed out") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise RuntimeError(f"{' '.join(argv[:3])} failed: {detail[:400]}")
    return result.stdout.strip()


def _git_head(mirror):
    return _run(["git", "--git-dir", mirror, "rev-parse", "HEAD"], timeout=60)


def _gh_text(args):
    """gh api with a --jq scalar projection prints the raw value."""
    return _run(["gh", "api", *args], timeout=120)


def fetch_d1_meta():
    token = os.environ.get("CLOUDFLARE_API_TOKEN")
    account = os.environ.get("CLOUDFLARE_ACCOUNT_ID")
    if not token or not account:
        raise RuntimeError(
            "CLOUDFLARE_API_TOKEN / CLOUDFLARE_ACCOUNT_ID not in environment "
            "(the systemd wrapper fetches them by vault name)"
        )
    fq_dir = os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "..",
        "..",
        "fleet-queue-backup",
        "bin",
    )
    sys.path.insert(0, os.path.abspath(fq_dir))
    import fq_common

    client = fq_common.D1Client(token, account)
    db = client.find_database(fq_common.DEFAULT_D1_NAME)
    if db is None:
        raise RuntimeError(f"D1 database {fq_common.DEFAULT_D1_NAME!r} not found")
    return client.read_meta(db["uuid"])


def fetch_mirror_pairs(root):
    pairs = []
    for repo in FLEET_REPOS:
        local = _git_head(mirror_path(root, repo))
        name = repo.split("/")[1]
        dest = f"developing-today/{name}-mirror"
        default = _gh_text([f"repos/{dest}", "--jq", ".default_branch"])
        remote = _gh_text([f"repos/{dest}/commits/{default}", "--jq", ".sha"])
        pairs.append((repo, local, remote))
    return pairs


def fetch_r2_pairs(root, bucket):
    pairs = []
    for repo in FLEET_REPOS:
        owner, name = repo.split("/")
        listing = _run(
            [
                "rclone",
                "lsjson",
                f"r2bak:{bucket}/{R2_PREFIX}/{owner}/{name}/",
            ]
        )
        head = newest_bundle_head(json.loads(listing))
        local = _git_head(mirror_path(root, repo))
        pairs.append((repo, local, head))
    return pairs


def _fetch_advertisement(url):
    request = urllib.request.Request(
        url, headers={"user-agent": "fleet-freshness-probe"}
    )
    try:
        with urllib.request.urlopen(
            request, timeout=REHOST_FETCH_TIMEOUT_SECONDS
        ) as response:
            return response.read()
    except Exception as exc:
        # Transport failures surface as URLError (an OSError), but a
        # truncated body is http.client.IncompleteRead — a bare
        # HTTPException, not an OSError. Normalize both to RuntimeError
        # so collect_outcomes records an unverified copy instead of
        # the probe crashing on a network-shaped error.
        raise RuntimeError(f"GET {url} failed: {exc}") from exc


def fetch_rehost_pairs(root, base):
    """(repo, local mirror HEAD, rehost-advertised HEAD) per fleet repo."""
    pairs = []
    for repo in FLEET_REPOS:
        local = _git_head(mirror_path(root, repo))
        body = _fetch_advertisement(
            f"{base}/{repo}/info/refs?service=git-upload-pack"
        )
        pairs.append((repo, local, parse_advertised_head(body)))
    return pairs


def collect_outcomes(root, bucket, now, rehost_base=DEFAULT_REHOST_BASE):
    """Run every checker; a fetcher failure is an unverified copy, not
    a crashed probe — the copy ages out on its previous stamp."""
    outcomes = {}
    try:
        meta = fetch_d1_meta()
        d1 = judge_d1_meta(meta, now)
    except (OSError, RuntimeError, ValueError) as exc:
        d1 = outcome(False, f"D1 meta check failed: {exc}")
    outcomes["turso_export"] = d1
    outcomes["d1_copy"] = d1
    try:
        outcomes["plain_mirror"] = judge_head_pairs(fetch_mirror_pairs(root))
        if outcomes["plain_mirror"]["ok"]:
            outcomes["plain_mirror"]["verified_at"] = now
    except (OSError, RuntimeError, ValueError) as exc:
        outcomes["plain_mirror"] = outcome(False, f"mirror check failed: {exc}")
    try:
        outcomes["r2_bundles"] = judge_head_pairs(fetch_r2_pairs(root, bucket))
        if outcomes["r2_bundles"]["ok"]:
            outcomes["r2_bundles"]["verified_at"] = now
    except (OSError, RuntimeError, ValueError) as exc:
        outcomes["r2_bundles"] = outcome(False, f"R2 check failed: {exc}")
    try:
        outcomes["cloudflare_rehost"] = judge_head_pairs(
            fetch_rehost_pairs(root, rehost_base)
        )
        if outcomes["cloudflare_rehost"]["ok"]:
            outcomes["cloudflare_rehost"]["verified_at"] = now
    except (OSError, RuntimeError, ValueError) as exc:
        outcomes["cloudflare_rehost"] = outcome(
            False, f"rehost check failed: {exc}"
        )
    return outcomes


# ------------------------------------------------------------ status doc
def load_previous(path):
    """Previous status doc, or ({}, problem|None). A corrupt previous
    doc is reported and treated as empty: every copy then has to
    re-prove itself, which is the safe direction."""
    if not os.path.exists(path):
        return {}, None
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, json.JSONDecodeError) as exc:
        return {}, f"previous status doc unreadable ({exc}); starting empty"
    if not isinstance(doc, dict) or not isinstance(doc.get("copies"), dict):
        return {}, "previous status doc has no copies object; starting empty"
    return doc, None


def build_status(previous, outcomes, now):
    copies = {}
    for name in freshness_readout.CANONICAL_COPIES:
        result = outcomes[name]
        prev = (previous.get("copies") or {}).get(name) or {}
        if result["ok"] and result["verified_at"] is not None:
            last_success = iso(result["verified_at"])
        else:
            last_success = prev.get("last_success")
        copies[name] = {
            "last_success": last_success,
            "target_hours": TARGET_HOURS[name],
            "detail": result["detail"],
            "verified": bool(result["ok"]),
        }
    return {"generated_at": iso(now), "copies": copies}


# ------------------------------------------------------------ alert issue
def _gh_issue(argv):
    return _run(["gh", "issue", *argv], timeout=120)


def find_alert_issue(runner=_gh_issue):
    listing = runner(
        [
            "list",
            "--repo",
            ALERT_REPO,
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,body",
        ]
    )
    for item in json.loads(listing):
        if ALERT_MARKER in (item.get("body") or ""):
            return item["number"]
    return None


def alert_body(results, now):
    lines = [
        f"<!-- {ALERT_MARKER} -->",
        "The scheduled five-copy freshness probe (#8584) reports copies "
        "behind their freshness target:",
        "",
    ]
    for row in results:
        if row["verdict"] != "PASS":
            lines.append(f"- {row['copy']}: {row['verdict']}: {row['detail']}")
    lines += ["", f"Probe time: {iso(now)}. Refs #8584."]
    return "\n".join(lines)


def sync_alert(results, now, runner=_gh_issue):
    """One open alert issue tracks the stale set: created when copies
    fall behind, updated while they stay behind, closed on recovery.
    Returns a short description of what was done."""
    stale = [r for r in results if r["verdict"] != "PASS"]
    existing = find_alert_issue(runner)
    if stale:
        body = alert_body(results, now)
        if existing is None:
            names = ", ".join(r["copy"] for r in stale)
            runner(
                [
                    "create",
                    "--repo",
                    ALERT_REPO,
                    "--title",
                    f"{ALERT_TITLE_PREFIX} {names} behind target (#8584)",
                    "--body",
                    body,
                ]
            )
            return "opened alert issue"
        runner(["comment", str(existing), "--repo", ALERT_REPO, "--body", body])
        return f"updated alert issue #{existing}"
    if existing is not None:
        runner(
            [
                "comment",
                str(existing),
                "--repo",
                ALERT_REPO,
                "--body",
                f"All five copies fresh again at {iso(now)}; closing. Refs #8584.",
            ]
        )
        runner(["close", str(existing), "--repo", ALERT_REPO])
        return f"closed alert issue #{existing} on recovery"
    return "no alert issue needed"


# ------------------------------------------------------------ main
def _parser():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument(
        "--status-json",
        help="status doc path (default: <backup root>/freshness-status.json)",
    )
    parser.add_argument("--root", help="backup tree root (defaults to configured)")
    parser.add_argument(
        "--bucket", default=DEFAULT_R2_BUCKET, help="existing R2 bucket"
    )
    parser.add_argument("--now", help="pin the clock (ISO 8601, tests only)")
    parser.add_argument(
        "--rehost-base",
        default=DEFAULT_REHOST_BASE,
        help="base URL of the Cloudflare rehost Worker",
    )
    parser.add_argument(
        "--format", choices=("human", "json"), default="human", help="output format"
    )
    parser.add_argument(
        "--file-issues",
        action="store_true",
        help="open/update/close the backup-freshness alert issue from the verdict",
    )
    return parser


def main(argv=None):
    args = _parser().parse_args(argv)
    now = (
        freshness_readout.parse_time(args.now, "--now")
        if args.now
        else datetime.now(timezone.utc)
    )
    if now is None:
        return 2
    root = (
        os.path.abspath(os.path.expanduser(args.root)) if args.root else backup_root()
    )
    status_path = args.status_json or os.path.join(root, "freshness-status.json")

    outcomes = collect_outcomes(root, args.bucket, now, args.rehost_base)
    for name, result in outcomes.items():
        if not result["ok"]:
            print(f"freshness_probe: {name}: {result['detail']}", file=sys.stderr)
    previous, problem = load_previous(status_path)
    if problem:
        print(f"freshness_probe: {problem}", file=sys.stderr)
    doc = build_status(previous, outcomes, now)
    atomic_write_text(status_path, json.dumps(doc, indent=2, sort_keys=True) + "\n")

    results = freshness_readout.judge(doc, now)
    if args.format == "json":
        print(json.dumps({"results": results}, indent=2, sort_keys=True))
    else:
        for row in results:
            print(
                f"freshness_probe: {row['copy']}: {row['verdict']}: {row['detail']}"
            )
    if args.file_issues:
        try:
            print("freshness_probe: alert: " + sync_alert(results, now))
        except (OSError, RuntimeError, ValueError) as exc:
            print(f"freshness_probe: alert filing failed: {exc}", file=sys.stderr)
            return 1
    return 0 if all(r["verdict"] == "PASS" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
