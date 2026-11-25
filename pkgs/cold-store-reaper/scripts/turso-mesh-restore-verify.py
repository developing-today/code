#!/usr/bin/env python3
"""Class-A first restore verification (#13455 slice 4, issue #14251).

The restore half of the backup-mesh class-A Turso leg (design
``doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md`` §9): a backup
is not verified until it has been restored and checked. The runner
slice (``turso-mesh-restic-runner.py``, #14247) files ONE restic
snapshot per class-A database (``fleet-queue``, ``fleet-platform``,
``atlas``) containing the zstd-compressed dump and the slice-1
``manifest.json`` together, tagged ``class-a``, ``mesh=turso-mesh``
and ``db=<name>``, against the dedicated ``restic-turso-mesh``
repository. This verifier is what the first restore verification
(#14251) runs against those snapshots once the live first-light
slice (#14250) has filed them; the code is testable end to end
against a fake-restic fixture repository, so the live run is a
configuration change, not new code.

Per database, independently (one database's failure is reported on
its own entry, never blocks the others, and fails the run at the
end):

1. Select the newest snapshot tagged ``class-a`` + ``db=<name>``
   (``restic snapshots --json``), or the snapshot named by
   ``--snapshot``.
2. Restore it with ``restic restore`` into an isolated scratch
   target ONLY. House rule: drills never write a production store.
   The scratch root must be a dedicated directory: the verifier
   refuses the filesystem root and the user's home directory, and
   refuses a per-database target that already exists non-empty
   (a previous drill's leftovers are evidence, not a target).
   The only restic subcommands issued are ``snapshots`` and
   ``restore`` — this tool cannot write to the repository or to
   any database.
3. Decompress the restored ``<db>.sql.zst`` through the decompressor
   seam (default ``zstd -d -q -c``) and check its sha256 against the
   manifest's ``dump_sha256``.
4. Rebuild a scratch SQLite file from the dump and recount it with
   the slice-1 manifest builder (``turso-mesh-class-a.py``
   ``build_manifest``, PR #13476): every table's row count AND
   content hash in the manifest must match the restored copy
   exactly, table for table.
5. For ``fleet-queue`` only, the restored copy must also answer a
   read-only queue query: the copy is opened ``mode=ro`` and a
   ``SELECT COUNT(*)`` runs against its queue table (the first
   table whose name mentions queue/task state, else the first
   table). The result is recorded in the report.

The report is printed as JSON: per database, the snapshot id, the
per-table manifest-vs-restored row counts with match flags, the
dump-hash check, and (for ``fleet-queue``) the queue-query result —
exactly the evidence #14251's acceptance asks to record on #13455.
The restic password is referenced by environment-variable NAME only
(``--password-env``, default ``RESTIC_PASSWORD``); the verifier
never reads, stores, or prints a credential value, and the
repository URL arrives by configuration only.

Operator script argument contract (#3352): the whole command line is
declared below and parsed strictly by argparse before any subprocess
or file write; unknown arguments are a usage error (exit 2).

Usage:

    turso-mesh-restore-verify.py verify [--db NAME ...] \
        --repo URL --scratch DIR [--snapshot ID] \
        [--restic-bin PATH] [--decompress-cmd 'zstd -d -q -c'] \
        [--password-env NAME]
"""

import argparse
import hashlib
import importlib.util
import json
import os
import shlex
import sqlite3
import subprocess
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent


