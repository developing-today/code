"""Disposable SQLite claim-contention experiment for issue #13602.

Oracle: one conditional UPDATE grants each eligible task to at most one
worker; with independent eligible rows, every successful grant has a unique
row id. This measures the normal-pick SELECT + conditional UPDATE only. It
omits the broker transport, lease/event writes, lane registration, guard,
quarantine, staleness probes, and managed-provider latency.

No network, credentials, or production database are used. Every run creates
and removes its own temporary SQLite database.
"""

import argparse
import hashlib
import json
import pathlib
import re
import sqlite3
import statistics
import tempfile
import threading
import time

SRC = pathlib.Path(__file__).resolve().parents[1] / "src"
RUST = (SRC / "turso_queue.rs").read_text() + "".join(
    p.read_text() for p in sorted((SRC / "turso_queue").glob("*.rs"))
)
# The benchmark intentionally isolates the queued branch of
# default_pick_predicate() and the current claim_update_sql() atomic guard.
PICK = "status = 'queued' AND attempts < max_attempts AND (not_before IS NULL OR not_before <= ?)"
UPDATE = """UPDATE tasks SET status='claimed', worker_id=?, claimed_at=?,
 lease_until=?, attempts=attempts+1, updated_at=?
 WHERE id=? AND status='queued' AND attempts < max_attempts
 AND (not_before IS NULL OR not_before <= ?)"""


def source_fingerprint():
    h = hashlib.sha256()
    for path in [SRC / "turso_queue.rs", *sorted((SRC / "turso_queue").glob("*.rs"))]:
        h.update(path.relative_to(SRC).as_posix().encode())
        h.update(path.read_bytes())
    return h.hexdigest()


def production_schema():
    marker = '"CREATE TABLE IF NOT EXISTS tasks ('
    start = RUST.find(marker)
    end = RUST.find("resolved_at TEXT", start)
    assert start >= 0 and end >= 0, "production tasks table DDL not found in SCHEMA"
    ddl = RUST[start + len(marker) : end]
    ddl = re.sub(r"\\\s*", "", ddl).replace('\\"', '"') + "resolved_at TEXT"
    index = re.search(
        r'"CREATE INDEX IF NOT EXISTS tasks_claim_scan \\\s*(.*?)",', RUST, re.S
    )
    assert index, "production tasks_claim_scan index not found in SCHEMA"
    index_sql = "CREATE INDEX IF NOT EXISTS tasks_claim_scan " + re.sub(
        r"\\\s*", "", index.group(1)
    )
    return "CREATE TABLE IF NOT EXISTS tasks (" + ddl + ")", index_sql


def create_db(path, fixture_size, eligible_count):
    db = sqlite3.connect(path)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=NORMAL")
    table_sql, index_sql = production_schema()
    db.execute(table_sql)
    db.execute(index_sql)
    # Ineligible rows model queue scan volume without bloating the fixture
    # with broker state unrelated to the single UPDATE under measurement.
    db.executemany(
        "INSERT INTO tasks(id,queue,status,priority,created_at,attempts,max_attempts,updated_at) VALUES(?, 'bench', 'completed', ?, ?, 0, 1000, ?)",
        ((i, i % 100, f"2026-01-{(i % 28) + 1:02d}T00:00:00Z", "fixture") for i in range(1, fixture_size + 1)),
    )
    start = fixture_size + 1
    db.executemany(
        "INSERT INTO tasks(id,queue,status,priority,created_at,attempts,max_attempts,updated_at) VALUES(?, 'bench', 'queued', ?, ?, 0, 1000, ?)",
        ((start + i, i, f"2026-02-{(i % 28) + 1:02d}T00:00:00Z", "fixture") for i in range(eligible_count)),
    )
    db.commit()
    db.close()
    return start


