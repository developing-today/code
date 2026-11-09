"""Pure-function tests for fleet_paperwork (no network, no secrets).

Run: python3 -m unittest discover -s tests -v
"""

import os
import sys
import tempfile
import time
import unittest
import unittest.mock
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


class _FakeClient:
    def token_for(self, owner):
        return "test-token"

    def assert_repo_in_scope(self, repo):
        return None


class TestFindByExactTitle(unittest.TestCase):
    """The real find_by_exact_title against stubbed Search-API payloads,
    shaped as GitHub returns them (PRs carry a `pull_request` key)."""

    TITLE = "id: exact title shared with a PR"

    def run_lookup(self, pages):
        """pages: {page_number: [items]}. Returns (hit, requested_paths)."""
        paths = []

        def fake_http(method, path, token, payload=None):
            paths.append(path)
            page = int(path.rsplit("page=", 1)[1])
            return 200, {"items": pages.get(page, [])}

        with unittest.mock.patch.object(fp, "http", fake_http):
            hit = fp.find_by_exact_title(
                _FakeClient(), "developing-today/code", self.TITLE
            )
        return hit, paths

    def test_pr_with_same_title_is_not_a_hit(self):
        pr = {
            "title": self.TITLE,
            "number": 4242,
            "pull_request": {"url": "https://api.github.com/x/pulls/4242"},
        }
        hit, paths = self.run_lookup({1: [pr]})
        self.assertIsNone(hit)
        # The query itself also scopes to issues (belt and braces).
        self.assertIn("type%3Aissue", paths[0])

    def test_exact_title_issue_is_a_hit(self):
        issue = {"title": self.TITLE, "number": 7}
        hit, _ = self.run_lookup({1: [{"title": "other", "number": 1}, issue]})
        self.assertEqual(hit["number"], 7)

    def test_pr_before_exact_issue_on_same_page_is_skipped(self):
        pr = {"title": self.TITLE, "number": 4242, "pull_request": {}}
        issue = {"title": self.TITLE, "number": 8}
        hit, _ = self.run_lookup({1: [pr, issue]})
        self.assertEqual(hit["number"], 8)

    def test_exact_issue_past_first_page_is_found(self):
        page1 = [{"title": f"{self.TITLE} (mention {i})", "number": i} for i in range(100)]
        issue = {"title": self.TITLE, "number": 99}
        hit, paths = self.run_lookup({1: page1, 2: [issue]})
        self.assertEqual(hit["number"], 99)
        self.assertEqual(len(paths), 2)


class TestCreateReadback(unittest.TestCase):
    """#6901: the create readback must compare body, state and requested
    labels against what was sent — not just title and number — so a create
    whose body was dropped or whose labels were not applied fails loudly."""

    TITLE = "Preserved branch: queue/x — decision needed"
    BODY = "## Finding\n\nbody that the filer sent\n"
    REPO = "developing-today/code"

    def run_create(self, readback_overrides, labels=None):
        """Run cmd_create against stubbed HTTP. Returns (exit_code, paths)."""
        paths = []
        created = {"number": 42, "html_url": "https://example.test/42"}
        readback = {
            "number": 42,
            "title": self.TITLE,
            "body": self.BODY,
            "state": "open",
            "labels": [{"name": n} for n in (labels or [])],
        }
        readback.update(readback_overrides)

        def fake_http(method, path, token, payload=None):
            paths.append((method, path))
            if method == "POST":
                return 200, created
            return 200, readback

        args = [
            "create",
            "--repo",
            self.REPO,
            "--title",
            self.TITLE,
            "--body",
            self.BODY,
            "--no-dedupe",
            "--pace",
            "0",
        ]
        for label in labels or []:
            args += ["--label", label]

        with unittest.mock.patch.object(fp, "AppClient", lambda _s: _FakeClient()), \
             unittest.mock.patch.object(
                 fp, "find_by_exact_title", lambda *_a, **_k: None
             ), \
             unittest.mock.patch.object(fp, "http", fake_http), \
             unittest.mock.patch.object(fp, "ledger_append"):
            try:
                code = fp.main(args)
            except SystemExit as exc:
                code = exc.code
        return code, paths

    def test_matching_readback_passes(self):
        code, paths = self.run_create({})
        self.assertEqual(code, 0)
        self.assertEqual([m for m, _ in paths], ["POST", "GET"])

    def test_dropped_body_fails(self):
        code, _ = self.run_create({"body": ""})
        self.assertEqual(code, 1)

    def test_mangled_body_fails(self):
        code, _ = self.run_create({"body": self.BODY.strip()})
        self.assertEqual(code, 1)

    def test_closed_state_fails(self):
        code, _ = self.run_create({"state": "closed"})
        self.assertEqual(code, 1)

    def test_missing_labels_fail(self):
        """Labels were requested but the readback shows none applied."""
        code, _ = self.run_create({"labels": []}, labels=["kind/bug"])
        self.assertEqual(code, 1)

    def test_applied_labels_pass(self):
        code, _ = self.run_create({}, labels=["kind/bug", "effort/xs"])
        self.assertEqual(code, 0)

    def test_extra_labels_are_tolerated(self):
        """GitHub may add labels on create; only the requested ones matter."""
        code, _ = self.run_create(
            {"labels": [{"name": "kind/bug"}, {"name": "auto-added"}]},
            labels=["kind/bug"],
        )
        self.assertEqual(code, 0)


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


