#!/usr/bin/env python3
"""Hosted five-leg freshness comparison for #15001 (slice of #14964).

The laptop-era freshness_probe.py judges copies against the HEAD of
the laptop's local backup mirror. That reference point is exactly what
#15001 excludes: laptop-local state is non-authoritative, and a probe
that needs the laptop cannot notice the laptop is the thing that died.

This probe's reference point is the source repository's own GitHub
refs. Every leg is observed through a hosted surface only:

- source:            the source repo's default-branch head, via gh api.
                     This leg defines truth; it is reported so a report
                     shows the exact heads every copy was judged against.
- plain_mirror:      the `developing-today/<repo>-mirror` head, via gh api.
- r2_bundles:        the newest content-addressed bundle object in R2
                     (its name carries the head it captured; its ModTime
                     is the capture time), via rclone lsjson.
- cloudflare_rehost: the HEAD in the rehost Worker's smart-HTTP
                     advertisement at git.security.cab.
- d1_export:         the `_fleet_queue_backup_meta` row of the
                     fleet-queue-backup D1 database (fleet-queue-backup's
                     D1Client; credentials via environment only).

Each leg carries an explicit SLO (SLO_HOURS). A per-repo leg result is
one of:

- fresh:   the copy provably matches the source head (git legs), or
           its timestamped proof is within the leg's SLO (r2_bundles,
           d1_export; r2_bundles must also name the source head).
- stale:   a copy exists but is behind the source head, or its proof
           is older than the leg's SLO.
- missing: no copy exists at all (no mirror repo, no bundle object,
           no rehost advertisement, no export meta record).
- failed:  the check itself could not run (transport/API error, an
           unusable proof record, a source head that could not be
           fetched). A failed check is never read as a fresh copy.

A leg's rollup across the inventory is fresh when every repo is fresh,
missing when every repo is missing, failed when every repo failed,
partial when at least one repo is fresh but not all are, and stale
otherwise (copies exist or checks failed, but none is fresh). The
report therefore distinguishes stale, missing, partial, and failed
copies per leg and per repo, with the observed heads/timestamps and
the run identity as evidence.

The report is the only output: JSON on stdout, exit 0 when every leg
is fresh, 1 otherwise, 2 on usage error. Alerting (deduplicated
GitHub issues) and missed-run watching are separate slices of #15001
that consume this report; this module files nothing.

Command line: [--bucket NAME] [--rehost-base URL] [--now ISO]
[--run-id ID]. Arguments are parsed strictly before any file or
network access; anything unaccounted for is a usage error (exit 2,
#3352). Credentials arrive via the environment only; this module
never reads credential files and never prints a credential.
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

import freshness_probe
import freshness_readout
from plain_mirror import FLEET_REPOS

PROBE_ID = "hosted-freshness:v1"
REPORT_VERSION = 1

# Per-leg SLOs in hours. Daily publishers (mirror, R2, rehost, queue
# export) on daily cadences: 26h absorbs schedule jitter and still
# catches a fully missed day the same morning — the same reasoning as
# the laptop probe's targets, restated per leg so each leg's tolerance
# is explicit and independently tunable.
SLO_HOURS = {
    "plain_mirror": 26,
    "r2_bundles": 26,
    "cloudflare_rehost": 26,
    "d1_export": 26,
}

GIT_LEGS = ("plain_mirror", "cloudflare_rehost")
DEFAULT_R2_BUCKET = freshness_probe.DEFAULT_R2_BUCKET
R2_PREFIX = freshness_probe.R2_PREFIX
DEFAULT_REHOST_BASE = freshness_probe.DEFAULT_REHOST_BASE
SUBPROCESS_TIMEOUT_SECONDS = 300
REHOST_FETCH_TIMEOUT_SECONDS = 60

FRESH = "fresh"
STALE = "stale"
MISSING = "missing"
PARTIAL = "partial"
FAILED = "failed"


def iso(when):
    return when.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def result(status, detail, **evidence):
    entry = {"status": status, "detail": detail}
    entry.update(evidence)
    return entry


# ------------------------------------------------------------ judging
def judge_head_leg(source_head, copy_head):
    """Git-shaped legs: the copy is fresh only at the source head."""
    if copy_head is None:
        return result(MISSING, "no copy exists")
    if copy_head != source_head:
        return result(
            STALE,
            f"copy head {copy_head} != source head {source_head}",
            copy_head=copy_head,
        )
    return result(FRESH, "copy head matches source head", copy_head=copy_head)


def judge_bundle_result(source_head, bundle, now, slo_hours):
    """bundle: (head, modtime) of the newest bundle, or None."""
    if bundle is None:
        return result(MISSING, "no bundle object names a captured head")
    head, modtime = bundle
    evidence = {"copy_head": head, "bundle_at": iso(modtime)}
    if head != source_head:
        return result(
            STALE,
            f"newest bundle names head {head} != source head {source_head}",
            **evidence,
        )
    age = now - modtime
    if age > timedelta(hours=slo_hours):
        return result(
            STALE,
            f"newest bundle is {age} old, beyond the {slo_hours}h SLO",
            **evidence,
        )
    return result(FRESH, "newest bundle names the source head", **evidence)


def judge_d1_meta(meta, now, slo_hours):
    if meta is None:
        return result(MISSING, "D1 copy has no export meta record")
    raw = meta.get("exported_at")
    when = freshness_readout.parse_time(raw, "D1 meta exported_at") if raw else None
    if when is None:
        return result(
            FAILED, f"D1 export meta has no usable exported_at: {raw!r}"
        )
    if when > now + timedelta(minutes=5):
        return result(FAILED, f"D1 export meta exported_at is in the future: {raw}")
    age = now - when
    if age > timedelta(hours=slo_hours):
        return result(
            STALE,
            f"last verified export at {raw} is {age} old, beyond the {slo_hours}h SLO",
            exported_at=raw,
        )
    return result(FRESH, f"last verified export at {raw}", exported_at=raw)


def rollup(statuses):
    """Leg status across the inventory; see the module docstring."""
    statuses = list(statuses)
    if not statuses:
        return FAILED
    if all(s == FRESH for s in statuses):
        return FRESH
    if all(s == MISSING for s in statuses):
        return MISSING
    if all(s == FAILED for s in statuses):
        return FAILED
    if any(s == FRESH for s in statuses):
        return PARTIAL
    return STALE


# ------------------------------------------------------------ fetchers
def _run(argv, timeout=SUBPROCESS_TIMEOUT_SECONDS):
    try:
        completed = subprocess.run(
            argv, capture_output=True, text=True, timeout=timeout
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"{argv[0]} timed out") from exc
    if completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"{' '.join(argv[:3])} failed: {detail[:400]}")
    return completed.stdout.strip()


def _gh_text(args):
    return _run(["gh", "api", *args], timeout=120)


def _gh_head(repo):
    """Default-branch head of a GitHub repo, or None when it 404s."""
    try:
        default = _gh_text([f"repos/{repo}", "--jq", ".default_branch"])
        return _gh_text([f"repos/{repo}/commits/{default}", "--jq", ".sha"])
    except RuntimeError as exc:
        if "404" in str(exc):
            return None
        raise


def fetch_source_head(repo):
    head = _gh_head(repo)
    if head is None:
        raise RuntimeError(f"source repo {repo} not found")
    return head


def fetch_mirror_head(repo):
    name = repo.split("/")[1]
    return _gh_head(f"developing-today/{name}-mirror")


def newest_bundle(entries):
    """(head, modtime) of the newest valid bundle in an rclone listing."""
    best = None
    for entry in entries:
        if entry.get("IsDir"):
            continue
        head = freshness_probe.parse_bundle_head(entry.get("Name", ""))
        if head is None:
            continue
        mod = freshness_readout.parse_time(entry.get("ModTime", ""), "rclone ModTime")
        if mod is None:
            continue
        if best is None or mod > best[1]:
            best = (head, mod)
    return best


def fetch_bundle(repo, bucket):
    owner, name = repo.split("/")
    listing = _run(
        ["rclone", "lsjson", f"r2bak:{bucket}/{R2_PREFIX}/{owner}/{name}/"]
    )
    return newest_bundle(json.loads(listing))


def fetch_rehost_head(repo, base):
    """Advertised HEAD, or None when the rehost has no such repo (404)."""
    url = f"{base}/{repo}/info/refs?service=git-upload-pack"
    request = urllib.request.Request(url, headers={"user-agent": PROBE_ID})
    try:
        with urllib.request.urlopen(
            request, timeout=REHOST_FETCH_TIMEOUT_SECONDS
        ) as response:
            body = response.read()
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise RuntimeError(f"GET {url} failed: HTTP {exc.code}") from exc
    except Exception as exc:
        raise RuntimeError(f"GET {url} failed: {exc}") from exc
    return freshness_probe.parse_advertised_head(body)


def real_fetchers(bucket, rehost_base):
    return {
        "source_head": fetch_source_head,
        "mirror_head": fetch_mirror_head,
        "bundle": lambda repo: fetch_bundle(repo, bucket),
        "rehost_head": lambda repo: fetch_rehost_head(repo, rehost_base),
        "d1_meta": freshness_probe.fetch_d1_meta,
    }


# ------------------------------------------------------------ evaluation
def evaluate_repo(repo, fetchers):
    """Per-leg results for one repo; fetcher errors become FAILED.

    Returns (source_head, legs). The r2_bundles entry is deferred as
    ("bundle", value) because judging it needs `now` and the leg SLO;
    evaluate() resolves it. A source failure fails every copy leg for
    the repo: without the source head there is nothing to judge a
    copy against, and reporting the copies as anything else would be
    inventing evidence.
    """
    try:
        source_head = fetchers["source_head"](repo)
    except Exception as exc:
        detail = f"source head unavailable: {exc}"
        return None, {leg: result(FAILED, detail) for leg in (*GIT_LEGS, "r2_bundles")}
    legs = {}
    for leg, key in (
        ("plain_mirror", "mirror_head"),
        ("cloudflare_rehost", "rehost_head"),
    ):
        try:
            legs[leg] = judge_head_leg(source_head, fetchers[key](repo))
        except Exception as exc:
            legs[leg] = result(FAILED, f"check could not run: {exc}")
    try:
        legs["r2_bundles"] = ("bundle", fetchers["bundle"](repo))
    except Exception as exc:
        legs["r2_bundles"] = result(FAILED, f"check could not run: {exc}")
    return source_head, legs


def evaluate(repos, fetchers, now):
    """Run every comparison; returns the legs dict of the report."""
    per_repo = {}
    source_heads = {}
    for repo in repos:
        source_head, legs = evaluate_repo(repo, fetchers)
        source_heads[repo] = source_head
        bundle_entry = legs.get("r2_bundles")
        if isinstance(bundle_entry, tuple):
            legs["r2_bundles"] = judge_bundle_result(
                source_head, bundle_entry[1], now, SLO_HOURS["r2_bundles"]
            )
        per_repo[repo] = legs

    legs_report = {
        "source": {
            "status": rollup(
                FRESH if source_heads[repo] else FAILED for repo in repos
            ),
            "repos": {
                repo: (
                    result(FRESH, "source head observed", copy_head=source_heads[repo])
                    if source_heads[repo]
                    else result(FAILED, "source head unavailable")
                )
                for repo in repos
            },
        }
    }
    for leg in (*GIT_LEGS, "r2_bundles"):
        legs_report[leg] = {
            "slo_hours": SLO_HOURS[leg],
            "status": rollup(per_repo[repo][leg]["status"] for repo in repos),
            "repos": {repo: per_repo[repo][leg] for repo in repos},
        }
    try:
        d1 = judge_d1_meta(fetchers["d1_meta"](), now, SLO_HOURS["d1_export"])
    except Exception as exc:
        d1 = result(FAILED, f"check could not run: {exc}")
    legs_report["d1_export"] = {
        "slo_hours": SLO_HOURS["d1_export"],
        "status": d1["status"],
        "detail": d1["detail"],
        "repos": {},
    }
    if "exported_at" in d1:
        legs_report["d1_export"]["exported_at"] = d1["exported_at"]
    return legs_report


def build_report(repos, fetchers, now, run_id):
    legs = evaluate(repos, fetchers, now)
    verdict = (
        FRESH
        if all(leg["status"] == FRESH for leg in legs.values())
        else "attention"
    )
    return {
        "probe": PROBE_ID,
        "version": REPORT_VERSION,
        "run_id": run_id,
        "generated_at": iso(now),
        "inventory": list(repos),
        "legs": legs,
        "verdict": verdict,
    }


# ------------------------------------------------------------ CLI
def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="hosted_freshness.py",
        description="Hosted five-leg backup freshness comparison (#15001).",
    )
    parser.add_argument("--bucket", default=DEFAULT_R2_BUCKET)
    parser.add_argument("--rehost-base", default=DEFAULT_REHOST_BASE)
    parser.add_argument("--now", default=None, help="ISO 8601 override (tests)")
    parser.add_argument(
        "--run-id",
        default=None,
        help="evidence identity for this run (default: $GITHUB_RUN_ID or 'local')",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    if args.now:
        now = freshness_readout.parse_time(args.now, "--now")
        if now is None:
            print(f"hosted_freshness: unusable --now {args.now!r}", file=sys.stderr)
            return 2
    else:
        now = datetime.now(timezone.utc)
    run_id = args.run_id or os.environ.get("GITHUB_RUN_ID") or "local"
    fetchers = real_fetchers(args.bucket, args.rehost_base.rstrip("/"))
    report = build_report(FLEET_REPOS, fetchers, now, run_id)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["verdict"] == FRESH else 1


if __name__ == "__main__":
    sys.exit(main())
