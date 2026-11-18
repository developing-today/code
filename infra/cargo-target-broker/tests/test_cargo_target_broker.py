"""Eviction accounting for the cargo target broker (#11417).

Oracle: the issue's acceptance shape. When removing an LRU victim fails,
its registry row and byte accounting must survive; when it succeeds, the
row goes and the eviction is counted.
"""

from __future__ import annotations

import importlib.util
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "cargo_target_broker", HERE.parent / "bin" / "cargo_target_broker.py"
)
broker_mod = importlib.util.module_from_spec(SPEC)
sys.modules["cargo_target_broker"] = broker_mod
SPEC.loader.exec_module(broker_mod)


class EvictionTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="ctb-test-"))
        self.pool = self.tmp / "pool"
        self.broker = broker_mod.Broker(
            pool_root=self.pool,
            state_dir=self.tmp / "state",
            cap_bytes=100,
            per_key=2,
            ttl_secs=60,
        )
        self.broker.__enter__()

    def tearDown(self) -> None:
        self.broker.__exit__(None, None, None)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def add_dir(
        self, name: str, size: int, last_used: float, key: str | None = None
    ) -> Path:
        path = self.pool / name
        path.mkdir(parents=True)
        (path / "blob").write_bytes(b"x" * size)
        self.broker.db.execute(
            "INSERT INTO dirs(key, path, created_at, last_used_at, bytes_cached)"
            " VALUES(?, ?, ?, ?, ?)",
            (key or name, str(path), last_used, last_used, size),
        )
        return path

    def test_successful_eviction_drops_the_row(self) -> None:
        old = self.add_dir("old", 80, 1.0)
        self.add_dir("new", 80, 2.0)
        evicted = self.broker.evict_until_under_cap(now=3.0)
        self.assertEqual(evicted, [str(old)])
        self.assertFalse(old.exists())
        self.assertEqual(self.broker.total_bytes(), 80)
        self.assertEqual(self.broker.counter("evictions"), 1)
        self.assertEqual(self.broker.counter("eviction_failures"), 0)

    def test_failed_removal_keeps_the_row_and_its_bytes(self) -> None:
        old = self.add_dir("old", 80, 1.0)
        new = self.add_dir("new", 80, 2.0)

        def refuse(path, *args, **kwargs):
            raise PermissionError(13, "Permission denied", str(path))

        with mock.patch.object(broker_mod.shutil, "rmtree", side_effect=refuse):
            evicted = self.broker.evict_until_under_cap(now=3.0)

        # Nothing was evicted, both dirs are still on disk and on the books,
        # and the loop terminated instead of retrying the stuck dir.
        self.assertEqual(evicted, [])
        self.assertTrue(old.exists() and new.exists())
        paths = {str(r.path): r.bytes_cached for r in self.broker.all_rows()}
        self.assertEqual(paths, {str(old): 80, str(new): 80})
        self.assertEqual(self.broker.total_bytes(), 160)
        self.assertEqual(self.broker.counter("evictions"), 0)
        self.assertEqual(self.broker.counter("eviction_failures"), 2)

    def test_partial_removal_rebooks_what_is_left(self) -> None:
        old = self.add_dir("old", 80, 1.0)
        (old / "small").write_bytes(b"y" * 5)
        self.add_dir("new", 30, 2.0)

        def partial(path, *args, **kwargs):
            (Path(path) / "blob").unlink()
            raise PermissionError(13, "Permission denied", str(Path(path) / "small"))

        with mock.patch.object(broker_mod.shutil, "rmtree", side_effect=partial):
            self.broker.evict_until_under_cap(now=3.0)

        rows = {r.path.name: r.bytes_cached for r in self.broker.all_rows()}
        self.assertEqual(rows, {"old": 5, "new": 30})
        self.assertEqual(self.broker.counter("eviction_failures"), 1)

    def test_a_failed_victim_is_not_readopted_as_a_stray(self) -> None:
        old = self.add_dir("k", 80, 1.0, key="k")
        self.add_dir("k.2", 80, 2.0, key="k")
        with mock.patch.object(
            broker_mod.shutil, "rmtree", side_effect=PermissionError("nope")
        ):
            self.broker.evict_until_under_cap(now=3.0)
        self.broker.adopt_strays("k", now=4.0)
        self.assertEqual(self.broker.counter("dirs_adopted"), 0)
        self.assertEqual(len(self.broker.all_rows()), 2)
        self.assertTrue(old.exists())


if __name__ == "__main__":
    unittest.main()
