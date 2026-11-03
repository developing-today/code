"""Pure-function tests for fleet_paperwork (no network, no secrets).

Run: python3 -m unittest discover -s tests -v
"""

import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import fleet_paperwork as fp


class TestB64Url(unittest.TestCase):
    def test_no_padding_and_urlsafe(self):
        self.assertEqual(fp.b64url(b"\xfb\xff\xfe"), "-__-")
        self.assertNotIn("=", fp.b64url(b"a"))
        self.assertNotIn("+", fp.b64url(b"\xfb" * 10))
        self.assertNotIn("/", fp.b64url(b"\xff" * 10))

    def test_roundtrip_shape(self):
        import base64
        raw = b'{"alg":"RS256","typ":"JWT"}'
        enc = fp.b64url(raw)
        pad = "=" * (-len(enc) % 4)
        self.assertEqual(base64.urlsafe_b64decode(enc + pad), raw)


class TestLedgerPacing(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.NamedTemporaryFile(
            "w", suffix=".tsv", delete=False)
        self.old_env = os.environ.get("FLEET_PAPERWORK_LEDGER")
        os.environ["FLEET_PAPERWORK_LEDGER"] = self.tmp.name

    def tearDown(self):
        self.tmp.close()
        try:
            os.unlink(self.tmp.name)
        except FileNotFoundError:
            pass
        if self.old_env is None:
            os.environ.pop("FLEET_PAPERWORK_LEDGER", None)
        else:
            os.environ["FLEET_PAPERWORK_LEDGER"] = self.old_env

    def test_missing_ledger_has_no_last_write(self):
        os.unlink(self.tmp.name)
        self.assertIsNone(fp.last_write_epoch(Path(self.tmp.name)))

    def test_append_then_read_back(self):
        fp.ledger_append("create", "developing-today/code", "42", "a title")
        last = fp.last_write_epoch(Path(self.tmp.name))
        self.assertIsNotNone(last)
        self.assertAlmostEqual(last, time.time(), delta=5)

    def test_pace_waits_remainder(self):
        fp.ledger_append("create", "developing-today/code", "42", "t")
        start = time.time()
        fp.pace(1.0)
        elapsed = time.time() - start
        # Waits out the remainder of the 1 s window since the ledger write:
        # substantial, but less than a full fresh second.
        self.assertGreaterEqual(elapsed, 0.4)
        self.assertLess(elapsed, 1.0)

    def test_pace_zero_is_immediate(self):
        fp.ledger_append("create", "developing-today/code", "42", "t")
        start = time.time()
        fp.pace(0)
        self.assertLess(time.time() - start, 0.5)


class TestExactTitleFilter(unittest.TestCase):
    """The dedupe contract: only a byte-exact title counts as already filed."""

    def test_exact_match_only(self):
        title = "Preserved branch: queue/x — decision needed"
        items = [
            {"title": title + " (copy)", "number": 1},
            {"title": title.lower(), "number": 2},
            {"title": title, "number": 3},
        ]
        hits = [i for i in items if i.get("title") == title]
        self.assertEqual([h["number"] for h in hits], [3])


if __name__ == "__main__":
    unittest.main()
