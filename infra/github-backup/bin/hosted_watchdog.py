#!/usr/bin/env python3
"""Missed-run watchdog for the hosted freshness probe (#15001).

Slices 1-2 of #15001 made the probe report and alert on stale copies,
but a probe that silently stops running produces no report and no
alerts at all — the exact silent-failure shape #15001 exists to
remove. This module is the watchdog for the probe itself: it judges
whether the probe has run within its cadence and raises a distinct
probe-missed alert when it has not.

The evidence lives outside the probe's failure domain. Every probe
run publishes its report as a heartbeat object in R2 (Cloudflare's
domain, not GitHub's) at HEARTBEAT_KEY in the probe bucket. The
watchdog reads only that heartbeat — never the probe workflow's job
status — so a probe whose schedule was dropped, whose workflow was
disabled, or whose runs die before the report step is detected from
missing evidence, and the evidence itself remains readable by any
checker that can reach R2 even while GitHub is down. The watchdog
judgement is one of:

- ok:     the newest heartbeat is within PROBE_SLO_HOURS of now.
- missed: no heartbeat exists, or the newest heartbeat is older than
          the cadence SLO. This is the probe-missed signal, kept
          distinct from every leg-stale signal slice 2 files.
- failed: the watchdog's own read could not run (transport/API error,
          an unusable heartbeat record, a heartbeat timestamped in
          the future). A failed read is never read as a miss: R2
          being unreachable says nothing about the probe.

Alerting reuses slice 2's deduplication scheme (the machine-readable
marker in the alert body) under leg "probe", repo "-": exactly one
open probe-missed alert exists at a time. A first miss creates it, a
continuing miss with unchanged evidence is a noop, a miss whose
last-seen run changed updates it, and a fresh heartbeat resolves it.
Without --apply the module is a dry run: it prints its judgement and
the planned actions as JSON and touches nothing. With --apply it
executes the plan through `gh api`, with the executor injectable for
tests. Credentials arrive via the environment only (rclone's r2bak
remote, gh's own auth); this module never reads credential files and
never prints a credential.

Command line: [--bucket NAME] [--now ISO] [--run-id ID]
[--existing FILE] [--repo OWNER/NAME] [--apply]. Arguments are
parsed strictly before any file or network access; anything
unaccounted for is a usage error (exit 2, #3352). Exit 0 when the
probe is within cadence and no miss is unalerted, 1 when the probe
is missed or the watchdog read failed, 2 on usage error.
"""

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import freshness_readout
import hosted_alerts as ha
import hosted_freshness as hf

WATCHDOG_ID = "hosted-freshness-watchdog:v1"
PROBE_LEG = "probe"

# The probe runs daily; 26h absorbs schedule jitter and still catches
# a fully missed day the same morning — the same reasoning as the
# per-leg SLOs in hosted_freshness.SLO_HOURS, restated for the probe's
# own cadence so it is independently tunable.
PROBE_SLO_HOURS = 26

# The heartbeat is the probe report itself, published by the probe
# workflow to this key in the probe bucket after every run. The
# workflow file names the same key; keep the two in step.
HEARTBEAT_KEY = "github-backup/hosted-freshness/heartbeat.json"

OK = "ok"
MISSED = "missed"
FAILED = "failed"

FUTURE_SKEW = timedelta(minutes=5)
SUBPROCESS_TIMEOUT_SECONDS = 120


