#!/usr/bin/env python3
"""Class-A restic backup runner (#13455 slice 2, issue #14247).

The restic execution leg of the backup-mesh class-A Turso leg (design
``doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md`` §5), built on
the slice-1 planner/manifest module ``turso-mesh-class-a.py``
(PR #13476). Code only: no live export, R2 write, or credential access
happens in this slice — the live backend is selected by configuration
and every external stage is an injectable command seam, so the whole
runner is testable against a local restic-repository fixture and a
fixture dump.

Per class-A database (``fleet-queue``, ``fleet-platform``, ``atlas``):

1. Export: run the export command seam for that database. The seam is
   a command template with ``{db}``, ``{dump}`` and ``{sqlite}``
   placeholders; it must write the filtered SQL dump (the
   export-turso.sh proven path, with its libsql /dump repair filter)
   to ``{dump}`` and the rebuilt SQLite file to ``{sqlite}``. In live
   configuration this wraps ``scripts/export-turso.sh``'s per-database
   output; in tests it is a fixture writer.
2. Manifest: build the export manifest with slice 1's
   ``build_manifest`` and write it next to the compressed dump.
3. Compress + snapshot: stream the dump through the compressor seam
   (default ``zstd``, the design's stream-zstd) into the staging
   directory — the dump is never loaded whole — then take ONE restic
   snapshot of the staging directory (compressed dump + manifest
   together, so restore verification always finds the manifest
   alongside the snapshot's dump), tagged ``class-a``,
   ``mesh=turso-mesh`` and ``db=<name>``, against the dedicated
   repository ``restic-turso-mesh`` (live: the R2 ``supabase-backups``
   bucket; the repository URL arrives by configuration only).

After all databases, retention runs once for the class-A tag set:
``restic forget --keep-daily 7 --keep-weekly 8 --keep-monthly 12
--prune`` (design §5).

Failure isolation: one database's failure is reported on its own
line, never blocks the others, and makes the run exit non-zero at the
end. The restic password is referenced by environment-variable NAME
only (``--password-env``, default ``RESTIC_PASSWORD``); the runner
never reads, stores, or prints a credential value, and the repository
URL is configuration, never hard-coded beyond the documented default
repository name.

Usage:

    turso-mesh-restic-runner.py run [--db NAME ...] \
        --repo URL --export-cmd 'TEMPLATE' [--work DIR] \
        [--restic-bin PATH] [--compress-cmd 'zstd -q -c'] \
        [--no-retention] [--exported-at ISO-8601]
"""

import argparse
import importlib.util
import json
import os
import shlex
import subprocess
import sys
from datetime import datetime, timezone
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
RETENTION = (("--keep-daily", "7"), ("--keep-weekly", "8"), ("--keep-monthly", "12"))
SNAPSHOT_TAGS = ("class-a", "mesh=turso-mesh")


def _run(cmd, **kw):
    return subprocess.run(cmd, **kw)


def export_one(db, stage_dir, export_cmd):
    """Run the export seam for one database; returns (dump, sqlite)."""
    dump = stage_dir / ("%s.sql" % db)
    sqlite = stage_dir / ("%s.db" % db)
    template = export_cmd.replace("{db}", db).replace(
        "{dump}", str(dump)
    ).replace("{sqlite}", str(sqlite))
    proc = _run(shlex.split(template))
    if proc.returncode != 0:
        raise RuntimeError("export command failed (exit %d)" % proc.returncode)
    if not dump.is_file() or not sqlite.is_file():
        raise RuntimeError("export command did not produce dump and sqlite files")
    return dump, sqlite


def compress_dump(dump, out_path, compress_cmd):
    """Stream ``dump`` through the compressor seam into ``out_path``."""
    with open(dump, "rb") as src, open(out_path, "wb") as dst:
        proc = subprocess.run(
            shlex.split(compress_cmd), stdin=src, stdout=dst
        )
    if proc.returncode != 0:
        raise RuntimeError("compressor failed (exit %d)" % proc.returncode)


def restic_backup(db, stage_dir, repo, restic_bin, password_env, env_extra):
    cmd = [
        restic_bin,
        "--repo",
        repo,
        "backup",
        str(stage_dir),
    ]
    for tag in SNAPSHOT_TAGS + ("db=%s" % db,):
        cmd += ["--tag", tag]
    proc = _run(cmd, env=env_extra)
    if proc.returncode != 0:
        raise RuntimeError("restic backup failed (exit %d)" % proc.returncode)


