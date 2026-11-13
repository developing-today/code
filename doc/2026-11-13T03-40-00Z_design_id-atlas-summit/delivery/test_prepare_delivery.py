#!/usr/bin/env python3
"""Offline validator/publisher guard regressions; no live calls or writes."""
import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("prepare_delivery", HERE / "prepare_delivery.py")
prep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prep)


def fixture(*, title="Slice", statements=None, file="pkgs/id/tests/new_slice.rs", dependency=None):
    item = {
        "id": "fixture-slice", "parent": 9110, "title": title,
        "goal": "Specify one bounded behavior and its implementation boundary.",
        "enables": "A separately reviewable contract increment.", "version": "V2",
        "tier": "crucial", "kind": "implementation", "owner": "fixture-owner",
        "statements": statements or ["S0.1"], "rollback": "Revert this slice's files only.",
        "design_source": "crosscut/versioned-design.md", "design_section": "Decision summary",
        "owned_files": [{"path": "pkgs/id/tests/new_slice.rs", "status": "new"}],
        "tests": {"file": file, "command": "cargo test --manifest-path pkgs/id/Cargo.toml --test new_slice",
                  "oracle": "Assert the reference model returns exactly the expected state for this fixture."},
        "requires": [], "after": [],
    }
    if dependency is not None: item["after"] = [dependency]
    return {"schema_version": 1, "slices": [item]}


class DeliveryGateTest(unittest.TestCase):
    def validate_fixture(self, data):
        with tempfile.TemporaryDirectory(dir=HERE) as temp:
            path = Path(temp) / "implementation-slices.json"
            path.write_text(json.dumps(data))
            return prep.validate([path])

    def test_soft_dependency_is_not_in_hard_graph(self):
        errors = []
        hard, soft = prep.normalize_dependencies({"after": ["another-slice"]}, "x", errors)
        self.assertEqual(errors, [])
        self.assertEqual(hard, [])
        self.assertEqual(soft[0]["strength"], "soft")

    def test_unknown_statement_and_missing_title_are_rejected(self):
        data = fixture(title="")
        data["slices"][0]["statements"] = ["S0.999"]
        _, errors, _ = self.validate_fixture(data)
        self.assertTrue(any("missing exact title" in x for x in errors))
        self.assertTrue(any("statement-to-slice/exemption mismatch" in x for x in errors))

    def test_missing_test_path_and_invalid_dependency_are_rejected(self):
        data = fixture(file="")
        data["slices"][0]["after"] = ["not an exact dependency"]
        _, errors, _ = self.validate_fixture(data)
        self.assertTrue(any("exact test_file required" in x for x in errors))
        self.assertTrue(any("dependency ref must be exact" in x for x in errors))

    def test_addressing_aliases_accept_issue_test_module_and_future_command(self):
        data = fixture()
        item = data["slices"][0]
        item["issue"] = "developing-today/code#9110"
        item.pop("parent")
        item["tests"] = ["PBT round-trip", "Unknown profile rejects"]
        item["test_module"] = "NEW new_slice.rs"
        item["acceptance_command"] = "FUTURE: cargo test --manifest-path pkgs/id/Cargo.toml --test new_slice"
        item.pop("requires")
        item["dependencies"] = {"hard": [], "soft": []}
        _, errors, _ = self.validate_fixture(data)
        self.assertFalse(any("exact test_file required" in x for x in errors))
        self.assertFalse(any("exact future test command required" in x for x in errors))

    def test_lower_tier_hard_dependency_is_rejected_but_soft_is_not(self):
        data = fixture()
        first = data["slices"][0]
        first["requires"] = ["lower"]
        second = fixture()["slices"][0]
        second.update(id="lower", title="Lower-tier prerequisite", tier="recommended", statements=["S0.2"])
        second["owned_files"] = [{"path": "pkgs/id/tests/lower.rs", "status": "new"}]
        second["tests"] = {"file": "pkgs/id/tests/lower.rs", "command": "cargo test --manifest-path pkgs/id/Cargo.toml --test lower", "oracle": "Assert the fixture matches its expected state and rejects invalid input."}
        data["slices"].append(second)
        _, errors, _ = self.validate_fixture(data)
        self.assertTrue(any("crucial depends on lower tier lower" in x for x in errors))

        first["requires"] = []
        first["after"] = ["lower"]
        _, soft_errors, _ = self.validate_fixture(data)
        self.assertFalse(any("crucial depends on lower tier lower" in x for x in soft_errors))

    def test_source_output_alias_is_refused(self):
        with self.assertRaises(ValueError):
            prep.check_output_path(prep.MAP, [prep.MAP])

    def test_unknown_cli_fails_before_output_write(self):
        target = HERE / "not-created-by-unknown-flag"
        buf = io.StringIO()
        with redirect_stdout(buf), redirect_stderr(buf), self.assertRaises(SystemExit) as result:
            prep.main(["--out", str(target), "--typo"])
        self.assertEqual(result.exception.code, 2)
        self.assertFalse(target.exists())

    def test_abbreviated_option_is_not_accepted(self):
        buf = io.StringIO()
        with redirect_stdout(buf), redirect_stderr(buf), self.assertRaises(SystemExit) as result:
            prep.main(["--app"])
        self.assertEqual(result.exception.code, 2)

    def test_current_manifests_remain_blocked_offline(self):
        _, errors, digest = prep.validate(sorted(prep.ROOT.glob("*/implementation-slices.json")))
        self.assertEqual(len(digest), 64)
        self.assertTrue(errors)


if __name__ == "__main__": unittest.main()