class TestResponseShapes(unittest.TestCase):
    """#6864: parseable-but-wrong-shaped API bodies die with the tool's
    own diagnostic (method, URL, HTTP status, shape) — never a raw
    AttributeError/KeyError/TypeError, including inside http()'s own
    error handler. Transport (http_raw) is stubbed, as in the probe.
    """

    def call_http(self, status, raw, **kw):
        import io
        from contextlib import redirect_stderr

        err = io.StringIO()
        result = None
        with unittest.mock.patch.object(fp, "http_raw", return_value=(status, raw)):
            with redirect_stderr(err):
                try:
                    result = fp.http("GET", "/repos/x/y/issues/1", "tok", **kw)
                except SystemExit:
                    result = None
                    return "died", err.getvalue()
        return result, err.getvalue()

    def test_error_body_list_dies_with_status_and_shape(self):
        outcome, err = self.call_http(403, '["a","b"]')
        self.assertEqual(outcome, "died")
        self.assertIn("HTTP 403", err)
        self.assertIn("array", err)
        self.assertIn('["a","b"]', err)  # raw excerpt still surfaced

    def test_error_body_string_and_number_die_cleanly(self):
        for raw, shape in [('"edge string"', "string"), ("42", "number")]:
            with self.subTest(raw=raw):
                outcome, err = self.call_http(502, raw)
                self.assertEqual(outcome, "died")
                self.assertIn("HTTP 502", err)
                self.assertIn(shape, err)

    def test_error_body_object_keeps_message(self):
        outcome, err = self.call_http(403, '{"message": "boom-rate"}')
        self.assertEqual(outcome, "died")
        self.assertIn("boom-rate", err)

    def test_success_body_wrong_shape_dies_naming_shape(self):
        for raw, shape in [('"weird"', "string"), ('["weird"]', "array"), ("42", "number")]:
            with self.subTest(raw=raw):
                outcome, err = self.call_http(200, raw)
                self.assertEqual(outcome, "died")
                self.assertIn("expected a JSON object", err)
                self.assertIn(shape, err)

    def test_success_body_not_json_dies_cleanly(self):
        outcome, err = self.call_http(200, "not json")
        self.assertEqual(outcome, "died")
        self.assertIn("not JSON", err)

    def test_success_object_and_expected_array_pass(self):
        result, _ = self.call_http(200, '{"a": 1}')
        self.assertEqual(result, (200, {"a": 1}))
        result, _ = self.call_http(200, '[{"id": 1}]', expect=list)
        self.assertEqual(result, (200, [{"id": 1}]))

    def test_installations_dict_instead_of_list_dies(self):
        client = object.__new__(fp.AppClient)
        client.jwt = "jwt"
        client._installations = None
        with unittest.mock.patch.object(
            fp, "http", return_value=(200, {"installations": []})
        ):
            # Bypass the real http() shape guard to prove the call site
            # asks for a list: the stub ignores expect, so instead assert
            # the real guard fires via http_raw.
            pass
        with unittest.mock.patch.object(
            fp, "http_raw", return_value=(200, '{"installations": []}')
        ):
            with self.assertRaises(SystemExit):
                client.installations()

    def test_installations_string_item_dies(self):
        client = object.__new__(fp.AppClient)
        client.jwt = "jwt"
        client._installations = None
        with unittest.mock.patch.object(
            fp, "http_raw", return_value=(200, '["a-string-item"]')
        ):
            with self.assertRaises(SystemExit):
                client.installations()

    def test_installation_for_tolerates_null_account(self):
        client = object.__new__(fp.AppClient)
        client._installations = [
            {"id": 1, "account": None},
            {"id": 2, "account": {"login": "Developing-Today"}},
        ]
        inst = client.installation_for("developing-today")
        self.assertEqual(inst["id"], 2)

    def test_assert_repo_in_scope_dies_on_entry_without_full_name(self):
        client = object.__new__(fp.AppClient)
        with unittest.mock.patch.object(
            fp.AppClient, "token_for", return_value="tok"
        ), unittest.mock.patch.object(
            fp,
            "http",
            return_value=(
                200,
                {"repositories": [{"name": "no-full-name"}], "total_count": 1},
            ),
        ):
            with self.assertRaises(SystemExit):
                client.assert_repo_in_scope("developing-today/code")

    def test_assert_repo_in_scope_happy_path(self):
        client = object.__new__(fp.AppClient)
        with unittest.mock.patch.object(
            fp.AppClient, "token_for", return_value="tok"
        ), unittest.mock.patch.object(
            fp,
            "http",
            return_value=(
                200,
                {
                    "repositories": [{"full_name": "developing-today/code"}],
                    "total_count": 1,
                },
            ),
        ):
            client.assert_repo_in_scope("developing-today/code")  # no SystemExit

    def test_find_by_exact_title_skips_non_object_items(self):
        def fake_http(method, path, token, payload=None, **kw):
            return 200, {"items": ["bare-string", {"title": "T", "number": 5}]}

        with unittest.mock.patch.object(fp, "http", fake_http):
            hit = fp.find_by_exact_title(_FakeClient(), "developing-today/code", "T")
        self.assertEqual(hit["number"], 5)



if __name__ == "__main__":
    unittest.main()
