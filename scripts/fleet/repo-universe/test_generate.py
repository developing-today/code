#!/usr/bin/env python3
"""Tests for the repo-universe generator (#1650).

Covers the packet's three named tests:
- overrides survival: a hand edit in the artifact's overrides section
  is still present (and applied) after regeneration;
- idempotence: two consecutive runs on unchanged inputs produce
  byte-identical output;
- coverage: the four fleet repos and the four side repos named in
  #1511/#1650 are present and class `owned`.
"""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
GENERATE = HERE / "generate.py"

FLEET_AND_SIDE = [
    "developing-today/code",
    "developing-today/opencode",
    "developing-today/openchamber",
    "dezren39/nix",
    "dezren39/hatch",
    "dezren39/hatch-personal",
    "developing-today/hardware-doc",
    "developing-today/opencode-auto-continue",
]


def repo(name: str, fork: bool = False) -> dict:
    return {"full_name": name, "html_url": f"https://github.com/{name}", "fork": fork}


ORG = [repo(n) for n in FLEET_AND_SIDE if n.startswith("developing-today/")]
ORG.append(repo("developing-today/some-fork", fork=True))
USER = [repo(n) for n in FLEET_AND_SIDE if n.startswith("dezren39/")]
USER.append(repo("dezren39/scratch"))

CENSUS = """# All-repos census — fixture

| /home/user/git/code | https://github.com/developing-today/code.git | main |
| /home/user/roc | https://github.com/roc-lang/roc | main |
| /home/user/repo-archive/hardware-doc/devices/m5stack/source-snapshots/M5GFX | https://github.com/m5stack/M5GFX.git | master |
| /home/user/repo-archive/hardware-doc/software/crosspoint/source-snapshots/crosspoint-reader | https://github.com/crosspoint-reader/crosspoint-reader.git | develop |
| /home/user/private-side | https://github.com/dezren39/private-side.git | main |
"""


class GenerateTest(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = Path(tempfile.mkdtemp())
        (self.dir / "org.json").write_text(json.dumps(ORG))
        (self.dir / "user.json").write_text(json.dumps(USER))
        (self.dir / "census.md").write_text(CENSUS)
        self.out = self.dir / "repo-universe.yaml"

    def run_gen(self) -> str:
        subprocess.run(
            [sys.executable, str(GENERATE),
             "--org-json", str(self.dir / "org.json"),
             "--user-json", str(self.dir / "user.json"),
             "--census", str(self.dir / "census.md"),
             "--census-label", "fixture census",
             "--output", str(self.out)],
            check=True, capture_output=True, text=True,
        )
        return self.out.read_text()

    def test_multipage_capture_files_decode(self) -> None:
        # Captures taken with the documented `gh api --paginate`
        # commands hold one JSON array per page concatenated; the
        # generator must decode every page, not just the first.
        (self.dir / "org.json").write_text(json.dumps(ORG[:2]) + "\n" + json.dumps(ORG[2:]))
        (self.dir / "user.json").write_text(json.dumps(USER[:1]) + json.dumps(USER[1:]))
        out = self.run_gen()
        for name in FLEET_AND_SIDE:
            self.assertIn(name, out)

    def test_live_refuses_wrong_authenticated_account(self) -> None:
        # --live enumerates user/repos?affiliation=owner, i.e. the
        # AUTHENTICATED account's repos. Run as anyone but dezren39 it
        # must fail before writing, never overwrite the artifact with
        # the wrong universe.
        spec = importlib.util.spec_from_file_location("generate", GENERATE)
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)

        def fake_run(cmd, **kwargs):
            if "--paginate" in cmd:
                return subprocess.CompletedProcess(cmd, 0, stdout="[]", stderr="")
            return subprocess.CompletedProcess(
                cmd, 0, stdout=json.dumps({"login": "some-collaborator"}), stderr="")

        live_out = self.dir / "live-out.yaml"
        with mock.patch.object(gen.subprocess, "run", side_effect=fake_run):
            with self.assertRaises(SystemExit):
                gen.main(["--live",
                          "--census", str(self.dir / "census.md"),
                          "--output", str(live_out)])
        self.assertFalse(live_out.exists())

    def test_idempotent_zero_diff_on_unchanged_inputs(self) -> None:
        first = self.run_gen()
        second = self.run_gen()
        self.assertEqual(first, second)

    def test_overrides_survive_regeneration(self) -> None:
        self.run_gen()
        edited = self.out.read_text().replace(
            "overrides:\n  {}",
            "overrides:\n  roc-lang/roc:\n    class: upstream-dependency\n    watch: issues",
        )
        self.assertNotEqual(edited, self.out.read_text())
        self.out.write_text(edited)
        regenerated = self.run_gen()
        # The override block survives verbatim...
        self.assertIn("  roc-lang/roc:\n    class: upstream-dependency\n    watch: issues", regenerated)
        # ...and is applied to the entry.
        self.assertIn("- name: roc-lang/roc\n    url: https://github.com/roc-lang/roc\n"
                      "    class: upstream-dependency\n    watch: issues", regenerated)
        # And regeneration with the override is itself idempotent.
        self.assertEqual(regenerated, self.run_gen())

    def test_coverage_fleet_and_side_repos_are_owned(self) -> None:
        artifact = self.run_gen()
        for name in FLEET_AND_SIDE:
            self.assertIn(f"- name: {name}\n    url: https://github.com/{name}\n"
                          f"    class: owned\n    watch: issues", artifact,
                          f"{name} must be present and owned")

    def test_override_class_change_moves_the_watch_default(self) -> None:
        self.run_gen()
        edited = self.out.read_text().replace(
            "overrides:\n  {}",
            "overrides:\n  developing-today/some-fork:\n    class: owned",
        )
        self.out.write_text(edited)
        regenerated = self.run_gen()
        self.assertIn("- name: developing-today/some-fork\n    url: https://github.com/developing-today/some-fork\n"
                      "    class: owned\n    watch: issues", regenerated)

    def test_classification(self) -> None:
        artifact = self.run_gen()
        self.assertIn("- name: developing-today/some-fork\n    url: https://github.com/developing-today/some-fork\n"
                      "    class: fork\n    watch: inventory-only", artifact)
        self.assertIn("- name: m5stack/M5GFX\n    url: https://github.com/m5stack/M5GFX\n"
                      "    class: archive-nested\n    watch: inventory-only", artifact)
        self.assertIn("- name: crosspoint-reader/crosspoint-reader\n"
                      "    url: https://github.com/crosspoint-reader/crosspoint-reader\n"
                      "    class: archive-nested", artifact)
        self.assertIn("- name: dezren39/private-side\n    url: https://github.com/dezren39/private-side\n"
                      "    class: owned\n    watch: issues", artifact)
        self.assertIn("- name: dezren39/private-side\n    url: https://github.com/dezren39/private-side\n"
                      "    class: owned\n    watch: issues", artifact)
        self.assertIn("- name: roc-lang/roc\n    url: https://github.com/roc-lang/roc\n"
                      "    class: upstream-dependency\n    watch: inventory-only", artifact)


if __name__ == "__main__":
    unittest.main()
