"""Regression tests for the feature-plan checker CLI."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".agents/skills/feature-plan/scripts/plan_check.py"


def step(number, name, path, requires="none", parallel=False):
    marker = "[P] " if parallel else ""
    return (
        f"{number}. **{marker}{name}**\n"
        f"   - Tier: crucial. Goal: Check {name}. Enables: validation. "
        f"Requires: {requires}. Files: `{path}` Cites: #585.\n"
        "   - Test first: `python3 -m unittest`.\n"
        "   - Done when: `python3 -m unittest`.\n"
        "   - Size: S.\n"
    )


class PlanCheckTests(unittest.TestCase):
    def run_checker(self, plan):
        with tempfile.TemporaryDirectory(prefix="plan-check-test-") as temp:
            path = Path(temp) / "plan.md"
            path.write_text(plan, encoding="utf-8")
            return subprocess.run(
                [sys.executable, "-I", str(SCRIPT), str(path)],
                cwd=ROOT,
                text=True,
                capture_output=True,
                check=False,
            )

    def test_same_wave_duplicate_reports_duplicate_and_file_collision(self):
        plan = (
            "### Wave 1\n"
            + step(1, "first", "shared.md", parallel=True)
            + step(1, "second", "shared.md", parallel=True)
        )

        result = self.run_checker(plan)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("duplicate step number 1", result.stdout)
        self.assertIn("wave 1: shared.md written by steps 1, 1", result.stdout)

    def test_cross_wave_duplicate_is_rejected_and_names_both_waves(self):
        plan = (
            "### Wave 1\n"
            + step(1, "first", "first.md")
            + "### Wave 2\n"
            + step(1, "second", "second.md")
        )

        result = self.run_checker(plan)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("duplicate step number 1", result.stdout)
        self.assertIn("first in wave 1", result.stdout)
        self.assertIn("repeated in wave 2", result.stdout)

    def test_unique_step_numbers_pass(self):
        plan = (
            "### Wave 1\n"
            + step(1, "first", "first.md")
            + "### Wave 2\n"
            + step(2, "second", "second.md", requires="1")
        )

        result = self.run_checker(plan)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("ok: 2 steps in 2 waves", result.stdout)


if __name__ == "__main__":
    unittest.main()