def judge_heartbeat(heartbeat, now, slo_hours=PROBE_SLO_HOURS):
    """Pure judgement: heartbeat record (or None) -> verdict dict.

    The heartbeat is a hosted_freshness report; only its probe id,
    run identity, and generation time are judged here — the leg
    contents are slice 1/2's concern, not the watchdog's.
    """
    if heartbeat is None:
        return {
            "status": MISSED,
            "detail": "no probe heartbeat exists in R2; "
                      "the probe has never published a run record",
            "last_run_id": None,
            "last_generated_at": None,
            "age_hours": None,
        }
    if heartbeat.get("probe") != hf.PROBE_ID:
        return {
            "status": FAILED,
            "detail": f"heartbeat is not a {hf.PROBE_ID} record: "
                      f"probe={heartbeat.get('probe')!r}",
            "last_run_id": heartbeat.get("run_id"),
            "last_generated_at": heartbeat.get("generated_at"),
            "age_hours": None,
        }
    raw = heartbeat.get("generated_at")
    generated = freshness_readout.parse_time(raw, "heartbeat generated_at") if raw else None
    if generated is None:
        return {
            "status": FAILED,
            "detail": f"heartbeat has no usable generated_at: {raw!r}",
            "last_run_id": heartbeat.get("run_id"),
            "last_generated_at": raw,
            "age_hours": None,
        }
    if generated > now + FUTURE_SKEW:
        return {
            "status": FAILED,
            "detail": f"heartbeat generated_at is in the future: {raw}",
            "last_run_id": heartbeat.get("run_id"),
            "last_generated_at": raw,
            "age_hours": None,
        }
    age = now - generated
    age_hours = round(age.total_seconds() / 3600, 2)
    evidence = {
        "last_run_id": heartbeat.get("run_id"),
        "last_generated_at": raw,
        "age_hours": age_hours,
    }
    if age > timedelta(hours=slo_hours):
        return {
            "status": MISSED,
            "detail": f"newest probe heartbeat is {age} old, beyond the "
                      f"{slo_hours}h cadence SLO (last run {raw})",
            **evidence,
        }
    return {
        "status": OK,
        "detail": f"newest probe heartbeat is {age} old, within the "
                  f"{slo_hours}h cadence SLO",
        **evidence,
    }


def evidence_run(judgement):
    """The dedup evidence key: the last probe run the alert records.

    A probe that has never run has no run id; the literal "none" is
    recorded instead so re-planning an unchanged never-ran miss is a
    noop, exactly like an unchanged stale-run miss.
    """
    return judgement["last_run_id"] or "none"


def alert_title():
    return "[backup-freshness] probe is missed (#15001)"


def alert_body(judgement, watchdog_run_id, checked_at):
    lines = [
        ha.marker(PROBE_LEG, ha.GLOBAL_REPO),
        "",
        "The hosted freshness probe itself has **missed** its cadence: "
        "no run record within the probe SLO was found in R2. This is "
        "the probe-missed signal — distinct from a leg-stale alert: "
        "the copies may be fine, but nothing is checking them.",
        "",
        f"- status: {MISSED}",
        f"- detail: {judgement['detail']}",
        # Named last_probe_run, not last_run_id: the dedup scan
        # (hosted_alerts.RUN_RE) takes the first "run_id: " match in
        # the body, and it must land on the evidence line below.
        f"- last_probe_run: {judgement['last_run_id']}",
        f"- last_generated_at: {judgement['last_generated_at']}",
        f"- age_hours: {judgement['age_hours']}",
        f"- slo_hours: {PROBE_SLO_HOURS}",
        f"- run_id: {evidence_run(judgement)}",
        f"- watchdog_run_id: {watchdog_run_id}",
        f"- checked_at: {checked_at}",
        "",
        "This alert is maintained by hosted_watchdog.py (#15001): it "
        "is updated in place while the probe stays missed and closed "
        "automatically when a fresh probe heartbeat is observed.",
    ]
    return "\n".join(lines) + "\n"


def plan_watchdog_actions(judgement, issues, watchdog_run_id, checked_at):
    """Pure planner: (judgement, existing issues) -> action list.

    Mirrors hosted_alerts.plan_actions semantics for the single
    (probe, -) key, so exactly one open probe-missed alert exists.
    A failed watchdog read plans nothing: it proves nothing about
    the probe.
    """
    existing = ha.parse_existing(issues)
    current = existing.get((PROBE_LEG, ha.GLOBAL_REPO))
    status = judgement["status"]
    if status == FAILED:
        return []
    if status == OK:
        if current is None:
            return []
        return [{
            "action": "resolve",
            "leg": PROBE_LEG,
            "repo": ha.GLOBAL_REPO,
            "issue": current["number"],
            "status": status,
            "run_id": judgement["last_run_id"],
            "comment": (
                f"Resolved by hosted watchdog run {watchdog_run_id} "
                f"({checked_at}): probe heartbeat observed from run "
                f"{judgement['last_run_id']} ({judgement['last_generated_at']}), "
                f"within the {PROBE_SLO_HOURS}h cadence SLO."
            ),
        }]
    # missed
    if current is None:
        return [{
            "action": "create",
            "leg": PROBE_LEG,
            "repo": ha.GLOBAL_REPO,
            "status": status,
            "run_id": judgement["last_run_id"],
            "title": alert_title(),
            "body": alert_body(judgement, watchdog_run_id, checked_at),
        }]
    recorded = ha.RUN_RE.search(current.get("body") or "")
    if recorded and recorded.group("run") == evidence_run(judgement):
        return [{
            "action": "noop",
            "leg": PROBE_LEG,
            "repo": ha.GLOBAL_REPO,
            "issue": current["number"],
            "status": status,
            "run_id": judgement["last_run_id"],
        }]
    return [{
        "action": "update",
        "leg": PROBE_LEG,
        "repo": ha.GLOBAL_REPO,
        "issue": current["number"],
        "status": status,
        "run_id": judgement["last_run_id"],
        "title": alert_title(),
        "body": alert_body(judgement, watchdog_run_id, checked_at),
    }]


