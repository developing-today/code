#!/usr/bin/env python3
"""ainventory-interpretation — the interpretation pass over gather batches
and the findings event store (issue code#1248, Phase 2 slice 2; slice
tracker #10040).

Two jobs, one operator surface, against the muse-dev2 `ainventory` schema:

  awaiting   List non-empty gather batches with no interpretation run yet
             (`ainventory.v_batches_awaiting_interpretation`). Read only.

  record     Record one completed interpretation pass for a batch: the
             batch note (an `ainventory.notes` row of kind 'agent_note'),
             the `ainventory.interpretation_runs` row saying what changed,
             the follow-up research trail, and any findings. These are
             separate PostgREST inserts, not one transaction: if a later
             request fails, earlier append-only rows remain and need
             operator review before retrying.

  import-findings
             Subsume entries from a permanent findings store file into
             `ainventory.findings` (#1248's 2026-10-27 pointer: the
             pipeline consumes the store, it does not start a parallel
             one). Entries are never re-imported: the store path plus the
             entry heading is the identity, and an entry already present
             is skipped, not duplicated.

Append-only: the tables this writes are INSERT-only at the database level
(BEFORE UPDATE/DELETE triggers raise, UPDATE/DELETE grants revoked), so
re-interpreting a batch is a NEW interpretation run, never an edit. The
separate PostgREST requests are not transactional; after a failed record
command, inspect the batch's notes, findings, and runs before retrying.

Argument contract (repo #3352): the whole command line is declared below
and parsed strictly before any subprocess, network call or file write.
Unknown flags and stray positionals are usage errors naming the offending
argument; `--opt=value` and `--opt value` are identical; a present-but-
valueless option is a usage error, never the default. `--dry-run` is
honoured on every code path that would write.

Exit codes: 0 success, 1 operational failure (network, DB refusal, an
empty import), 2 usage error.

Secrets: the service-role key is read by name through `bws`
(`supabase.muse-dev2.service-role-key`), never printed, with
BWS_ACCESS_TOKEN honoured when set — the same lookup the gatherer uses.

Run:
  scripts/ainventory-interpretation.py awaiting
  scripts/ainventory-interpretation.py awaiting --limit 5 --json
  scripts/ainventory-interpretation.py record --batch <uuid> \
      --note-file /tmp/note.md --author repo-worker-code --model <model>
  scripts/ainventory-interpretation.py record --batch <uuid> --note-file n.md \
      --dry-run --json
  scripts/ainventory-interpretation.py import-findings \
      --file findings-export.json --dry-run
"""

import argparse
import json
import os
import re
import subprocess
import sys

REF = os.environ.get("AINVENTORY_SUPABASE_REF", "keihfchslgjetkmnmzra")
BASE = f"https://{REF}.supabase.co/rest/v1"
BWS = os.environ.get("BWS_BIN", "/run/current-system/sw/bin/bws")
SERVICE_ROLE_SECRET = "supabase.muse-dev2.service-role-key"
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36"
PROFILE = "ainventory"
NOTE_KIND = "agent_note"


