"""Pure-function tests for fleet_paperwork (no network, no secrets).

Run: python3 -m unittest discover -s tests -v
"""

import os
import re
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
        self.tmp = tempfile.NamedTemporaryFile("w", suffix=".tsv", delete=False)
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
        last = fp.last_write_epoch(Path(self.tmp.name))
        start = time.time()
        fp.pace(1.0)
        elapsed = time.time() - start
        # Waits out the remainder of the 1 s window since the ledger write:
        # substantial, but no more than the stamp-derived remainder plus a
        # small wake-up tolerance (a fixed 1.0 bound flakes on overshoot).
        self.assertGreaterEqual(elapsed, 0.4)
        self.assertLess(elapsed, 1.0 - (start - last) + 0.1)

    def test_append_records_subsecond_precision(self):
        fp.ledger_append("create", "developing-today/code", "42", "t")
        stamp = Path(self.tmp.name).read_text().strip().split("	")[0]
        # A fractional-second part must be present: whole-second stamps
        # truncate up to ~1 s of age and defeat pace() spacing (#5109).
        self.assertIn(".", stamp)
        last = fp.last_write_epoch(Path(self.tmp.name))
        self.assertIsNotNone(last)
        self.assertAlmostEqual(last, time.time(), delta=0.5)

    def test_second_precision_ledger_line_still_parses(self):
        # Lines written by older versions (timespec="seconds") must keep
        # parsing via fromisoformat.
        Path(self.tmp.name).write_text(
            "2026-01-01T00:00:00+00:00\tcreate\tdeveloping-today/code\t1\tx\n"
        )
        last = fp.last_write_epoch(Path(self.tmp.name))
        self.assertIsNotNone(last)
        self.assertAlmostEqual(last, 1767225600.0, delta=1)

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


class _StubClient:
    """Minimal AppClient stand-in: find_by_exact_title only needs a token."""

    def token_for(self, owner):
        return "stub-token"


class TestFindByExactTitle(unittest.TestCase):
    """The real find_by_exact_title against a stubbed http (#6697).

    Oracle: Search-API payloads shaped as GitHub actually returns them —
    PR items carry a `pull_request` key, and phrase matches include
    non-exact titles because the search matches title+body text."""

    TITLE = "id: exact title shared with a PR"

    def _run(self, pages):
        """pages: {page_number: (items, total_count)}. Returns (hit, paths)."""
        calls = []

        def fake_http(method, path, token, body=None):
            calls.append(path)
            m = re.search(r"[?&]page=(\d+)", path)
            page = int(m.group(1)) if m else 1
            items, total = pages[page]
            return 200, {"items": items, "total_count": total}

        orig = fp.http
        fp.http = fake_http
        try:
            hit = fp.find_by_exact_title(_StubClient(), "developing-today/code", self.TITLE)
        finally:
            fp.http = orig
        return hit, calls

    def test_pr_with_exact_title_is_not_a_hit(self):
        pr = {"title": self.TITLE, "number": 4242, "pull_request": {"url": "x"}}
        hit, _ = self._run({1: ([pr], 1)})
        self.assertIsNone(hit)

    def test_issue_found_behind_a_same_titled_pr(self):
        pr = {"title": self.TITLE, "number": 4242, "pull_request": {"url": "x"}}
        issue = {"title": self.TITLE, "number": 4243}
        hit, _ = self._run({1: ([pr, issue], 2)})
        self.assertIsNotNone(hit)
        self.assertEqual(hit["number"], 4243)

    def test_query_asks_for_issues_only(self):
        issue = {"title": self.TITLE, "number": 7}
        _, calls = self._run({1: ([issue], 1)})
        self.assertIn("type%3Aissue", calls[0])

    def test_exact_issue_past_first_page_of_matches_is_found(self):
        filler = [{"title": f"{self.TITLE} (mention {i})", "number": 100 + i} for i in range(100)]
        issue = {"title": self.TITLE, "number": 4243}
        hit, calls = self._run({1: (filler, 101), 2: ([issue], 101)})
        self.assertIsNotNone(hit)
        self.assertEqual(hit["number"], 4243)
        self.assertEqual(len(calls), 2)

    def test_no_hit_walks_all_pages_and_returns_none(self):
        filler = [{"title": f"{self.TITLE} (mention {i})", "number": 100 + i} for i in range(100)]
        hit, calls = self._run({1: (filler, 101), 2: ([{"title": "other", "number": 1}], 101)})
        self.assertIsNone(hit)
        self.assertEqual(len(calls), 2)

    def test_exact_match_behavior_for_issues_preserved(self):
        near = {"title": self.TITLE.lower(), "number": 2}
        exact = {"title": self.TITLE, "number": 3}
        hit, _ = self._run({1: ([near, exact], 2)})
        self.assertIsNotNone(hit)
        self.assertEqual(hit["number"], 3)


class TestGenericCommandArgs(unittest.TestCase):
    """The generic commands must not be able to act by accident: `api`
    requires an explicit --method for every call (no default to hide a
    write behind), and both new commands require --repo so a token is
    never minted without a scope-checked target. Argparse failures raise
    SystemExit before any command runs, so these need no network."""

    def test_api_requires_method(self):
        with self.assertRaises(SystemExit):
            fp.main(
                [
                    "api",
                    "--repo",
                    "developing-today/code",
                    "--path",
                    "/repos/developing-today/code",
                ]
            )

    def test_api_requires_path(self):
        with self.assertRaises(SystemExit):
            fp.main(["api", "--repo", "developing-today/code", "--method", "GET"])

    def test_api_requires_repo(self):
        with self.assertRaises(SystemExit):
            fp.main(
                ["api", "--method", "GET", "--path", "/repos/developing-today/code"]
            )

    def test_token_requires_repo(self):
        with self.assertRaises(SystemExit):
            fp.main(["token"])


if __name__ == "__main__":
    unittest.main()
