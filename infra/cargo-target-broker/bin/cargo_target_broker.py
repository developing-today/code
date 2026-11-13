#!/usr/bin/env python3
"""Cargo target-dir broker: leased, bounded pool of warm cargo target dirs.

Why this exists (code#6797, code#3448): per-worktree cargo target dirs
grow without bound and fill the laptop disk, but sharing ONE target dir
across worktrees was tried and rejected -- concurrent/alternating use
thrashes cargo fingerprints and workspace crates recompile on every
checkout switch. sccache (user-wide, code#3448) caches compiler outputs
but every worktree still materializes a full target dir from them.

The broker keeps the sharing and removes the thrash with lease
discipline, the same pattern as the fleet's other brokers:

- A pool of target dirs keyed by (repo, profile label, rustc version,
  host triple). Dependency artifacts are checkout-independent, so a
  pooled dir stays warm across DIFFERENT worktrees of the same repo.
- `checkout` grants an EXCLUSIVE lease on one pooled dir. One writer
  per dir at a time; a concurrent request for the same key gets a
  second pooled dir (up to the per-key cap) or waits -- never shares
  a live dir.
- `checkin` releases the lease; the dir stays warm for the next lane.
- The pool is bounded: a hard total-size cap with LRU eviction of
  idle (unleased) dirs, a per-key dir cap, and leases that expire
  after a TTL so a dead lane's dir is reclaimable. A leased dir is
  never evicted.
- Pre-existing dirs in the pool root matching a key are adopted on
  first sight, so dirs created by older tooling are not orphaned.

State is a SQLite file under the state dir; every mutation runs under
an flock so concurrent broker processes cannot double-lease a dir.
No daemon: checkout/checkin/exec are one-shot CLI calls, like the
cargo-target-janitor and the fleet queue CLI.

Typical lane use:

    target-broker exec --repo code --profile test -- cargo test -p id

or explicitly:

    DIR=$(target-broker checkout --repo code --profile test)
    CARGO_TARGET_DIR=$DIR cargo test -p id
    target-broker checkin --path "$DIR"
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

GIB = 1024**3
DEFAULT_POOL_ROOT = Path.home() / "build-cache" / "broker-pool"
DEFAULT_STATE_DIR = Path.home() / ".local" / "state" / "cargo-target-broker"
DEFAULT_CAP_BYTES = 300 * GIB
DEFAULT_PER_KEY = 3
DEFAULT_TTL_SECS = 6 * 3600
POLL_SECS = 2.0


def eprint(*args: object) -> None:
    print(*args, file=sys.stderr)


def sanitize(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", "_", value)


def rustc_fingerprint() -> tuple[str, str]:
    """(rustc version string, host triple) from `rustc -vV`."""
    out = subprocess.run(
        ["rustc", "-vV"], capture_output=True, text=True, check=True
    ).stdout
    version = ""
    host = ""
    for line in out.splitlines():
        if line.startswith("release:"):
            version = line.split(":", 1)[1].strip()
        elif line.startswith("host:"):
            host = line.split(":", 1)[1].strip()
    return version, host


def make_key(repo: str, profile: str) -> str:
    repo_name = sanitize(Path(repo).name if "/" in repo else repo)
    version, host = rustc_fingerprint()
    digest = hashlib.sha256(f"{version}|{host}".encode()).hexdigest()[:8]
    return f"{repo_name}-{sanitize(profile)}-r{digest}"


@dataclass
class DirRow:
    id: int
    key: str
    path: Path
    created_at: float
    last_used_at: float
    bytes_cached: int


class Broker:
    def __init__(
        self,
        pool_root: Path,
        state_dir: Path,
        cap_bytes: int,
        per_key: int,
        ttl_secs: int,
    ) -> None:
        self.pool_root = pool_root
        self.state_dir = state_dir
        self.cap_bytes = cap_bytes
        self.per_key = per_key
        self.ttl_secs = ttl_secs
        self.pool_root.mkdir(parents=True, exist_ok=True)
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.db_path = self.state_dir / "broker.db"
        self._lock_fd = None

    # -- locking / db -------------------------------------------------

    def __enter__(self) -> "Broker":
        lock_path = self.state_dir / "broker.lock"
        self._lock_fd = open(lock_path, "w")
        fcntl.flock(self._lock_fd, fcntl.LOCK_EX)
        self.db = sqlite3.connect(self.db_path)
        self.db.row_factory = sqlite3.Row
        self.db.executescript(
            """
            CREATE TABLE IF NOT EXISTS dirs (
                id INTEGER PRIMARY KEY,
                key TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                created_at REAL NOT NULL,
                last_used_at REAL NOT NULL,
                bytes_cached INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS leases (
                lease_id TEXT PRIMARY KEY,
                dir_id INTEGER NOT NULL REFERENCES dirs(id),
                holder TEXT NOT NULL,
                acquired_at REAL NOT NULL,
                expires_at REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS counters (
                name TEXT PRIMARY KEY,
                value INTEGER NOT NULL DEFAULT 0
            );
            """
        )
        return self

    def __exit__(self, *exc: object) -> None:
        self.db.commit()
        self.db.close()
        if self._lock_fd is not None:
            fcntl.flock(self._lock_fd, fcntl.LOCK_UN)
            self._lock_fd.close()

    # -- helpers --------------------------------------------------------

    def bump(self, name: str, by: int = 1) -> None:
        self.db.execute(
            "INSERT INTO counters(name, value) VALUES(?, ?) "
            "ON CONFLICT(name) DO UPDATE SET value = value + excluded.value",
            (name, by),
        )

    def counter(self, name: str) -> int:
        row = self.db.execute(
            "SELECT value FROM counters WHERE name = ?", (name,)
        ).fetchone()
        return int(row["value"]) if row else 0

    @staticmethod
    def dir_size(path: Path) -> int:
        total = 0
        for root, _dirs, files in os.walk(path):
            for name in files:
                try:
                    total += os.lstat(os.path.join(root, name)).st_size
                except OSError:
                    pass
        return total

    @staticmethod
    def looks_warm(path: Path) -> bool:
        return (path / "debug" / ".fingerprint").exists() or (
            path / "release" / ".fingerprint"
        ).exists()

    def rows_for_key(self, key: str) -> list[DirRow]:
        rows = self.db.execute(
            "SELECT * FROM dirs WHERE key = ? ORDER BY last_used_at",
            (key,),
        ).fetchall()
        return [self._row(r) for r in rows]

    def all_rows(self) -> list[DirRow]:
        rows = self.db.execute(
            "SELECT * FROM dirs ORDER BY key, last_used_at"
        ).fetchall()
        return [self._row(r) for r in rows]

    @staticmethod
    def _row(r: sqlite3.Row) -> DirRow:
        return DirRow(
            id=r["id"],
            key=r["key"],
            path=Path(r["path"]),
            created_at=r["created_at"],
            last_used_at=r["last_used_at"],
            bytes_cached=r["bytes_cached"],
        )

    def leased_dir_ids(self) -> set[int]:
        rows = self.db.execute("SELECT dir_id FROM leases").fetchall()
        return {r["dir_id"] for r in rows}

    def reap_expired(self, now: float) -> int:
        cur = self.db.execute("DELETE FROM leases WHERE expires_at <= ?", (now,))
        if cur.rowcount:
            self.bump("leases_expired", cur.rowcount)
        return cur.rowcount

    def adopt_strays(self, key: str, now: float) -> None:
        """Register pool-root dirs matching `key` that the DB doesn't know."""
        known = {str(r.path) for r in self.rows_for_key(key)}
        prefix = key
        for child in sorted(self.pool_root.iterdir()):
            if not child.is_dir():
                continue
            name = child.name
            if name != prefix and not re.fullmatch(re.escape(prefix) + r"\.\d+", name):
                continue
            if str(child) in known:
                continue
            size = self.dir_size(child)
            self.db.execute(
                "INSERT OR IGNORE INTO dirs(key, path, created_at, last_used_at,"
                " bytes_cached) VALUES(?, ?, ?, ?, ?)",
                (key, str(child), now, now, size),
            )
            self.bump("dirs_adopted")

    def total_bytes(self) -> int:
        row = self.db.execute(
            "SELECT COALESCE(SUM(bytes_cached), 0) AS s FROM dirs"
        ).fetchone()
        return int(row["s"])

    def evict_until_under_cap(self, now: float) -> list[str]:
        """Evict LRU idle dirs while over cap. Never evicts a leased dir."""
        evicted: list[str] = []
        while self.total_bytes() > self.cap_bytes:
            leased = self.leased_dir_ids()
            row = self.db.execute(
                "SELECT * FROM dirs ORDER BY last_used_at ASC"
            ).fetchall()
            victim = None
            for r in row:
                if r["id"] not in leased:
                    victim = self._row(r)
                    break
            if victim is None:
                break
            shutil.rmtree(victim.path, ignore_errors=True)
            self.db.execute("DELETE FROM dirs WHERE id = ?", (victim.id,))
            self.bump("evictions")
            evicted.append(str(victim.path))
            eprint(f"target-broker: evicted {victim.path} (LRU, over cap)")
        return evicted

    # -- verbs ----------------------------------------------------------

    def checkout(
        self, key: str, holder: str, wait_secs: int
    ) -> tuple[DirRow, str, bool] | None:
        """Grant an exclusive lease. Returns (dir, lease_id, warm) or None."""
        deadline = time.time() + wait_secs
        while True:
            now = time.time()
            self.reap_expired(now)
            self.adopt_strays(key, now)
            leased = self.leased_dir_ids()
            rows = self.rows_for_key(key)
            free = [r for r in rows if r.id not in leased]
            if free:
                # Prefer the warmest free dir (most recently used).
                chosen = sorted(free, key=lambda r: r.last_used_at)[-1]
                warm = self.looks_warm(chosen.path)
                lease_id = uuid.uuid4().hex
                self.db.execute(
                    "INSERT INTO leases(lease_id, dir_id, holder, acquired_at,"
                    " expires_at) VALUES(?, ?, ?, ?, ?)",
                    (lease_id, chosen.id, holder, now, now + self.ttl_secs),
                )
                self.db.execute(
                    "UPDATE dirs SET last_used_at = ? WHERE id = ?",
                    (now, chosen.id),
                )
                self.bump("checkouts_warm" if warm else "checkouts_cold")
                self.db.commit()
                return chosen, lease_id, warm
            if len(rows) < self.per_key:
                # Room for another dir for this key. Creating one adds
                # ~0 bytes now; enforce the cap against existing bytes
                # first so a new dir never pushes us further over.
                self.evict_until_under_cap(now)
                if self.total_bytes() <= self.cap_bytes or not rows:
                    idx = len(rows) + 1
                    name = key if idx == 1 else f"{key}.{idx}"
                    path = self.pool_root / name
                    # A same-named stray may exist with different key
                    # bookkeeping; only create when truly absent.
                    if path.exists() and str(path) not in {str(r.path) for r in rows}:
                        self.adopt_strays(key, now)
                        continue
                    path.mkdir(parents=True, exist_ok=True)
                    cur = self.db.execute(
                        "INSERT INTO dirs(key, path, created_at, last_used_at,"
                        " bytes_cached) VALUES(?, ?, ?, ?, 0)",
                        (key, str(path), now, now),
                    )
                    dir_id = cur.lastrowid
                    lease_id = uuid.uuid4().hex
                    self.db.execute(
                        "INSERT INTO leases(lease_id, dir_id, holder,"
                        " acquired_at, expires_at) VALUES(?, ?, ?, ?, ?)",
                        (lease_id, dir_id, holder, now, now + self.ttl_secs),
                    )
                    self.bump("dirs_created")
                    self.bump("checkouts_cold")
                    self.db.commit()
                    return (
                        DirRow(dir_id, key, path, now, now, 0),
                        lease_id,
                        False,
                    )
            if time.time() >= deadline:
                return None
            # Release the flock while waiting so checkins can proceed.
            self.db.commit()
            fcntl.flock(self._lock_fd, fcntl.LOCK_UN)
            time.sleep(POLL_SECS)
            fcntl.flock(self._lock_fd, fcntl.LOCK_EX)

    def checkin(self, lease_id: str | None, path: str | None) -> DirRow | None:
        now = time.time()
        if lease_id:
            row = self.db.execute(
                "SELECT dir_id FROM leases WHERE lease_id = ?", (lease_id,)
            ).fetchone()
            if row is None:
                return None
            dir_id = row["dir_id"]
            self.db.execute("DELETE FROM leases WHERE lease_id = ?", (lease_id,))
        elif path:
            r = self.db.execute(
                "SELECT id FROM dirs WHERE path = ?", (path,)
            ).fetchone()
            if r is None:
                return None
            dir_id = r["id"]
            self.db.execute("DELETE FROM leases WHERE dir_id = ?", (dir_id,))
        else:
            return None
        d = self.db.execute("SELECT * FROM dirs WHERE id = ?", (dir_id,)).fetchone()
        if d is None:
            return None
        # Refresh the cached size now that the writer has stopped.
        size = self.dir_size(Path(d["path"]))
        self.db.execute(
            "UPDATE dirs SET bytes_cached = ?, last_used_at = ? WHERE id = ?",
            (size, now, dir_id),
        )
        self.bump("checkins")
        row = self._row(
            self.db.execute("SELECT * FROM dirs WHERE id = ?", (dir_id,)).fetchone()
        )
        # The eviction pass may shed this very dir if the pool is over
        # cap; the lease is released either way, so return the row as
        # it stood at checkin rather than re-reading a deleted row.
        self.evict_until_under_cap(now)
        self.db.commit()
        return row

    def status(self) -> dict:
        now = time.time()
        self.reap_expired(now)
        leased = {
            r["dir_id"]: r for r in self.db.execute("SELECT * FROM leases").fetchall()
        }
        by_key: dict[str, list[dict]] = {}
        total = 0
        for r in self.all_rows():
            total += r.bytes_cached
            entry = {
                "path": str(r.path),
                "gib": round(r.bytes_cached / GIB, 2),
                "leased": r.id in leased,
                "holder": leased[r.id]["holder"] if r.id in leased else None,
                "last_used_at": r.last_used_at,
            }
            by_key.setdefault(r.key, []).append(entry)
        return {
            "pool_root": str(self.pool_root),
            "cap_gib": round(self.cap_bytes / GIB, 1),
            "total_gib": round(total / GIB, 2),
            "dirs": by_key,
            "counters": {
                name: self.counter(name)
                for name in (
                    "checkouts_warm",
                    "checkouts_cold",
                    "checkins",
                    "dirs_created",
                    "dirs_adopted",
                    "evictions",
                    "leases_expired",
                )
            },
        }


def default_holder() -> str:
    return os.environ.get(
        "CARGO_TARGET_BROKER_HOLDER",
        f"{os.environ.get('USER', 'unknown')}@{os.uname().nodename}:{os.getpid()}",
    )


def build_broker(args: argparse.Namespace) -> Broker:
    return Broker(
        pool_root=Path(args.pool_root).expanduser(),
        state_dir=Path(args.state_dir).expanduser(),
        cap_bytes=int(args.cap_gib * GIB),
        per_key=args.per_key,
        ttl_secs=args.ttl_secs,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="target-broker", description=__doc__)
    parser.add_argument("--pool-root", default=str(DEFAULT_POOL_ROOT))
    parser.add_argument("--state-dir", default=str(DEFAULT_STATE_DIR))
    parser.add_argument(
        "--cap-gib",
        type=float,
        default=DEFAULT_CAP_BYTES / GIB,
        help="total pool size cap in GiB (default 300)",
    )
    parser.add_argument(
        "--per-key",
        type=int,
        default=DEFAULT_PER_KEY,
        help="max pooled dirs per key (default 3)",
    )
    parser.add_argument(
        "--ttl-secs",
        type=int,
        default=DEFAULT_TTL_SECS,
        help="lease TTL in seconds (default 21600 = 6h)",
    )
    sub = parser.add_subparsers(dest="verb", required=True)

    p_co = sub.add_parser("checkout")
    p_co.add_argument("--repo", required=True, help="repo name or path")
    p_co.add_argument(
        "--profile",
        default="dev",
        help="build profile label: dev | test | release | <custom>",
    )
    p_co.add_argument("--holder", default=None)
    p_co.add_argument(
        "--wait",
        type=int,
        default=0,
        help="seconds to wait for a free dir before failing",
    )
    p_co.add_argument("--json", action="store_true")

    p_ci = sub.add_parser("checkin")
    p_ci.add_argument("--lease", default=None)
    p_ci.add_argument("--path", default=None)

    p_ex = sub.add_parser("exec")
    p_ex.add_argument("--repo", required=True)
    p_ex.add_argument("--profile", default="dev")
    p_ex.add_argument("--holder", default=None)
    p_ex.add_argument("--wait", type=int, default=0)
    p_ex.add_argument("cmd", nargs=argparse.REMAINDER)

    p_st = sub.add_parser("status")
    p_st.add_argument("--json", action="store_true")

    args = parser.parse_args(argv)
    broker = build_broker(args)

    if args.verb == "status":
        with broker:
            st = broker.status()
        if args.json:
            print(json.dumps(st, indent=2))
        else:
            print(
                f"pool: {st['pool_root']}  total {st['total_gib']} GiB"
                f" / cap {st['cap_gib']} GiB"
            )
            for key, dirs in st["dirs"].items():
                for d in dirs:
                    state = f"leased by {d['holder']}" if d["leased"] else "idle"
                    print(f"  {key}  {d['path']}  {d['gib']} GiB  {state}")
            c = st["counters"]
            warm, cold = c["checkouts_warm"], c["checkouts_cold"]
            rate = (100.0 * warm / (warm + cold)) if (warm + cold) else 0.0
            print(
                f"checkouts: {warm} warm / {cold} cold ({rate:.0f}% warm),"
                f" evictions: {c['evictions']},"
                f" expired leases: {c['leases_expired']}"
            )
        return 0

    if args.verb == "checkin":
        if not args.lease and not args.path:
            eprint("target-broker: checkin needs --lease or --path")
            return 2
        with broker:
            row = broker.checkin(args.lease, args.path)
        if row is None:
            eprint("target-broker: no matching lease/dir")
            return 1
        print(str(row.path))
        return 0

    if args.verb in ("checkout", "exec"):
        key = make_key(args.repo, args.profile)
        holder = args.holder or default_holder()
        with broker:
            got = broker.checkout(key, holder, args.wait)
        if got is None:
            eprint(
                f"target-broker: no dir available for key {key} "
                f"(per-key cap {args.per_key} reached, all leased)"
            )
            return 3
        row, lease_id, warm = got
        if args.verb == "checkout":
            if args.json:
                print(
                    json.dumps(
                        {
                            "path": str(row.path),
                            "lease": lease_id,
                            "key": key,
                            "warm": warm,
                        }
                    )
                )
            else:
                print(str(row.path))
            eprint(
                f"target-broker: lease {lease_id} on {row.path}"
                f" ({'warm' if warm else 'cold'})"
            )
            return 0
        # exec
        cmd = args.cmd
        if cmd and cmd[0] == "--":
            cmd = cmd[1:]
        if not cmd:
            eprint("target-broker: exec needs a command after --")
            return 2
        env = dict(os.environ)
        env["CARGO_TARGET_DIR"] = str(row.path)
        env["CARGO_TARGET_BROKER_LEASE"] = lease_id
        eprint(
            f"target-broker: lease {lease_id} on {row.path}"
            f" ({'warm' if warm else 'cold'}); running {cmd}"
        )
        try:
            proc = subprocess.run(cmd, env=env)
            rc = proc.returncode
        finally:
            with broker:
                broker.checkin(lease_id, None)
        return rc

    return 2


if __name__ == "__main__":
    raise SystemExit(main())