def _load_class_a():
    spec = importlib.util.spec_from_file_location(
        "turso_mesh_class_a", _HERE / "turso-mesh-class-a.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


CLASS_A = _load_class_a()
CLASS_A_DATABASES = CLASS_A.CLASS_A_DATABASES

DEFAULT_REPOSITORY_NAME = "restic-turso-mesh"
SNAPSHOT_TAGS = ("class-a", "mesh=turso-mesh")
QUEUE_DATABASE = "fleet-queue"


def _run(cmd, **kw):
    return subprocess.run(cmd, **kw)


def select_snapshot(db, repo, restic_bin, env_extra, snapshot=None):
    """Return the snapshot id to restore for ``db``.

    With no explicit snapshot, the newest snapshot carrying the
    class-A and per-database tags wins (restic's ``time`` field is
    ISO-8601, so the maximum is the latest).
    """
    if snapshot:
        return snapshot
    cmd = [restic_bin, "--repo", repo, "snapshots", "--json"]
    for tag in SNAPSHOT_TAGS + ("db=%s" % db,):
        cmd += ["--tag", tag]
    proc = _run(cmd, env=env_extra, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError("restic snapshots failed (exit %d)" % proc.returncode)
    snaps = json.loads(proc.stdout or "[]")
    if not snaps:
        raise RuntimeError("no snapshot tagged db=%s in repository" % db)
    return max(snaps, key=lambda s: s.get("time", ""))["id"]


def check_scratch_target(scratch, db):
    """Return the per-db restore target, enforcing the scratch rules."""
    root = Path(scratch).resolve()
    if root == Path(root.anchor) or root == Path.home().resolve():
        raise RuntimeError(
            "scratch target %s is not an isolated scratch directory" % root
        )
    target = root / db
    if target.exists() and any(target.iterdir()):
        raise RuntimeError(
            "scratch target %s already exists and is not empty; "
            "refusing to restore over a previous drill's evidence" % target
        )
    target.mkdir(parents=True, exist_ok=True)
    return target


def restic_restore(snapshot_id, target, repo, restic_bin, env_extra):
    cmd = [
        restic_bin,
        "--repo",
        repo,
        "restore",
        snapshot_id,
        "--target",
        str(target),
    ]
    proc = _run(cmd, env=env_extra)
    if proc.returncode != 0:
        raise RuntimeError("restic restore failed (exit %d)" % proc.returncode)


def _find_one(target, name):
    hits = sorted(Path(target).rglob(name))
    if len(hits) != 1:
        raise RuntimeError(
            "expected exactly one %s in restored snapshot, found %d" % (name, len(hits))
        )
    return hits[0]


def decompress_dump(zst_path, out_path, decompress_cmd):
    with open(zst_path, "rb") as src, open(out_path, "wb") as dst:
        proc = subprocess.run(shlex.split(decompress_cmd), stdin=src, stdout=dst)
    if proc.returncode != 0:
        raise RuntimeError("decompressor failed (exit %d)" % proc.returncode)
    return out_path.read_bytes()


def rebuild_sqlite(dump_bytes, sqlite_path):
    if Path(sqlite_path).exists():
        Path(sqlite_path).unlink()
    conn = sqlite3.connect(str(sqlite_path))
    try:
        conn.executescript(dump_bytes.decode("utf-8"))
        conn.commit()
    finally:
        conn.close()
    return sqlite_path


def queue_query(sqlite_path):
    """Answer one read-only queue query against the restored copy."""
    conn = sqlite3.connect("file:%s?mode=ro" % sqlite_path, uri=True)
    try:
        tables = CLASS_A._table_names(conn)
        if not tables:
            raise RuntimeError("restored fleet-queue copy has no tables")
        table = next(
            (t for t in tables if "queue" in t or "task" in t), tables[0]
        )
        rows = conn.execute(
            "SELECT COUNT(*) FROM %s" % CLASS_A._quote_ident(table)
        ).fetchone()[0]
        return {"table": table, "rows": rows}
    finally:
        conn.close()


def verify_one(db, repo, scratch, restic_bin, decompress_cmd, env_extra,
               snapshot=None):
    """Verify one database's latest snapshot; returns its report entry."""
    entry = {"database": db, "status": "fail"}
    try:
        snapshot_id = select_snapshot(db, repo, restic_bin, env_extra, snapshot)
        entry["snapshot"] = snapshot_id
        target = check_scratch_target(scratch, db)
        restic_restore(snapshot_id, target, repo, restic_bin, env_extra)
        manifest = json.loads(_find_one(target, "manifest.json").read_text())
        if manifest.get("database") != db:
            raise RuntimeError(
                "manifest is for database %r, not %r" % (manifest.get("database"), db)
            )
        entry["exported_at"] = manifest.get("exported_at")
        dump_bytes = decompress_dump(
            _find_one(target, "%s.sql.zst" % db),
            target / ("%s.sql" % db),
            decompress_cmd,
        )
        entry["dump_sha256_match"] = (
            hashlib.sha256(dump_bytes).hexdigest() == manifest.get("dump_sha256")
        )
        sqlite_path = rebuild_sqlite(dump_bytes, target / ("%s.restored.db" % db))
        recount = CLASS_A.build_manifest(
            db, str(sqlite_path), dump_bytes, manifest.get("exported_at")
        )
        restored = {t["name"]: t for t in recount["tables"]}
        tables = []
        all_match = True
        for want in manifest["tables"]:
            got = restored.get(want["name"])
            match = (
                got is not None
                and got["rows"] == want["rows"]
                and got["sha256"] == want["sha256"]
            )
            all_match = all_match and match
            tables.append(
                {
                    "name": want["name"],
                    "manifest_rows": want["rows"],
                    "restored_rows": None if got is None else got["rows"],
                    "match": match,
                }
            )
        extra = sorted(set(restored) - {t["name"] for t in manifest["tables"]})
        if extra:
            all_match = False
            entry["extra_tables"] = extra
        entry["tables"] = tables
        if db == QUEUE_DATABASE:
            entry["queue_query"] = queue_query(sqlite_path)
        entry["status"] = (
            "pass" if (all_match and entry["dump_sha256_match"]) else "fail"
        )
        if entry["status"] != "pass":
            entry["error"] = "restored copy does not match the snapshot manifest"
    except Exception as exc:  # per-db isolation is the contract
        entry["error"] = str(exc)
    return entry


def verify(databases, repo, scratch, restic_bin="restic",
           decompress_cmd="zstd -d -q -c", password_env="RESTIC_PASSWORD",
           snapshot=None):
    """Verify each database independently; returns the full report."""
    if password_env not in os.environ:
        print(
            "turso-mesh-restore-verify: WARNING: password env %s is not set; "
            "restic will fail unless the repository needs no password"
            % password_env,
            file=sys.stderr,
        )
    env = dict(os.environ)
    env["RESTIC_PASSWORD_ENV"] = password_env
    results = [
        verify_one(db, repo, scratch, restic_bin, decompress_cmd, env, snapshot)
        for db in databases
    ]
    return {
        "repository_name": DEFAULT_REPOSITORY_NAME,
        "databases": results,
        "status": "pass" if all(r["status"] == "pass" for r in results) else "fail",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Class-A first restore verification (#13455 slice 4, #14251)."
    )
    sub = parser.add_subparsers(dest="command", required=True)
    verify_p = sub.add_parser(
        "verify", help="restore and verify the class-A snapshots"
    )
    verify_p.add_argument("--db", action="append", default=[],
                          help="database to verify (repeatable; default: class-A three)")
    verify_p.add_argument("--repo", required=True,
                          help="restic repository URL (live: R2 supabase-backups/%s)"
                          % DEFAULT_REPOSITORY_NAME)
    verify_p.add_argument("--scratch", required=True,
                          help="isolated scratch directory to restore into")
    verify_p.add_argument("--snapshot", default=None,
                          help="snapshot id to restore (default: newest per database)")
    verify_p.add_argument("--restic-bin", default="restic")
    verify_p.add_argument("--decompress-cmd", default="zstd -d -q -c")
    verify_p.add_argument("--password-env", default="RESTIC_PASSWORD",
                          help="NAME of the env var holding the restic password")
    args = parser.parse_args(argv)
    report = verify(
        args.db or list(CLASS_A_DATABASES),
        args.repo,
        args.scratch,
        restic_bin=args.restic_bin,
        decompress_cmd=args.decompress_cmd,
        password_env=args.password_env,
        snapshot=args.snapshot,
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
