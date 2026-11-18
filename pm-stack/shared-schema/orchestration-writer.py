#!/usr/bin/env python3
"""Writer for the #5005 orchestration-state cutover (phase 1 of #3142).

Implements the writer contract in pm-stack/shared-schema/ORCHESTRATION.md:
one transaction per orchestration pass, in this order:
  1. insert the orch_runs row (run id, generated-at, provenance, the
     complete document, item count) — idempotent per run_id;
  2. replace orch_items with the document's tiers, carrying
     first_seen_run_id forward for items that already existed;
  3. read back and verify: the stored document equals the file's
     document and the item counts agree (the export regeneration step
     is the pass's own file write; this read-back is its parity proof).

Transport: Turso fleet-platform Hrana, using Fleet secret resolution.
Statements and credentials stay in memory; failed batches roll back.
Usage: orchestration-writer.py [path-to-orchestration.json]
Exit 0 with a one-line JSON verdict on success; nonzero on any failure
(the files remain authoritative — a failed write changes nothing the
dashboard serves).

Argument contract (#3352, doc/agents/operator-script-argument-contract.md):
the whole command line is exactly one optional positional argument, the
orchestration JSON path. Arguments are parsed strictly (argparse) BEFORE
any subprocess, secret fetch, network request, or file read; any
unaccounted argument (extra positional, unknown flag) is a usage error
with exit 2.
"""

import argparse, json, os, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from fleet_turso import Turso


DEFAULT_PATH = os.path.expanduser(
    "~/workspace/goals/keep-the-four-repos-healthy/hidden_files/orchestration.json"
)


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="orchestration-writer.py",
        allow_abbrev=False,
        description="Write the orchestration document to the shared schema "
        "(#5005 / #3142).",
    )
    parser.add_argument(
        "path",
        nargs="?",
        default=DEFAULT_PATH,
        metavar="path-to-orchestration.json",
        help="orchestration JSON path (default: %(default)s).",
    )
    return parser.parse_args(argv)


def main():
    args = parse_args(sys.argv[1:])
    path = args.path
    with open(path) as f:
        doc = json.load(f)
    run_id = doc["runId"]
    generated_at = doc["generatedAt"]
    version = int(doc["version"])
    provenance = doc.get("provenance") or {}
    tiers = doc.get("tiers") or {}
    items = []
    for tier in ("needsDrew", "needsOrchestrator"):
        for item in tiers.get(tier) or []:
            items.append((tier, item))
    item_count = len(items)

    db = Turso()
    existing = db.query(
        "SELECT document,item_count FROM orch_runs WHERE run_id=?", [run_id]
    )
    expected_document = json.dumps(doc)
    if existing:
        if (
            len(existing) != 1
            or json.loads(existing[0]["document"]) != doc
            or existing[0]["item_count"] != item_count
        ):
            raise SystemExit("conflicting run-id replay; no writes attempted")
        # Compare JSON semantically in Python, then guard the exact observed text
        # inside the transaction. This accepts restored JSONB key ordering while
        # refusing any concurrent change between this read and the atomic write.
        expected_document = existing[0]["document"]
    stmts = [
        (
            "INSERT INTO orch_runs (run_id,generated_at,document_version,provenance,document,item_count) VALUES (?,?,?,?,?,?) ON CONFLICT(run_id) DO NOTHING",
            [run_id, generated_at, version, provenance, doc, item_count],
        )
    ]
    # Upsert before removing stale rows; preserve first_seen atomically.
    for tier, item in items:
        stmts.append(
            (
                "INSERT INTO orch_items (id,tier,kind,title,source,state,reason,ref,since,last_seen_at,item,first_seen_run_id,last_seen_run_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET tier=excluded.tier,kind=excluded.kind,title=excluded.title,source=excluded.source,state=excluded.state,reason=excluded.reason,ref=excluded.ref,since=excluded.since,last_seen_at=excluded.last_seen_at,item=excluded.item,last_seen_run_id=excluded.last_seen_run_id",
                [
                    item["id"],
                    tier,
                    item["kind"],
                    item["title"],
                    item["source"],
                    item.get("state"),
                    item["reason"],
                    item.get("ref"),
                    item["since"],
                    item["lastSeenAt"],
                    item,
                    run_id,
                    run_id,
                ],
            )
        )
    # Exact document membership, including repeat run IDs with a changed item set.
    ids = [item["id"] for _, item in items]
    stmts.append(
        (
            "DELETE FROM orch_items WHERE id NOT IN (SELECT value FROM json_each(?))",
            [ids],
        )
    )
    # Conflicting run-id replay aborts the transaction, never changes the snapshot.
    stmts.append(
        (
            "INSERT INTO orch_runs(run_id,generated_at,document_version,provenance,document,item_count) SELECT run_id,generated_at,document_version,provenance,document,item_count FROM orch_runs WHERE run_id=? AND document != ?",
            [run_id, expected_document],
        )
    )
    db.batch(stmts, transaction=True)
    runs = db.query(
        "SELECT item_count,document FROM orch_runs WHERE run_id=?", [run_id]
    )
    if (
        len(runs) != 1
        or runs[0]["item_count"] != item_count
        or json.loads(runs[0]["document"]) != doc
    ):
        raise SystemExit("read-back: stored run differs from the file document")
    counts = db.query("SELECT count(*) AS n FROM orch_items")
    if counts[0]["n"] != item_count:
        raise SystemExit("read-back: item count differs from file document")
    print(json.dumps({"ok": True, "run_id": run_id, "items": item_count}))


if __name__ == "__main__":
    main()
