#!/usr/bin/env python3
"""Keep Dependabot npm targets manifest-backed and exclude tarball builds."""

import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / ".github" / "dependabot.yml"
TARBALL_DIRS = {
    "pkgs/cf",
    "pkgs/grok-cli",
    "pkgs/supergateway",
    "pkgs/wrangler",
    "pkgs/command-code",
    "pkgs/desktop-commander",
}
BUN_DIRS = {"pm-stack/voice-line-worker", "pm-stack/voice-line-worker/ci"}


def npm_block(text):
    blocks = re.findall(
        r'(?ms)^  - package-ecosystem: "npm"\n(.*?)(?=^  - package-ecosystem:|\Z)',
        text,
    )
    if len(blocks) != 1:
        raise AssertionError(f"expected one npm update block, found {len(blocks)}")
    return blocks[0]


def list_values(block, key):
    match = re.search(rf"(?m)^    {re.escape(key)}:\n((?:      - .*\n)+)", block)
    if not match:
        raise AssertionError(f"missing {key} list in npm block")
    return [line.strip()[2:].strip().strip('"') for line in match.group(1).splitlines()]


class DependabotNpmTargetTests(unittest.TestCase):
    def test_every_npm_target_has_a_manifest_and_tarball_dirs_are_not_targets(self):
        block = npm_block(CONFIG.read_text())
        targets = {path.lstrip("/") or "." for path in list_values(block, "directories")}
        exclusions = set(list_values(block, "exclude-paths"))

        tracked_manifests = subprocess.check_output(
            ["git", "-C", str(ROOT), "ls-files", "**/package.json", "package.json"],
            text=True,
        ).splitlines()
        manifest_dirs = {
            str(Path(path).parent) if Path(path).parent != Path(".") else "."
            for path in tracked_manifests
        }
        # The repository root is intentionally not an npm target: root
        # discovery was the path that let lock-only subdirectories enter
        # the security update job without a package.json.
        expected = manifest_dirs - TARBALL_DIRS - BUN_DIRS - {"."}

        self.assertEqual(targets, expected, "npm directories must match tracked npm manifests")
        for directory in TARBALL_DIRS:
            self.assertTrue(
                any(path.startswith(f"{directory}/") or path == f"{directory}/**" for path in exclusions),
                f"tarball-built npm directory is not excluded: {directory}",
            )
        self.assertNotIn(".", targets, "root npm discovery can select lock-only package directories")


if __name__ == "__main__":
    unittest.main()
