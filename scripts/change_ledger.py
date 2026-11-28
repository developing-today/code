#!/usr/bin/env python3
"""Check the pool change-event ledger against config changes (#1334 / #1339).

The steward spec (#1339, child of epic #1334) says the change-event
ledger is append-only and every decision is an event — timestamp, pool,
the change (PR link or proposal), rationale, expected effect, the
pool's window, and a quota snapshot before — and that "a config change
with no ledger event ... is a defect". This script is the check the
meta-reviewer (#1340) runs: given a ledger file and a list of config
changes in a window, it reports which changes have no matching event
and which events are malformed or incomplete.

Ledger format: JSONL, one event per line, fields per #1339:
  timestamp (ISO-8601), pool, change (PR link, "PR #N", or "commit <sha>"),
  rationale, expected_effect, window, before_snapshot (non-empty object).
  outcome (measured follow-up) is optional and appended later.

Changes file: JSON produced from the canonical config's history:
  {"config": ".opencode/opencode.jsonc",
   "window": {"since": "2026-10-27", "until": "2026-11-23"},
   "changes": [{"pr": 13838, "sha": "...", "date": "...", "title": "..."}]}
  One entry per merged PR touching the config in the window (commits
  grouped by PR via GET /repos/{owner}/{repo}/commits/{sha}/pulls);
  a direct-to-main commit is its own entry with "pr": null and its sha.
  Generate the commit list with:
    gh api "repos/developing-today/code/commits?path=.opencode/opencode.jsonc\
      &since=2026-10-27T00:00:00Z&per_page=100" --paginate \
      --jq '.[] | [.sha, .commit.author.date, (.commit.message | split("\\n")[0])] | @tsv'

Argument contract (#3352): the whole command line is declared here
(one subcommand, two required options); anything unaccounted for is a
usage error naming the offending argument, exit 2. Exit codes: 0 every
change matched and no defective event; 1 unmatched change or defective
event; 2 usage error or unreadable changes file.

Usage:
  python3 -I scripts/change_ledger.py check \
    --ledger ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/change-event-ledger.jsonl \
    --changes /tmp/config-changes.json
"""

import argparse
import json
import re
import sys
from datetime import datetime

REQUIRED_TEXT_FIELDS = ("timestamp", "pool", "change", "rationale",
                        "expected_effect", "window")
PR_RE = re.compile(r"(?:pull/|#)(\d+)")
SHA_RE = re.compile(r"\b[0-9a-f]{40}\b")


class Parser(argparse.ArgumentParser):
    """argparse that names unaccounted-for arguments per #3352."""

    def error(self, message):
        message = message.replace("unrecognized arguments",
                                  "unexpected argument")
        super().error(message)


def build_parser():
    parser = Parser(prog="change_ledger.py",
                    description="Check the pool change-event ledger "
                                "against config changes (#1339).")
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check", help="check a ledger against a "
                                         "changes file")
    check.add_argument("--ledger", required=True,
                       help="path to the ledger JSONL file")
    check.add_argument("--changes", required=True,
                       help="path to the changes JSON file")
    return parser


def _valid_timestamp(value):
    if not isinstance(value, str) or not value:
        return False
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return False
    return True


def _event_defects(line_no, event):
    """Defect dicts for one parsed event; [] when the event is complete."""
    defects = []
    for field in REQUIRED_TEXT_FIELDS:
        value = event.get(field)
        if field == "timestamp":
            if not _valid_timestamp(value):
                kind = ("missing_timestamp" if not value
                        else "bad_timestamp")
                defects.append({"line": line_no, "kind": kind,
                                "detail": "timestamp must be ISO-8601"})
        elif not isinstance(value, str) or not value.strip():
            defects.append({"line": line_no, "kind": "missing_" + field,
                            "detail": "event needs a non-empty " + field})
    snapshot = event.get("before_snapshot")
    if not isinstance(snapshot, dict) or not snapshot:
        defects.append({"line": line_no,
                        "kind": "missing_before_snapshot",
                        "detail": "event needs a non-empty before_snapshot "
                                  "object (quota snapshot before the change)"})
    refs = extract_refs(event.get("change"))
    if not refs:
        defects.append({"line": line_no, "kind": "unmatchable_change",
                        "detail": "change must reference a PR number "
                                  "(#N or pull/N) or a full commit SHA"})
    return defects


