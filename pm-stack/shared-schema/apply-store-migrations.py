#!/usr/bin/env python3
"""Apply the shared-schema migrations to one named store (#14256).

The app's migration action: the pm-store-migrate workflow dispatches
this script the way a deploy is started today. It applies the
PostgreSQL-dialect migrations in ``migrations/`` to a Turso store,
translated by ``turso_migrations.translate`` and recorded in the
store's ``pm_schema_migrations`` ledger; a migration that is already
recorded is a no-op this run reports as such.

Targets (exactly one):

- ``--store NAME``: a store from ``pm-stack/platform/stores.json``.
  The store's address comes from the inventory and the client pins
  the connection to it; the token comes from the environment variable
  named by ``--token-env`` (default ``STORE_TOKEN``) — the workflow
  maps the store to its repository secret, so the token value never
  appears in the workflow, the inventory, or this script's output.
- ``--local PATH``: a local SQLite file — the stand-in store the CI
  proof runs against.
- ``--url URL --expected-host HOST``: an explicit Turso origin,
  pinned to the given host, for stores not yet in the inventory.

``--dry-run`` translates the set and reports what would apply
(reading the ledger where the target allows) without writing.

Argument contract (#3352, doc/agents/operator-script-argument-contract.md):
the whole command line is declared below and parsed strictly with
argparse BEFORE any file read, secret lookup, or network request;
anything unaccounted for is a usage error with exit 2. Exit 0 means
every migration is applied (by this run or an earlier one); exit 1
means a migration failed or the target could not be reached, and the
verdict line names the failed migration.
"""

import argparse
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import turso_migrations as tm

STORES_FILE = Path(__file__).resolve().parents[1] / "platform" / "stores.json"


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="apply-store-migrations.py",
        allow_abbrev=False,
        description="Apply the shared-schema migrations to one store (#14256).",
    )
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument(
        "--store",
        metavar="NAME",
        help="a store named in pm-stack/platform/stores.json",
    )
    target.add_argument(
        "--local",
        metavar="PATH",
        help="a local SQLite file (the stand-in store)",
    )
    target.add_argument(
        "--url",
        metavar="URL",
        help="an explicit Turso HTTPS origin (requires --expected-host)",
    )
    parser.add_argument(
        "--expected-host",
        metavar="HOST",
        help="the host --url must name; required with --url, unused otherwise",
    )
    parser.add_argument(
        "--token-env",
        metavar="NAME",
        default="STORE_TOKEN",
        help="environment variable holding the store token (default: %(default)s)",
    )
    parser.add_argument(
        "--migrations-dir",
        metavar="DIR",
        default=str(tm.MIGRATIONS_DIR),
        help="directory of *.up.sql migrations (default: the sibling migrations/)",
    )
    parser.add_argument(
        "--stores-file",
        metavar="PATH",
        default=str(STORES_FILE),
        help="the store inventory for --store (default: pm-stack/platform/stores.json)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="translate and report what would apply; write nothing",
    )
    args = parser.parse_args(argv)
    if args.url and not args.expected_host:
        parser.error("--url requires --expected-host")
    if args.expected_host and not args.url:
        parser.error("--expected-host is only meaningful with --url")
    return args


def store_target(args) -> tuple[str, str, str]:
    """(display name, URL, expected host) for a --store target."""
    inventory = json.loads(Path(args.stores_file).read_text())
    for row in inventory.get("stores", []):
        if row.get("name") == args.store:
            address = row["address"]
            host = address.split("://", 1)[1].split("/")[0]
            return args.store, address, host
    raise tm.StoreError(f"store {args.store!r} is not in {args.stores_file}")


def build_client(args):
    """(display name, client) for the parsed target."""
    if args.local:
        return f"local:{args.local}", tm.LocalSqliteClient(args.local)
    if args.store:
        name, url, host = store_target(args)
    else:
        name, url, host = args.url, args.url, args.expected_host
    token = os.environ.get(args.token_env, "")
    if not token:
        raise tm.StoreError(
            f"no store token in ${args.token_env}; the workflow maps the "
            "store to its repository secret — an unset secret fails here, "
            "before any network request"
        )
    return name, tm.HttpTursoClient(url, token, host)


def dry_run(args, migrations) -> dict:
    translated = {}
    for migration_id, sql in migrations:
        translated[migration_id] = len(tm.translate(sql))
    already: list[str] = []
    if args.local and Path(args.local).exists():
        client = tm.LocalSqliteClient(args.local)
        try:
            rows = client.query(
                f"SELECT migration_id FROM {tm.LEDGER_TABLE}"
            )
            already = [row["migration_id"] for row in rows]
        except Exception:
            already = []
    return {
        "store": args.store or args.url or f"local:{args.local}",
        "dry_run": True,
        "statements_per_migration": translated,
        "already_applied": already,
        "would_apply": [m for m, _ in migrations if m not in already],
        "failed": None,
    }


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        migrations = tm.load_migrations(Path(args.migrations_dir))
        if args.dry_run:
            verdict = dry_run(args, migrations)
            print(json.dumps(verdict))
            return 0
        name, client = build_client(args)
        verdict = tm.apply_migrations(client, migrations)
        verdict = {"store": name, **verdict}
    except (tm.StoreError, tm.TranslationError, OSError, ValueError) as exc:
        print(json.dumps({"failed": {"migration": None, "error": str(exc)}}))
        print(f"apply-store-migrations: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(verdict))
    if verdict["failed"]:
        print(
            f"apply-store-migrations: migration "
            f"{verdict['failed']['migration']} failed: "
            f"{verdict['failed']['error']}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
