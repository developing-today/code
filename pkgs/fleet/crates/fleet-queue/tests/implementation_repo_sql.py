"""Oracle: production SQLite claim predicates select work target and fitting window.
In-memory only; no broker, credentials, or live claims. Uses production expressions,
not a second hand-written repo/budget selection algorithm.
"""

import itertools
import json
import pathlib
import re
import sqlite3
import unittest

SRC = (
    pathlib.Path(__file__).resolve().parents[1] / "src/turso_queue/claim.rs"
).read_text()


def constant(name):
    raw = re.search(r"const " + name + r': &str = "(.*?)";', SRC, re.S).group(1)
    return re.sub(r"\\\n\s*", "", raw).replace('\\"', '"')


def budget_gate():
    body = re.search(r'fn budget_gate_sql\(\).*?format!\("(.*?)"\)', SRC, re.S).group(1)
    return body.replace("{REQUIRED_BUDGET_EXPR}", constant("REQUIRED_BUDGET_EXPR"))


class ImplementationRepoSQL(unittest.TestCase):
    def test_target_and_budget_matrix(self):
        # Model: an explicitly declared target overrides tracking, and a declared
        # requirement fits iff the claimant provides at least that much time.
        db = sqlite3.connect(":memory:")
        db.execute(
            "CREATE TABLE tasks(queue TEXT, payload TEXT, required_budget_minutes INTEGER)"
        )
        subject = constant("IMPLEMENTATION_REPO_SQL")
        self.assertIn("{IMPLEMENTATION_REPO_SQL} = ?", SRC)
        for target, requirement, remaining in itertools.product(
            [None, "developing-today/code", "developing-today/kaneo"],
            [None, 45, 180],
            [None, 30, 45, 179, 180, 240],
        ):
            payload = {"repo": "developing-today/code"}
            if target is not None:
                payload["implementation_repo"] = target
            if requirement is not None:
                payload["required_budget_minutes"] = requirement
            db.execute("DELETE FROM tasks")
            db.execute(
                "INSERT INTO tasks VALUES(?,?,NULL)",
                ("repo-health-code-pilot", json.dumps(payload)),
            )
            for repo in ["code", "kaneo"]:
                actual = db.execute(
                    "SELECT COUNT(*) FROM tasks WHERE ("
                    + subject
                    + " = ? OR "
                    + subject
                    + " LIKE '%/' || ?) AND "
                    + budget_gate(),
                    (repo, repo, remaining),
                ).fetchone()[0]
                expected = (target or "developing-today/code").endswith(
                    "/" + repo
                ) and (
                    requirement is None
                    or (remaining is not None and remaining >= requirement)
                )
                self.assertEqual(
                    actual, int(expected), (target, requirement, remaining, repo)
                )
                negated = db.execute(
                    "SELECT COUNT(*) FROM tasks WHERE NOT ("
                    + subject
                    + " = ? OR "
                    + subject
                    + " LIKE '%/' || ?) AND "
                    + budget_gate(),
                    (repo, repo, remaining),
                ).fetchone()[0]
                expected_not = not (target or "developing-today/code").endswith(
                    "/" + repo
                ) and (
                    requirement is None
                    or (remaining is not None and remaining >= requirement)
                )
                self.assertEqual(
                    negated, int(expected_not), (target, requirement, remaining, repo)
                )

    def test_park_requirement_overrides_packet_requirement(self):
        db = sqlite3.connect(":memory:")
        db.execute("CREATE TABLE tasks(payload TEXT, required_budget_minutes INTEGER)")
        db.execute(
            "INSERT INTO tasks VALUES(?,180)", ('{"required_budget_minutes":45}',)
        )
        for window, matches in [(45, 0), (179, 0), (180, 1)]:
            self.assertEqual(
                db.execute(
                    "SELECT COUNT(*) FROM tasks WHERE " + budget_gate(), (window,)
                ).fetchone()[0],
                matches,
            )


if __name__ == "__main__":
    unittest.main()