def claim(db_path, worker, start_barrier, granted):
    db = sqlite3.connect(db_path, timeout=20, isolation_level=None)
    db.execute("PRAGMA busy_timeout=20000")
    start_barrier.wait()
    started = time.perf_counter()
    while True:
        try:
            picked = db.execute(
                f"SELECT id FROM tasks WHERE {PICK} ORDER BY (priority IS NULL), priority, created_at LIMIT 1",
                ("9999-12-31T00:00:00Z",),
            ).fetchone()
            if picked is None:
                break
            now = "2026-11-26T23:00:00Z"
            changed = db.execute(
                UPDATE,
                (worker, now, "2026-11-27T03:00:00Z", now, picked[0], now),
            ).rowcount
            if changed == 1:
                granted.append((worker, picked[0]))
                break
            # A competing worker won the selected row; re-pick just as the
            # production claim loop does after a conditional-update loss.
        except sqlite3.OperationalError as exc:
            if "locked" not in str(exc).lower() and "busy" not in str(exc).lower():
                raise
            time.sleep(0.001)
    elapsed = time.perf_counter() - started
    db.close()
    return elapsed


def sample(fixture_size, concurrency, eligible_count):
    with tempfile.TemporaryDirectory(prefix="fleet-claim-contention-") as tmp:
        db_path = str(pathlib.Path(tmp) / "fixture.sqlite")
        create_db(db_path, fixture_size, eligible_count)
        barrier = threading.Barrier(concurrency)
        grants = []
        latencies = []
        errors = []

        def run(i):
            try:
                latencies.append(claim(db_path, f"worker-{i}", barrier, grants))
            except Exception as exc:  # surfaced below, never counted as a win
                errors.append(repr(exc))

        threads = [threading.Thread(target=run, args=(i,)) for i in range(concurrency)]
        start = time.perf_counter()
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        elapsed = time.perf_counter() - start
        ids = [task_id for _, task_id in grants]
        return {
            "fixture_rows": fixture_size,
            "eligible_rows": eligible_count,
            "callers": concurrency,
            "elapsed_seconds": elapsed,
            "throughput_grants_per_second": len(grants) / elapsed if elapsed else 0,
            "latency_ms_p50": statistics.median(latencies) * 1000 if latencies else 0,
            "latency_ms_max": max(latencies, default=0) * 1000,
            "grants": len(grants),
            "wrong_grants": max(0, len(grants) - eligible_count),
            "duplicate_task_grants": len(ids) - len(set(ids)),
            "errors": errors,
        }


def verify():
    # Both fixture sizes and claim shapes are covered by the smoke oracle.
    results = [
        sample(size, callers, eligible)
        for size in (100, 1000)
        for callers, eligible in ((1, 1), (8, 1), (8, 8))
    ]
    for result in results:
        assert not result["errors"], result
        assert result["wrong_grants"] == 0, result
        assert result["duplicate_task_grants"] == 0, result
        assert result["grants"] <= result["eligible_rows"], result
        if result["eligible_rows"] == result["callers"]:
            assert result["grants"] == result["eligible_rows"], result
    return results


def benchmark(repeats):
    return [
        {"repeat": repeat, **sample(size, callers, eligible)}
        for size in (1000, 10000)
        for callers in (1, 8, 16, 32)
        for eligible in (1, callers)
        for repeat in range(1, repeats + 1)
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="run the fast correctness oracle")
    parser.add_argument("--benchmark", action="store_true", help="run all contention measurements")
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    results = verify() if args.verify else benchmark(args.repeats) if args.benchmark else None
    if results is None:
        parser.error("choose --verify or --benchmark")
    print(json.dumps({
        "source_sha": __import__("subprocess").check_output(["git", "rev-parse", "HEAD"], cwd=SRC, text=True).strip(),
        "source_fingerprint_sha256": source_fingerprint(),
        "protocol": "one temporary SQLite file per sample using extracted production tasks DDL and tasks_claim_scan index; one connection per caller opened before a barrier; worker operations are SELECT then conditional UPDATE in autocommit; busy timeout 20s; lock/busy retries re-pick and are not grants; fixture rows are completed and eligible rows queued",
        "throughput_denominator": "wall-clock seconds from immediately before starting worker threads through all joins; includes thread scheduling and SELECT/conditional-UPDATE contention; excludes fixture creation and connection setup",
        "latency_denominator": "per-worker elapsed time from barrier release through its first successful conditional UPDATE or no-task result; includes SQLite operations and retry waits",
        "repeats": 1 if args.verify else args.repeats,
        "oracle": "unique task ids, no wrong grants, and all independent eligible rows granted",
        "omitted": ["broker transport", "lease and event writes", "lane registration", "guard/quarantine/staleness probes", "managed-provider latency"],
        "results": results,
    }, indent=2))


if __name__ == "__main__":
    main()
