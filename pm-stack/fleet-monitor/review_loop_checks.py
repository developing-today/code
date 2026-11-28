#!/usr/bin/env python3
"""Review-loop monitor checks — design #2025 §7.3, implemented for #2312.

The fleet monitor reconciles queue<->GitHub divergence on its hourly
pass; the review loop adds three checks to that duty, all REPORT-ONLY
at Stage B (no check here takes an action; the one repair §7.3 names —
correcting a diverged pass label from the authoritative packet
payload — is computed and reported as an exact repair instruction for
the monitor agent to apply and note, never applied by this module):

1. Verdict without packet: a verdict agent-note (footer
   `state=verdict verdict=second-pass pass=<n>`) or a
   `review/second-pass` label with no live second-pass (or
   interim-form) packet for that artifact -> protocol gap, handled
   like lifecycle-protocol §6.4(a).
2. Count divergence: label `review/pass-<n>` != packet payload
   `review.pass` != latest verdict-footer pass -> flag, with the
   payload value authoritative and the label repair spelled out.
   Plus §7.4's cousin: a `review/*` label surviving artifact closure
   is a finding (the artifact left the loop without a terminal
   verdict).
3. Stale pass: a commissioned pass packet unclaimed past the queue's
   normal claim latency, or a claimed pass whose artifact is still
   unverdicted past its effort-class deadline -> tracker flag, as for
   any assigned item.

Inputs are plain dicts so the checks are testable without GitHub or
the queue (see test_review_loop_checks.py, including the planted
divergence acceptance case from #2312):

  artifact: {"repo", "number", "kind": "issue"|"pr",
             "state": "open"|"closed", "labels": [str],
             "verdicts": [{"pass": int, "cap": int, "at": iso8601}]}
             (verdicts parsed from agent-note footers in comments)
  packet:   {"task_id", "status", "issue": int,
             "review_pass": int|None, "review_pass_cap": int|None,
             "effort": "xs"|"s"|"m"|"l"|None,
             "created_at": iso8601, "claimed_at": iso8601|None}

CLI:
  review_loop_checks.py --fixture state.json [--now ISO]
  review_loop_checks.py --repo owner/repo [--queue repo-health-code-pilot]

Live mode pulls artifacts carrying review/* labels (plus their
verdict footers) via `gh`, and live packets via `fleet queue list`,
then prints findings as JSON lines. Exit 0 always (report-only);
findings appear in the output only when they fire.
"""

import argparse
import json
import re
import subprocess
import sys
from datetime import datetime, timezone

LIVE_PACKET_STATUSES = {"queued", "claimed", "parked"}
DEFAULT_CLAIM_LATENCY_SECONDS = 3600  # lanes fire every 40 minutes
EFFORT_DEADLINE_SECONDS = {
    "xs": 2 * 3600,
    "s": 4 * 3600,
    "m": 24 * 3600,
    "l": 72 * 3600,
}
DEFAULT_EFFORT = "m"

VERDICT_FOOTER_RE = re.compile(
    r"<!--\s*agent-note:v1\s+[^>]*?state=verdict\s+verdict=second-pass\s+"
    r"pass=(?P<pass>\d+)\s+cap=(?P<cap>\d+)\s+at=(?P<at>\S+?)\s*-->"
)
PASS_LABEL_RE = re.compile(r"^review/pass-(\d+)$")


def parse_ts(value):
    if value is None:
        return None
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        # Fixtures and some producers emit naive ISO timestamps; the
        # loop's timestamps are UTC by convention, and a naive value
        # must not crash the age arithmetic in check_stale_pass.
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def parse_verdict_footers(comment_bodies):
    """Extract verdict records from agent-note footers in comments."""
    verdicts = []
    for body in comment_bodies or []:
        for match in VERDICT_FOOTER_RE.finditer(body or ""):
            verdicts.append(
                {
                    "pass": int(match.group("pass")),
                    "cap": int(match.group("cap")),
                    "at": match.group("at"),
                }
            )

    def sort_key(v):
        parsed = parse_ts(v["at"])
        return (
            parsed if parsed is not None else datetime.min.replace(tzinfo=timezone.utc)
        )

    verdicts.sort(key=sort_key)
    return verdicts


def label_pass(labels):
    for label in labels or []:
        match = PASS_LABEL_RE.match(label)
        if match:
            return int(match.group(1))
    return None


