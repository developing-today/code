#!/usr/bin/env python3
"""Oracle tests for check_deployed_version.check.

Oracle: the predicate from commit 6b059084's "Check the deployed Worker
answers" step — print the wanted SHA only when the health payload says
ok + edge + that version AND deployedAt/ageSeconds describe the real
deploy time within 5 seconds. Every other payload prints "".
"""

import datetime
import io
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import check_deployed_version as cdv

WANT = "8012c1ac1707c32e76e3f757ac5168d380f45ce8"


def payload(**over):
    now = datetime.datetime.now(datetime.timezone.utc)
    base = {
        "ok": True,
        "edge": True,
        "version": WANT,
        "deployedAt": (now - datetime.timedelta(seconds=30)).isoformat(),
        "ageSeconds": 30,
    }
    base.update(over)
    return base


class CheckDeployedVersion(unittest.TestCase):
    def test_truthful_answer_prints_the_wanted_sha(self):
        self.assertEqual(cdv.check(payload(), WANT), WANT)

    def test_wrong_version_prints_empty(self):
        self.assertEqual(cdv.check(payload(version="deadbeef"), WANT), "")

    def test_not_ok_or_not_edge_prints_empty(self):
        self.assertEqual(cdv.check(payload(ok=False), WANT), "")
        self.assertEqual(cdv.check(payload(edge=False), WANT), "")

    def test_missing_keys_print_empty(self):
        self.assertEqual(cdv.check({}, WANT), "")
        self.assertEqual(cdv.check({"ok": True, "edge": True}, WANT), "")

    def test_non_dict_payload_prints_empty(self):
        self.assertEqual(cdv.check([1, 2], WANT), "")
        self.assertEqual(cdv.check("ok", WANT), "")

    def test_lying_age_prints_empty(self):
        self.assertEqual(cdv.check(payload(ageSeconds=3000), WANT), "")

    def test_negative_age_prints_empty(self):
        self.assertEqual(cdv.check(payload(ageSeconds=-5), WANT), "")

    def test_future_deployed_at_prints_empty(self):
        future = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(
            seconds=600
        )
        self.assertEqual(
            cdv.check(payload(deployedAt=future.isoformat(), ageSeconds=0), WANT),
            "",
        )

    def test_malformed_timestamp_prints_empty(self):
        self.assertEqual(cdv.check(payload(deployedAt="not-a-time"), WANT), "")
        self.assertEqual(cdv.check(payload(deployedAt=None), WANT), "")

    def test_non_integer_age_prints_empty(self):
        self.assertEqual(cdv.check(payload(ageSeconds="thirty"), WANT), "")

    def test_main_reads_stdin_and_prints_sha(self):
        old_in, old_out = sys.stdin, sys.stdout
        try:
            sys.stdin = io.StringIO(__import__("json").dumps(payload()))
            sys.stdout = io.StringIO()
            self.assertEqual(cdv.main(["check_deployed_version.py", WANT]), 0)
            self.assertEqual(sys.stdout.getvalue(), WANT + "\n")
        finally:
            sys.stdin, sys.stdout = old_in, old_out

    def test_main_with_bad_json_prints_empty(self):
        old_in, old_out = sys.stdin, sys.stdout
        try:
            sys.stdin = io.StringIO("{not json")
            sys.stdout = io.StringIO()
            self.assertEqual(cdv.main(["check_deployed_version.py", WANT]), 0)
            self.assertEqual(sys.stdout.getvalue(), "\n")
        finally:
            sys.stdin, sys.stdout = old_in, old_out

    def test_main_without_argv_prints_empty_and_exits_2(self):
        old_out = sys.stdout
        try:
            sys.stdout = io.StringIO()
            self.assertEqual(cdv.main(["check_deployed_version.py"]), 2)
            self.assertEqual(sys.stdout.getvalue(), "\n")
        finally:
            sys.stdout = old_out


if __name__ == "__main__":
    unittest.main()