# ------------------------------------------------------------ fetching
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


def fetch_heartbeat(bucket):
    """Newest probe heartbeat from R2, or None when no object exists."""
    try:
        out = _run(["rclone", "cat", f"r2bak:{bucket}/{HEARTBEAT_KEY}"])
    except RuntimeError as exc:
        if "not found" in str(exc).lower() or "404" in str(exc):
            return None
        raise
    return json.loads(out)


def real_fetcher(bucket):
    return lambda: fetch_heartbeat(bucket)


# ------------------------------------------------------------ CLI
def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="hosted_watchdog.py",
        description="Missed-run watchdog for the hosted freshness probe (#15001).",
    )
    parser.add_argument("--bucket", default=hf.DEFAULT_R2_BUCKET)
    parser.add_argument("--now", default=None, help="ISO 8601 override (tests)")
    parser.add_argument(
        "--run-id",
        default=None,
        help="evidence identity for this watchdog run "
             "(default: $GITHUB_RUN_ID or 'local')",
    )
    parser.add_argument("--existing", default=None,
                        help="JSON file of existing open issues "
                             "(default: fetch via gh api when --apply)")
    parser.add_argument("--repo", default="developing-today/code",
                        help="repository that hosts the alert issues")
    parser.add_argument("--apply", action="store_true",
                        help="execute the plan via gh api (default: dry-run plan only)")
    return parser.parse_args(argv)


def run(now, watchdog_run_id, fetch_heartbeat_fn, issues, apply, repo):
    """One watchdog pass; returns (readout, exit_code). Injectable seams
    (heartbeat fetcher, existing issues, executor via hosted_alerts)
    keep the whole pass hermetically testable."""
    checked_at = hf.iso(now)
    try:
        heartbeat = fetch_heartbeat_fn()
    except Exception as exc:
        judgement = {
            "status": FAILED,
            "detail": f"heartbeat read could not run: {exc}",
            "last_run_id": None,
            "last_generated_at": None,
            "age_hours": None,
        }
        actions = []
    else:
        judgement = judge_heartbeat(heartbeat, now)
        actions = plan_watchdog_actions(
            judgement, issues, watchdog_run_id, checked_at)
    if apply and actions:
        actions = ha.apply_actions(actions, ha.gh_executor(repo))
    readout = {
        "watchdog": WATCHDOG_ID,
        "run_id": watchdog_run_id,
        "checked_at": checked_at,
        "slo_hours": PROBE_SLO_HOURS,
        **judgement,
        "actions": actions,
    }
    exit_code = 0 if judgement["status"] == OK else 1
    if any(a["action"] in ("create", "update") for a in actions):
        exit_code = 1
    return readout, exit_code


def main(argv=None):
    args = parse_args(argv)
    if args.now:
        now = freshness_readout.parse_time(args.now, "--now")
        if now is None:
            print(f"hosted_watchdog: unusable --now {args.now!r}", file=sys.stderr)
            return 2
    else:
        now = datetime.now(timezone.utc)
    watchdog_run_id = args.run_id or os.environ.get("GITHUB_RUN_ID") or "local"
    if args.existing:
        try:
            with open(args.existing, encoding="utf-8") as handle:
                issues = json.load(handle)
        except (OSError, json.JSONDecodeError) as exc:
            print(f"hosted_watchdog: unusable --existing {args.existing!r}: {exc}",
                  file=sys.stderr)
            return 2
    elif args.apply:
        issues = ha.fetch_existing(args.repo)
    else:
        issues = []
    readout, exit_code = run(
        now, watchdog_run_id, real_fetcher(args.bucket),
        issues, args.apply, args.repo,
    )
    print(json.dumps(readout, indent=2, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
