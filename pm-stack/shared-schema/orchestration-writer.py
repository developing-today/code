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

Transport: the standing Supabase Management API SQL route (the same
route domain_append_pg.py uses) with secrets fetched by name via
`fleet secrets get` — values are never printed or written to disk.

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

import argparse, json, os, subprocess, sys, tempfile


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


# Strict parse first: a usage error (exit 2) must precede every secret
# fetch, subprocess, network request, and file read below.
ARGS = parse_args(sys.argv[1:])


def secret(name):
    fleet = "/home/hatch/tools/bin/fleet"
    if not os.path.exists(fleet):
        fleet = "fleet"
    out = subprocess.run(
        [fleet, "secrets", "get", name], capture_output=True, text=True
    )
    if out.returncode != 0:
        raise SystemExit(f"secret fetch failed: {name}")
    return out.stdout.strip()


REF = secret("supabase.muse-dev2.project-ref")
TOKEN = secret("supabase.muse-dev2.access-token-muse-cli")
URL = f"https://api.supabase.com/v1/projects/{REF}/database/query"


def sql(query):
    fd, tmp = tempfile.mkstemp(suffix=".json")
    with os.fdopen(fd, "w") as f:
        json.dump({"query": query}, f)
    try:
        out = subprocess.run(
            [
                "curl",
                "-sS",
                "-m",
                "120",
                "-A",
                "Mozilla/5.0",
                "-X",
                "POST",
                "-H",
                f"Authorization: Bearer {TOKEN}",
                "-H",
                "Content-Type: application/json",
                "--data",
                "@" + tmp,
                URL,
                "-w",
                "\n%{http_code}",
            ],
            capture_output=True,
            text=True,
        )
    finally:
        os.unlink(tmp)
    if out.returncode != 0:
        raise SystemExit(f"curl failed: {out.stderr[:200]}")
    body, _, code = out.stdout.rpartition("\n")
    if int(code) >= 300:
        raise SystemExit(f"SQL failed (HTTP {code}): {body[:400]}")
    return json.loads(body) if body.strip() else []


def lit(value):
    if value is None:
        return "NULL"
    return "'" + str(value).replace("'", "''") + "'"


def jsonb(obj):
    return lit(json.dumps(obj)) + "::jsonb"


def main():
    path = ARGS.path
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

    prior = sql("SELECT id, first_seen_run_id FROM public.orch_items")
    first_seen = {r["id"]: r["first_seen_run_id"] for r in prior}

    stmts = ["BEGIN;"]
    stmts.append(
        "INSERT INTO public.orch_runs (run_id, generated_at, document_version, provenance, document, item_count) VALUES ("
        + ", ".join(
            [
                lit(run_id),
                lit(generated_at),
                str(version),
                jsonb(provenance),
                jsonb(doc),
                str(item_count),
            ]
        )
        + ") ON CONFLICT (run_id) DO NOTHING;"
    )
    stmts.append("DELETE FROM public.orch_items;")
    for tier, item in items:
        fsr = first_seen.get(item["id"], run_id)
        stmts.append(
            "INSERT INTO public.orch_items (id, tier, kind, title, source, state, reason, ref, since, last_seen_at, item, first_seen_run_id, last_seen_run_id) VALUES ("
            + ", ".join(
                [
                    lit(item["id"]),
                    lit(tier),
                    lit(item["kind"]),
                    lit(item["title"]),
                    lit(item["source"]),
                    lit(item.get("state")),
                    lit(item["reason"]),
                    lit(item.get("ref")),
                    lit(item["since"]),
                    lit(item["lastSeenAt"]),
                    jsonb(item),
                    lit(fsr),
                    lit(run_id),
                ]
            )
            + ");"
        )
    stmts.append("COMMIT;")
    sql("\n".join(stmts))

    runs = sql(
        "SELECT run_id, item_count, document FROM public.orch_runs WHERE run_id = "
        + lit(run_id)
    )
    if len(runs) != 1:
        raise SystemExit(f"read-back: run row missing for {run_id}")
    if runs[0]["item_count"] != item_count:
        raise SystemExit(
            f"read-back: item_count {runs[0]['item_count']} != {item_count}"
        )
    if runs[0]["document"] != doc:
        raise SystemExit("read-back: stored document differs from the file document")
    counts = sql("SELECT count(*) AS n FROM public.orch_items")
    if counts[0]["n"] != item_count:
        raise SystemExit(
            f"read-back: orch_items has {counts[0]['n']} rows, expected {item_count}"
        )
    print(json.dumps({"ok": True, "run_id": run_id, "items": item_count}))


if __name__ == "__main__":
    main()