def live_packets_for(packets, issue_number):
    return [
        p
        for p in packets
        if p.get("issue") == issue_number and p.get("status") in LIVE_PACKET_STATUSES
    ]


def check_verdict_without_packet(artifacts, packets):
    """§7.3.1 — a second-pass verdict (or label) with no live packet."""
    findings = []
    for artifact in artifacts:
        # Only review packets commission a pass; an unrelated live
        # packet for the same issue must not mask a protocol gap.
        live = [
            p
            for p in live_packets_for(packets, artifact["number"])
            if p.get("review_pass") is not None
        ]
        live_passes = {p.get("review_pass") for p in live}
        outstanding = [
            v for v in artifact.get("verdicts", []) if v["pass"] not in live_passes
        ]
        has_label = "review/second-pass" in (artifact.get("labels") or [])
        if outstanding or (has_label and not live):
            findings.append(
                {
                    "check": "verdict_without_packet",
                    "repo": artifact["repo"],
                    "number": artifact["number"],
                    "kind": artifact["kind"],
                    "detail": (
                        "second-pass verdict/label present but no live "
                        "second-pass packet commissions it (protocol gap, "
                        "lifecycle-protocol 6.4(a) handling)"
                    ),
                    "verdict_passes_without_packet": [v["pass"] for v in outstanding],
                    "has_second_pass_label": has_label,
                    "live_packet_task_ids": [p["task_id"] for p in live],
                }
            )
    return findings


def check_count_divergence(artifacts, packets):
    """§7.3.2 + §7.4 — the three pass records must agree; the payload
    is authoritative; a review/* label on a closed artifact is a
    finding of its own."""
    findings = []
    for artifact in artifacts:
        labels = artifact.get("labels") or []
        if artifact.get("state") == "closed":
            surviving = [l for l in labels if l.startswith("review/")]
            if surviving:
                findings.append(
                    {
                        "check": "review_label_survives_closure",
                        "repo": artifact["repo"],
                        "number": artifact["number"],
                        "kind": artifact["kind"],
                        "detail": (
                            "review/* label(s) survived closure: the artifact "
                            "left the loop without a terminal verdict (7.4)"
                        ),
                        "labels": surviving,
                    }
                )
            continue
        by_label = label_pass(labels)
        pass_labels = [l for l in labels if PASS_LABEL_RE.match(l)]
        live = [
            p
            for p in live_packets_for(packets, artifact["number"])
            if p.get("review_pass") is not None
        ]
        # With several live packets, the authoritative payload is the
        # highest commissioned pass (ties: latest created) — never an
        # arbitrary first row.
        authoritative_packet = None
        if live:
            authoritative_packet = max(
                live,
                key=lambda p: (p["review_pass"], p.get("created_at") or ""),
            )
        by_payload = (
            authoritative_packet.get("review_pass") if authoritative_packet else None
        )
        verdicts = artifact.get("verdicts", [])
        by_footer = verdicts[-1]["pass"] if verdicts else None
        present = {
            "label": by_label,
            "payload": by_payload,
            "verdict_footer": by_footer,
        }
        values = {v for v in present.values() if v is not None}
        if len(values) > 1 or len(pass_labels) > 1:
            finding = {
                "check": "count_divergence",
                "repo": artifact["repo"],
                "number": artifact["number"],
                "kind": artifact["kind"],
                "detail": "label pass / payload review.pass / verdict footer disagree",
                "values": present,
                "authoritative": by_payload,
            }
            if len(pass_labels) > 1:
                finding["pass_labels"] = pass_labels
            if by_payload is not None and (
                by_label != by_payload or len(pass_labels) > 1
            ):
                finding["repair"] = {
                    "action": "set_label",
                    "remove": [l for l in labels if PASS_LABEL_RE.match(l)],
                    "add": f"review/pass-{by_payload}",
                    "note": "repair from the packet payload (authoritative), per 7.3.2",
                }
            findings.append(finding)
    return findings