def restic_retention(repo, restic_bin, env_extra):
    cmd = [restic_bin, "--repo", repo, "forget", "--tag", SNAPSHOT_TAGS[0]]
    for flag, value in RETENTION:
        cmd += [flag, value]
    cmd.append("--prune")
    proc = _run(cmd, env=env_extra)
    if proc.returncode != 0:
        raise RuntimeError("restic forget failed (exit %d)" % proc.returncode)


def run(databases, repo, export_cmd, work_dir, restic_bin="restic",
        compress_cmd="zstd -q -c", password_env="RESTIC_PASSWORD",
        exported_at=None, retention=True):
    """Back up each database independently; returns a per-db report.

    The report maps each database name to "ok" or the failure text.
    Retention runs only when at least one database succeeded.
    """
    if password_env not in os.environ:
        # Named, never valued: say which variable is missing, nothing more.
        print(
            "turso-mesh-restic: WARNING: password env %s is not set; "
            "restic will fail unless the repository needs no password"
            % password_env,
            file=sys.stderr,
        )
    env = dict(os.environ)
    env["RESTIC_PASSWORD_ENV"] = password_env
    exported_at = exported_at or datetime.now(timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )
    work = Path(work_dir)
    work.mkdir(parents=True, exist_ok=True)
    report = {}
    for db in databases:
        stage = work / db
        stage.mkdir(parents=True, exist_ok=True)
        try:
            dump, sqlite = export_one(db, stage, export_cmd)
            with open(dump, "rb") as fh:
                dump_bytes = fh.read()
            manifest = CLASS_A.build_manifest(db, str(sqlite), dump_bytes, exported_at)
            (stage / "manifest.json").write_text(
                json.dumps(manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            compress_dump(dump, stage / ("%s.sql.zst" % db), compress_cmd)
            dump.unlink()
            sqlite.unlink()
            restic_backup(db, stage, repo, restic_bin, password_env, env)
            report[db] = "ok"
            print("turso-mesh-restic: %s: snapshot filed" % db)
        except Exception as exc:  # per-db isolation is the contract
            report[db] = "failed: %s" % exc
            print("turso-mesh-restic: %s: ERROR: %s" % (db, exc), file=sys.stderr)
    if retention and any(v == "ok" for v in report.values()):
        try:
            restic_retention(repo, restic_bin, env)
        except Exception as exc:
            report["__retention__"] = "failed: %s" % exc
            print("turso-mesh-restic: retention: ERROR: %s" % exc, file=sys.stderr)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Class-A restic backup runner (#13455 slice 2, #14247)."
    )
    sub = parser.add_subparsers(dest="command", required=True)
    run_p = sub.add_parser("run", help="back up the class-A databases")
    run_p.add_argument("--db", action="append", default=[],
                       help="database to back up (repeatable; default: class-A three)")
    run_p.add_argument("--repo", required=True,
                       help="restic repository URL (live: R2 supabase-backups/%s)"
                       % DEFAULT_REPOSITORY_NAME)
    run_p.add_argument("--export-cmd", required=True,
                       help="export seam template with {db} {dump} {sqlite}")
    run_p.add_argument("--work", default=None, help="staging work directory")
    run_p.add_argument("--restic-bin", default="restic")
    run_p.add_argument("--compress-cmd", default="zstd -q -c")
    run_p.add_argument("--password-env", default="RESTIC_PASSWORD",
                       help="NAME of the env var holding the restic password")
    run_p.add_argument("--exported-at", default=None)
    run_p.add_argument("--no-retention", action="store_true")
    args = parser.parse_args(argv)
    work = args.work or os.environ.get(
        "TURSO_MESH_RESTIC_WORK_DIR",
        str(Path(os.environ.get("TMPDIR", "/tmp")) / "turso-mesh-restic"),
    )
    report = run(
        args.db or list(CLASS_A_DATABASES),
        args.repo,
        args.export_cmd,
        work,
        restic_bin=args.restic_bin,
        compress_cmd=args.compress_cmd,
        password_env=args.password_env,
        exported_at=args.exported_at,
        retention=not args.no_retention,
    )
    failed = [k for k, v in report.items() if v != "ok"]
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