def extract_refs(change):
    """(pr_numbers, shas) referenced by a ledger event's change field."""
    if not isinstance(change, str):
        return set(), set()
    return set(int(n) for n in PR_RE.findall(change)), set(SHA_RE.findall(change))


def load_changes(path):
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)
    if not isinstance(doc, dict) or not isinstance(doc.get("changes"), list):
        raise ValueError("changes file must be an object with a "
                         "'changes' list")
    return doc


def check(ledger_path, changes_path):
    """Check the ledger; return the report dict (includes exit_code)."""
    doc = load_changes(changes_path)
    changes = doc["changes"]
    window = doc.get("window", {})
    config = doc.get("config", "")

    by_pr = {}
    by_sha = {}
    for idx, entry in enumerate(changes):
        if entry.get("pr") is not None:
            by_pr.setdefault(entry["pr"], idx)
        if entry.get("sha"):
            by_sha.setdefault(entry["sha"], idx)

    report = {
        "config": config,
        "window": window,
        "ledger": {"path": ledger_path, "missing": False, "events": 0},
        "total_changes": len(changes),
        "matched": 0,
        "unmatched": [],
        "defects": [],
        "outside_window_events": [],
    }

    try:
        with open(ledger_path, encoding="utf-8") as fh:
            lines = fh.readlines()
    except FileNotFoundError:
        report["ledger"]["missing"] = True
        report["unmatched"] = list(changes)
        report["exit_code"] = 1 if changes else 0
        return report

    matched_keys = set()
    for line_no, raw in enumerate(lines, start=1):
        if not raw.strip():
            continue
        try:
            event = json.loads(raw)
        except json.JSONDecodeError as exc:
            report["defects"].append(
                {"line": line_no, "kind": "malformed_event",
                 "detail": "not valid JSON: %s" % exc})
            continue
        if not isinstance(event, dict):
            report["defects"].append(
                {"line": line_no, "kind": "malformed_event",
                 "detail": "event must be a JSON object"})
            continue
        report["ledger"]["events"] += 1
        defects = _event_defects(line_no, event)
        report["defects"].extend(defects)
        if defects:
            # An incomplete event is not a ledger event for its change.
            continue
        prs, shas = extract_refs(event["change"])
        hit = False
        for pr in prs:
            if pr in by_pr:
                matched_keys.add(by_pr[pr])
                hit = True
        for sha in shas:
            if sha in by_sha:
                matched_keys.add(by_sha[sha])
                hit = True
        if not hit:
            report["outside_window_events"].append(
                {"line": line_no, "change": event["change"]})

    report["matched"] = len(matched_keys)
    report["unmatched"] = [c for i, c in enumerate(changes)
                           if i not in matched_keys]
    report["exit_code"] = 0 if (not report["unmatched"]
                                and not report["defects"]) else 1
    return report


def main(argv=None):
    args = build_parser().parse_args(argv)
    if args.command == "check":
        try:
            report = check(args.ledger, args.changes)
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            print("change_ledger.py: error: cannot read changes file "
                  "%s: %s" % (args.changes, exc), file=sys.stderr)
            return 2
        print(json.dumps(report, indent=2))
        print("change-event ledger: %d/%d config changes matched, "
              "%d defective event(s), %d event(s) outside the window"
              % (report["matched"], report["total_changes"],
                 len(report["defects"]),
                 len(report["outside_window_events"])), file=sys.stderr)
        return report["exit_code"]
    return 2


if __name__ == "__main__":
    sys.exit(main())