def check_stale_pass(packets, now, claim_latency_seconds=DEFAULT_CLAIM_LATENCY_SECONDS):
    """§7.3.3 — unclaimed past claim latency, or claimed but
    unverdicted past the effort-class deadline. `run_checks` supplies
    each packet's `verdicted` flag, computed pass-aware from the
    artifact's verdict footers: a packet whose issue has a verdict
    footer at or after the packet's pass is not stale. (Direct
    callers may preset `verdicted` on a packet; `run_checks` always
    recomputes it and never trusts a preset value.)"""
    findings = []
    for packet in packets:
        if packet.get("status") not in LIVE_PACKET_STATUSES:
            continue
        if packet.get("review_pass") is None:
            continue  # not a review-loop packet
        created = parse_ts(packet.get("created_at"))
        claimed = parse_ts(packet.get("claimed_at"))
        if packet["status"] == "queued" and created is not None:
            age = (now - created).total_seconds()
            if age > claim_latency_seconds:
                findings.append(
                    {
                        "check": "stale_pass",
                        "task_id": packet["task_id"],
                        "issue": packet["issue"],
                        "detail": "commissioned pass unclaimed past normal claim latency",
                        "age_seconds": int(age),
                        "claim_latency_seconds": claim_latency_seconds,
                        "tracker": "flag in assigned-tracker.md as for any assigned item",
                    }
                )
        elif packet["status"] == "claimed" and claimed is not None:
            effort = packet.get("effort") or DEFAULT_EFFORT
            deadline = EFFORT_DEADLINE_SECONDS.get(
                effort, EFFORT_DEADLINE_SECONDS[DEFAULT_EFFORT]
            )
            age = (now - claimed).total_seconds()
            if age > deadline and not packet.get("verdicted"):
                findings.append(
                    {
                        "check": "stale_pass",
                        "task_id": packet["task_id"],
                        "issue": packet["issue"],
                        "detail": "pass artifact unverdicted past its effort-class deadline",
                        "age_seconds": int(age),
                        "effort": effort,
                        "deadline_seconds": deadline,
                        "tracker": "flag in assigned-tracker.md as for any assigned item",
                    }
                )
    return findings


def run_checks(state, now=None):
    """Run all §7.3 checks over a {"artifacts": [...], "packets": [...]}
    state dict. Returns the findings list (empty = clean)."""
    now = now or datetime.now(timezone.utc)
    artifacts = state.get("artifacts", [])
    packets = state.get("packets", [])
    # Join verdicts onto COPIES of the packets: the checks are
    # report-only and must not mutate the caller's state (a preset
    # `verdicted` value in the input is recomputed, never trusted).
    # The join is pass-aware: only a verdict at or after the packet's
    # own pass marks it verdicted.
    max_verdict_pass = {}
    for artifact in artifacts:
        passes = [v["pass"] for v in artifact.get("verdicts", [])]
        if passes:
            max_verdict_pass[artifact["number"]] = max(passes)
    joined = []
    for packet in packets:
        joined_packet = dict(packet)
        highest = max_verdict_pass.get(packet.get("issue"))
        review_pass = packet.get("review_pass")
        joined_packet["verdicted"] = bool(
            highest is not None and review_pass is not None and highest >= review_pass
        )
        joined.append(joined_packet)
    findings = []
    findings += check_verdict_without_packet(artifacts, packets)
    findings += check_count_divergence(artifacts, packets)
    findings += check_stale_pass(joined, now)
    return findings


# --- live state collection (gh + fleet queue) -------------------------------


def _run(cmd):
    return subprocess.run(
        cmd, capture_output=True, text=True, check=True, timeout=60
    ).stdout


def _parse_task_table(out):
    """(task_id, dedupe_key) rows from `fleet queue list` table output.

    The bulk list has no JSON form (list --json is a scoped single-task
    exact read), so candidate review packets are located by dedupe key
    in the table and then read back exactly, one by one. Rows look like:
      task 22217  70  queued  2:18:26  repo-health:v1:second-pass:...  Title
    """
    rows = []
    for line in (out or "").splitlines():
        parts = line.split()
        if len(parts) >= 6 and parts[0] == "task" and parts[1].isdigit():
            rows.append((int(parts[1]), parts[5]))
    return rows


def _packet_from_record(rec):
    """Build a packet dict from one exact-read task record (or None)."""
    payload = rec.get("payload") or {}
    review = payload.get("review") or {}
    issue = (payload.get("issue") or {}).get("number")
    if issue is None or review.get("pass") is None:
        return None
    return {
        "task_id": rec["id"],
        "status": rec["status"],
        "issue": issue,
        "review_pass": review.get("pass"),
        "review_pass_cap": review.get("pass_cap"),
        "effort": None,
        "created_at": rec.get("created_at"),
        "claimed_at": rec.get("claimed_at"),
    }