# --------------------------------------------------------------- arguments
def build_parser():
    # --dry-run / --json live on a parent parser so they parse on either
    # side of the verb: `--dry-run record …` and `record … --dry-run` are
    # the same command line. An operator who writes the flag after the
    # verb must not silently lose the safety flag (#3352).
    common = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    common.add_argument(
        "--dry-run",
        action="store_true",
        default=argparse.SUPPRESS,
        help="parse everything and report what would be written, write nothing",
    )
    common.add_argument(
        "--json",
        action="store_true",
        default=argparse.SUPPRESS,
        help="machine-readable output on stdout",
    )

    parser = argparse.ArgumentParser(
        prog="ainventory-interpretation.py",
        allow_abbrev=False,
        parents=[common],
        description="Interpretation pass + findings store for the model "
        "data pipeline (code#1248 slice 2).",
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    awaiting = sub.add_parser(
        "awaiting",
        allow_abbrev=False,
        parents=[common],
        help="list non-empty batches with no interpretation run yet",
    )
    awaiting.add_argument(
        "--limit",
        type=positive_int,
        default=20,
        metavar="N",
        help="maximum rows to return (default: %(default)s)",
    )

    record = sub.add_parser(
        "record",
        allow_abbrev=False,
        parents=[common],
        help="record one interpretation run for a batch",
    )
    record.add_argument(
        "--batch", required=True, metavar="UUID", help="ingest batch id"
    )
    record.add_argument(
        "--note-file",
        required=True,
        metavar="PATH",
        help="file holding the batch note body (what changed, implications)",
    )
    record.add_argument("--author", required=True, metavar="WHO", help="agent identity")
    record.add_argument(
        "--model",
        default=None,
        metavar="MODEL",
        help="model that produced the note (recorded, not inferred)",
    )
    record.add_argument(
        "--followups",
        default=None,
        metavar="PATH",
        help="JSON file: [{model, task_id, issue_url, status, reason?}]",
    )
    record.add_argument(
        "--findings",
        default=None,
        metavar="PATH",
        help="JSON file of findings rows to attach to this batch",
    )

    imp = sub.add_parser(
        "import-findings",
        allow_abbrev=False,
        parents=[common],
        help="subsume a structured findings export into ainventory.findings",
    )
    imp.add_argument(
        "--file",
        required=True,
        metavar="PATH",
        help="JSON array exported from the canonical findings store",
    )
    imp.add_argument(
        "--author",
        default="findings-event-store",
        metavar="WHO",
        help="who the entries are attributed to (default: %(default)s)",
    )
    return parser


def positive_int(text):
    try:
        value = int(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not an integer: {text!r}")
    if value < 1:
        raise argparse.ArgumentTypeError(f"must be >= 1: {text!r}")
    return value


# --------------------------------------------------------------- transport
def service_role_key():
    env = dict(os.environ)
    if not env.get("BWS_ACCESS_TOKEN"):
        token_file = os.path.expanduser("~/.config/bitwarden/access-token")
        if os.path.exists(token_file):
            with open(token_file) as f:
                env["BWS_ACCESS_TOKEN"] = f.read().strip()
    listing = subprocess.run(
        [BWS, "secret", "list", "-o", "json"],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    ).stdout
    try:
        secret_id = next(
            s["id"] for s in json.loads(listing) if s["key"] == SERVICE_ROLE_SECRET
        )
    except StopIteration:
        raise SystemExit(f"secret {SERVICE_ROLE_SECRET} not found in Bitwarden")
    got = subprocess.run(
        [BWS, "secret", "get", secret_id, "-o", "json"],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    ).stdout
    return json.loads(got)["value"]


def rest(key, method, path, payload=None, prefer="return=representation"):
    args = [
        "curl",
        "-sS",
        "-m",
        "120",
        "-A",
        UA,
        "-H",
        f"apikey: {key}",
        "-H",
        f"Authorization: Bearer {key}",
    ]
    if method == "GET":
        args += ["-H", f"Accept-Profile: {PROFILE}"]
    else:
        args += [
            "-H",
            f"Content-Profile: {PROFILE}",
            "-H",
            "Content-Type: application/json",
            "-H",
            f"Prefer: {prefer}",
            "-X",
            method,
        ]
        if payload is not None:
            args += ["-d", json.dumps(payload)]
    args += [BASE + path, "-w", "\n%{http_code}"]
    out = subprocess.run(args, capture_output=True, text=True, check=True).stdout
    body, _, status = out.rpartition("\n")
    if int(status) >= 400:
        # A rejected write must never look like a landed one (the lesson
        # from the gatherer's silent-HTTP-400 bug, #9999).
        raise RuntimeError(f"{method} {path} -> HTTP {status}: {body[:400]}")
    return json.loads(body) if body.strip() else None


# ------------------------------------------------------------------ verbs
def cmd_awaiting(args):
    rows = rest(
        service_role_key(),
        "GET",
        "/v_batches_awaiting_interpretation"
        "?select=id,source_kind,source_ref,ran_at,inserted_counts,note"
        f"&order=ran_at.asc&limit={args.limit}",
    )
    if getattr(args, "json", False):
        print(json.dumps(rows, indent=2))
        return 0
    if not rows:
        print("no non-empty batch awaits interpretation")
        return 0
    print(f"{len(rows)} batch(es) awaiting interpretation:")
    for row in rows:
        counts = ", ".join(
            f"{k}={v}" for k, v in (row.get("inserted_counts") or {}).items()
        )
        print(f"  {row['id']}  {row['source_kind']:<12} {row['ran_at']}  [{counts}]")
        if row.get("source_ref"):
            print(f"      {row['source_ref']}")
    return 0


def load_json_file(path, what):
    try:
        with open(path) as f:
            return json.load(f)
    except OSError as e:
        raise SystemExit(f"cannot read {what} file {path}: {e}")
    except json.JSONDecodeError as e:
        raise SystemExit(f"{what} file {path} is not valid JSON: {e}")


def normalise_followups(entries):
    if entries is None:
        return []
    if not isinstance(entries, list):
        raise SystemExit("followups must be a JSON array")
    out = []
    for i, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise SystemExit(f"followups[{i}] is not an object")
        model = entry.get("model")
        status = entry.get("status")
        if not model:
            raise SystemExit(f"followups[{i}] has no model")
        if status not in ("filed", "not_filed"):
            raise SystemExit(
                f"followups[{i}] status must be 'filed' or 'not_filed' (got {status!r})"
            )
        if status == "not_filed" and not entry.get("reason"):
            raise SystemExit(
                f"followups[{i}] is not_filed without a reason; an unfiled "
                "follow-up is recorded with why"
            )
        out.append(entry)
    return out


def cmd_record(args):
    try:
        with open(args.note_file) as f:
            note_body = f.read().strip()
    except OSError as e:
        raise SystemExit(f"cannot read note file {args.note_file}: {e}")
    if not note_body:
        raise SystemExit(f"note file {args.note_file} is empty")
    followups = normalise_followups(
        load_json_file(args.followups, "followups") if args.followups else None
    )
    findings = load_json_file(args.findings, "findings") if args.findings else []
    if not isinstance(findings, list):
        raise SystemExit("findings must be a JSON array")

    finding_rows = []
    for index, finding in enumerate(findings):
        if not isinstance(finding, dict):
            raise SystemExit(f"findings[{index}] is not an object")
        if not finding.get("subject") or not finding.get("finding"):
            raise SystemExit(
                f"findings[{index}] requires non-empty subject and finding"
            )
        finding_rows.append(
            {
                "subject": finding["subject"],
                "finding": finding["finding"],
                "evidence": finding.get("evidence"),
                "significance": finding.get("significance"),
                "action": finding.get("action"),
                "severity": finding.get("severity"),
                "model_name": finding.get("model_name"),
                "provider": finding.get("provider"),
                "ingest_batch_id": args.batch,
                "source_kind": finding.get("source_kind", "agent"),
                "source_ref": finding.get("source_ref"),
                "author": args.author,
                "raw": finding,
            }
        )

    batch_path = f"/ingest_batches?id=eq.{args.batch}&select=id,inserted_counts"
    plan = {
        "batch": args.batch,
        "note_kind": NOTE_KIND,
        "note_chars": len(note_body),
        "followups": followups,
        "findings": len(findings),
        "author": args.author,
        "model": args.model,
    }

    if getattr(args, "dry_run", False):
        print(json.dumps({"dry_run": True, "would_record": plan}, indent=2))
        return 0

    key = service_role_key()
    batches = rest(key, "GET", batch_path)
    if not batches:
        raise SystemExit(f"no ingest batch {args.batch} in ainventory.ingest_batches")

    # Each PostgREST request commits independently. If a later request
    # fails, earlier append-only rows remain; the operator must inspect
    # this batch before retrying rather than assume rollback.
    note_row = rest(
        key,
        "POST",
        "/notes",
        [
            {
                "ingest_batch_id": args.batch,
                "kind": NOTE_KIND,
                "body": note_body,
                "author": args.author,
                "source_ref": f"interpretation of batch {args.batch}",
            }
        ],
    )
    note_id = note_row[0]["id"] if note_row else None

    if finding_rows:
        rest(key, "POST", "/findings", finding_rows, prefer="return=minimal")

    run_row = rest(
        key,
        "POST",
        "/interpretation_runs",
        [
            {
                "ingest_batch_id": args.batch,
                "note_id": note_id,
                "author": args.author,
                "model_used": args.model,
                "changed": {
                    "inserted_counts": batches[0].get("inserted_counts") or {},
                    "note_body_chars": len(note_body),
                },
                "new_models": [f.get("model") for f in followups],
                "followups": followups,
                "findings_count": len(finding_rows),
            }
        ],
    )

    result = {
        "batch": args.batch,
        "note_id": note_id,
        "interpretation_run_id": run_row[0]["id"] if run_row else None,
        "findings": len(finding_rows),
        "followups": len(followups),
    }
    print(
        json.dumps(result, indent=2)
        if getattr(args, "json", False)
        else (
            f"recorded interpretation run {result['interpretation_run_id']} for "
            f"batch {args.batch}: note {note_id}, {len(finding_rows)} finding(s), "
            f"{len(followups)} follow-up(s)"
        )
    )
    return 0


def parse_findings_store(text):
    """Read structured findings exported from the canonical store.

    The original Markdown source named in #1248 is not present on this
    execution host. Do not guess its Markdown grammar. Import accepts an
    explicit JSON array in the pointer's documented entry shape
    (subject, finding, evidence, significance, action); preserve the
    source file and entry heading in source_ref. This adapter is the
    deliberate boundary until the canonical file is available for a
    source-backed parser.
    """
    try:
        value = json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(
            "findings import expects a JSON array exported from the "
            "canonical store; Markdown parsing is deferred until its "
            f"source file is available ({e})"
        )
    if not isinstance(value, list):
        raise ValueError("findings import expects a JSON array")
    entries, malformed = [], 0
    for i, item in enumerate(value):
        if not isinstance(item, dict) or not item.get("finding"):
            malformed += 1
            continue
        entry = dict(item)
        entry["_heading"] = str(item.get("heading") or f"entry-{i + 1}")
        if not entry.get("subject"):
            entry["subject"] = entry["_heading"]
        entries.append(entry)
    return entries, malformed


def cmd_import_findings(args):
    try:
        with open(args.file) as f:
            text = f.read()
    except OSError as e:
        raise SystemExit(f"cannot read findings store {args.file}: {e}")
    try:
        entries, malformed = parse_findings_store(text)
    except ValueError as e:
        raise SystemExit(str(e))
    if not entries:
        raise SystemExit(
            f"no findings entries parsed from {args.file} "
            f"({malformed} malformed); nothing imported"
        )

    rows = [
        {
            "subject": e.get("subject") or e["_heading"],
            "finding": e["finding"],
            "evidence": e.get("evidence"),
            "significance": e.get("significance"),
            "action": e.get("action"),
            "severity": e.get("severity"),
            "model_name": e.get("model"),
            "provider": e.get("provider"),
            "source_kind": "findings-event-store",
            "source_ref": f"{args.file}#{e['_heading']}",
            "author": args.author,
            "raw": {k: v for k, v in e.items() if not k.startswith("_")},
        }
        for e in entries
    ]

    if getattr(args, "dry_run", False):
        print(
            json.dumps(
                {
                    "dry_run": True,
                    "file": args.file,
                    "parsed": len(rows),
                    "malformed": malformed,
                    "would_import": rows,
                },
                indent=2,
            )
        )
        return 0

    key = service_role_key()
    existing = rest(key, "GET", "/findings?select=source_ref&limit=5000") or []
    seen = {r.get("source_ref") for r in existing}
    fresh = [r for r in rows if r["source_ref"] not in seen]
    skipped = len(rows) - len(fresh)
    for i in range(0, len(fresh), 200):
        rest(key, "POST", "/findings", fresh[i : i + 200], prefer="return=minimal")

    print(
        json.dumps(
            {
                "file": args.file,
                "parsed": len(rows),
                "imported": len(fresh),
                "already_present": skipped,
                "malformed": malformed,
            },
            indent=2,
        )
        if getattr(args, "json", False)
        else (
            f"{args.file}: parsed {len(rows)} entr(ies), imported {len(fresh)}, "
            f"already present {skipped}, malformed {malformed}"
        )
    )
    return 0


def main(argv=None):
    args = build_parser().parse_args(argv)
    handler = {
        "awaiting": cmd_awaiting,
        "record": cmd_record,
        "import-findings": cmd_import_findings,
    }[args.verb]
    try:
        return handler(args)
    except (RuntimeError, subprocess.CalledProcessError) as e:
        print(f"ainventory-interpretation: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