def _collect_live_packets(queue):
    """Live review packets (second-pass / interim form) from the queue.

    The queue CLI's bulk list is a human table with single-status
    filtering only, and its --json form is a scoped single-task read;
    so: enumerate each live status, keep rows whose dedupe key marks a
    review packet (`:second-pass:`), and exact-read those tasks. The
    list limit is capped at 500 by the CLI (a larger limit exits 2);
    the pilot queues' live sets sit below that today.
    """
    packets = []
    for status in ("queued", "claimed", "parked"):
        out = _run(
            [
                "fleet",
                "queue",
                "list",
                "--queue",
                queue,
                "--status",
                status,
                "--limit",
                "500",
            ]
        )
        for task_id, dedupe_key in _parse_task_table(out):
            if ":second-pass:" not in dedupe_key:
                continue
            raw = _run(
                [
                    "fleet",
                    "queue",
                    "list",
                    "--queue",
                    queue,
                    "--task",
                    str(task_id),
                    "--json",
                ]
            )
            rec = (json.loads(raw or "{}") or {}).get("task") or {}
            packet = _packet_from_record(rec)
            if packet is not None:
                packets.append(packet)
    return packets


def collect_live_state(repo, queue):
    """Bounded live pull: only artifacts carrying review/* labels can
    be in the loop, and only live packets can commission a pass."""
    artifacts = []
    for kind, sub in (("issue", "issue"), ("pr", "pr")):
        out = _run(
            [
                "gh",
                sub,
                "list",
                "--repo",
                repo,
                "--state",
                "all",
                "--search",
                "label:review/second-pass OR label:review/pass-1 OR label:review/pass-2",
                "--json",
                "number,state,labels",
                "--limit",
                "50",
            ]
        )
        for row in json.loads(out or "[]"):
            labels = [l["name"] for l in row.get("labels", [])]
            comments = _run(
                [
                    "gh",
                    sub,
                    "view",
                    str(row["number"]),
                    "--repo",
                    repo,
                    "--json",
                    "comments",
                    "--jq",
                    "[.comments[].body]",
                ]
            )
            artifacts.append(
                {
                    "repo": repo,
                    "number": row["number"],
                    "kind": kind,
                    "state": row["state"].lower(),
                    "labels": labels,
                    "verdicts": parse_verdict_footers(json.loads(comments or "[]")),
                }
            )
    packets = _collect_live_packets(queue)
    # A packet's issue may carry no review/* label anymore (removed
    # after its verdict) and so be missing from the pull above; its
    # verdicts still decide staleness, so fetch verdict-only records
    # for packet issues not already covered.
    covered = {a["number"] for a in artifacts}
    for issue in sorted({p["issue"] for p in packets} - covered):
        comments = _run(
            [
                "gh",
                "issue",
                "view",
                str(issue),
                "--repo",
                repo,
                "--json",
                "comments",
                "--jq",
                "[.comments[].body]",
            ]
        )
        artifacts.append(
            {
                "repo": repo,
                "number": issue,
                "kind": "issue",
                "state": "open",
                "labels": [],
                "verdicts": parse_verdict_footers(json.loads(comments or "[]")),
            }
        )
    return {"artifacts": artifacts, "packets": packets}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", help="JSON state file (artifacts + packets)")
    parser.add_argument("--repo", help="owner/repo for live mode")
    parser.add_argument("--queue", default="repo-health-code-pilot")
    parser.add_argument("--now", help="ISO timestamp override (fixtures/tests)")
    args = parser.parse_args(argv)

    try:
        if args.fixture:
            with open(args.fixture, encoding="utf-8") as fh:
                state = json.load(fh)
        elif args.repo:
            state = collect_live_state(args.repo, args.queue)
        else:
            parser.error("one of --fixture or --repo is required")
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        # Report-only contract: a failed lookup emits an error record
        # and still exits 0, so one bad pull cannot lose the pass.
        print(
            json.dumps(
                {"check": "collection_error", "detail": f"{type(exc).__name__}: {exc}"},
                sort_keys=True,
            )
        )
        return 0
    now = parse_ts(args.now) if args.now else None
    findings = run_checks(state, now=now)
    for finding in findings:
        print(json.dumps(finding, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
