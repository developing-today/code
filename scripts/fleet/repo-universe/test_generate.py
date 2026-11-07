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

# The developing-today-forks org capture (#5615): opencode and
# openchamber moved there on 2026-11-04 and are GitHub forks;
# some-tool is a forks-org-owned non-fork repo.
FORKS_ORG = [
    repo("developing-today-forks/opencode", fork=True),
    repo("developing-today-forks/openchamber", fork=True),
    repo("developing-today-forks/some-tool"),
]

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
        (self.dir / "forks-org.json").write_text(json.dumps(FORKS_ORG))
        (self.dir / "user.json").write_text(json.dumps(USER))
        (self.dir / "census.md").write_text(CENSUS)
        self.out = self.dir / "repo-universe.yaml"

    def run_gen(self) -> str:
        subprocess.run(
            [
                sys.executable,
                str(GENERATE),
                "--org-json",
                str(self.dir / "org.json"),
                "--forks-org-json",
                str(self.dir / "forks-org.json"),
                "--user-json",
                str(self.dir / "user.json"),
                "--census",
                str(self.dir / "census.md"),
                "--census-label",
                "fixture census",
                "--output",
                str(self.out),
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        return self.out.read_text()

    def test_multipage_capture_files_decode(self) -> None:
        # Captures taken with the documented `gh api --paginate`
        # commands hold one JSON array per page concatenated; the
        # generator must decode every page, not just the first.
        (self.dir / "org.json").write_text(
            json.dumps(ORG[:2]) + "\n" + json.dumps(ORG[2:])
        )
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
                cmd, 0, stdout=json.dumps({"login": "some-collaborator"}), stderr=""
            )

        live_out = self.dir / "live-out.yaml"
        with mock.patch.object(gen.subprocess, "run", side_effect=fake_run):
            with self.assertRaises(SystemExit):
                gen.main(
                    [
                        "--live",
                        "--census",
                        str(self.dir / "census.md"),
                        "--output",
                        str(live_out),
                    ]
                )
        self.assertFalse(live_out.exists())

    def test_forks_org_repos_are_enumerated(self) -> None:
        # #5615: the forks-org capture is merged before classification,
        # so forks-org repos land in the artifact from the GitHub
        # enumeration itself, not via census or overrides.
        artifact = self.run_gen()
        self.assertIn(
            "- name: developing-today-forks/some-tool\n"
            "    url: https://github.com/developing-today-forks/some-tool\n"
            "    class: owned\n    watch: issues\n    sources: [github]",
            artifact,
        )
        self.assertIn(
            "- name: developing-today-forks/opencode\n"
            "    url: https://github.com/developing-today-forks/opencode\n"
            "    class: fork\n    watch: inventory-only\n    sources: [github]",
            artifact,
        )
        self.assertIn(
            "- name: developing-today-forks/openchamber\n"
            "    url: https://github.com/developing-today-forks/openchamber\n"
            "    class: fork\n    watch: inventory-only\n    sources: [github]",
            artifact,
        )

    def test_live_fetches_both_orgs(self) -> None:
        # --live must enumerate orgs/developing-today-forks/repos
        # alongside orgs/developing-today/repos (#5615) and merge the
        # captures before classification.
        spec = importlib.util.spec_from_file_location("generate", GENERATE)
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        requested: list[str] = []

        def fake_run(cmd, **kwargs):
            if "--paginate" in cmd:
                endpoint = cmd[-1]
                requested.append(endpoint)
                if "developing-today-forks" in endpoint:
                    payload = FORKS_ORG
                elif "user/repos" in endpoint:
                    payload = USER
                else:
                    payload = ORG
                return subprocess.CompletedProcess(
                    cmd, 0, stdout=json.dumps(payload), stderr=""
                )
            return subprocess.CompletedProcess(
                cmd, 0, stdout=json.dumps({"login": "dezren39"}), stderr=""
            )

        live_out = self.dir / "live-out.yaml"
        with mock.patch.object(gen.subprocess, "run", side_effect=fake_run):
            rc = gen.main(
                [
                    "--live",
                    "--census",
                    str(self.dir / "census.md"),
                    "--output",
                    str(live_out),
                ]
            )
        self.assertEqual(rc, 0)
        self.assertTrue(
            any(e.startswith("orgs/developing-today/repos") for e in requested),
            f"developing-today org not enumerated: {requested}",
        )
        self.assertTrue(
            any(e.startswith("orgs/developing-today-forks/repos") for e in requested),
            f"developing-today-forks org not enumerated: {requested}",
        )
        out = live_out.read_text()
        self.assertIn("developing-today-forks/some-tool", out)
        self.assertIn("developing-today-forks/opencode", out)

    def run_gen_omitting(self, *omit: str) -> subprocess.CompletedProcess:
        captures = {
            "--org-json": "org.json",
            "--forks-org-json": "forks-org.json",
            "--user-json": "user.json",
        }
        argv = [sys.executable, str(GENERATE)]
        for flag, filename in captures.items():
            if flag not in omit:
                argv += [flag, str(self.dir / filename)]
        argv += [
            "--census",
            str(self.dir / "census.md"),
            "--output",
            str(self.out),
        ]
        return subprocess.run(argv, capture_output=True, text=True)

    def test_offline_mode_requires_both_captures(self) -> None:
        # Omitting a capture without --live must fail, not silently
        # regenerate with that source emptied (deleting every
        # GitHub-only repo from the omitted source).
        proc = self.run_gen_omitting("--forks-org-json", "--user-json")
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(self.out.exists())

    def test_offline_mode_requires_each_capture_singly(self) -> None:
        # Each capture omitted on its own must fail, name the missing
        # flag, and write no output (#6612: omitting only
        # --forks-org-json used to exit 0 with the whole forks org
        # silently dropped from the artifact).
        for flag in ("--org-json", "--forks-org-json", "--user-json"):
            with self.subTest(flag=flag):
                proc = self.run_gen_omitting(flag)
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn(flag, proc.stderr)
                self.assertFalse(self.out.exists())

    def run_gen_raw(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                str(GENERATE),
                "--org-json",
                str(self.dir / "org.json"),
                "--forks-org-json",
                str(self.dir / "forks-org.json"),
                "--user-json",
                str(self.dir / "user.json"),
                "--census",
                str(self.dir / "census.md"),
                "--census-label",
                "fixture census",
                "--output",
                str(self.out),
            ],
            capture_output=True,
            text=True,
        )

    def assert_wrong_shape_fails_named(self, proc: subprocess.CompletedProcess) -> None:
        # #6716: a wrong-shape entry must fail with a clear error
        # naming the capture source and the entry — no bare
        # TypeError/KeyError traceback, and no artifact written.
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("org.json", proc.stderr)
        self.assertNotIn("Traceback", proc.stderr)
        self.assertNotIn("TypeError", proc.stderr)
        self.assertNotIn("KeyError", proc.stderr)
        self.assertFalse(self.out.exists())

    def test_non_dict_entry_in_page_array_fails_named(self) -> None:
        (self.dir / "org.json").write_text(json.dumps(ORG + ["oops"]))
        self.assert_wrong_shape_fails_named(self.run_gen_raw())

    def test_dict_page_without_full_name_fails_named(self) -> None:
        # A saved API error page concatenated after a valid page: the
        # object document is an entry with no full_name.
        (self.dir / "org.json").write_text(
            json.dumps(ORG) + "\n" + json.dumps({"message": "API rate limit exceeded"})
        )
        proc = self.run_gen_raw()
        self.assert_wrong_shape_fails_named(proc)
        self.assertIn("full_name", proc.stderr)

    def test_null_entry_fails_named(self) -> None:
        (self.dir / "org.json").write_text(json.dumps(ORG + [None]))
        self.assert_wrong_shape_fails_named(self.run_gen_raw())

    def test_consumers_validate_entries_directly(self) -> None:
        # build/merge_repos validate entries that never passed
        # through decode_pages, so a direct caller gets the named
        # error instead of a bare KeyError/TypeError (#6716).
        spec = importlib.util.spec_from_file_location("generate", GENERATE)
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        with self.assertRaises(ValueError):
            gen.merge_repos([{"name": "no-full-name"}])
        with self.assertRaises(ValueError):
            gen.build([{"name": "no-full-name"}], [], CENSUS, "label", {})
        with self.assertRaises(ValueError):
            gen.build([], ["oops"], CENSUS, "label", {})
        with self.assertRaises(ValueError):
            gen.decode_pages(json.dumps([{"full_name": "a/b"}, None]), source="cap X")

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
        self.assertIn(
            "  roc-lang/roc:\n    class: upstream-dependency\n    watch: issues",
            regenerated,
        )
        # ...and is applied to the entry.
        self.assertIn(
            "- name: roc-lang/roc\n    url: https://github.com/roc-lang/roc\n"
            "    class: upstream-dependency\n    watch: issues",
            regenerated,
        )
        # And regeneration with the override is itself idempotent.
        self.assertEqual(regenerated, self.run_gen())

    def test_coverage_fleet_and_side_repos_are_owned(self) -> None:
        artifact = self.run_gen()
        for name in FLEET_AND_SIDE:
            self.assertIn(
                f"- name: {name}\n    url: https://github.com/{name}\n"
                f"    class: owned\n    watch: issues",
                artifact,
                f"{name} must be present and owned",
            )

    def test_override_class_change_moves_the_watch_default(self) -> None:
        self.run_gen()
        edited = self.out.read_text().replace(
            "overrides:\n  {}",
            "overrides:\n  developing-today/some-fork:\n    class: owned",
        )
        self.out.write_text(edited)
        regenerated = self.run_gen()
        self.assertIn(
            "- name: developing-today/some-fork\n    url: https://github.com/developing-today/some-fork\n"
            "    class: owned\n    watch: issues",
            regenerated,
        )

    def test_classification(self) -> None:
        artifact = self.run_gen()
        self.assertIn(
            "- name: developing-today/some-fork\n    url: https://github.com/developing-today/some-fork\n"
            "    class: fork\n    watch: inventory-only",
            artifact,
        )
        self.assertIn(
            "- name: m5stack/M5GFX\n    url: https://github.com/m5stack/M5GFX\n"
            "    class: archive-nested\n    watch: inventory-only",
            artifact,
        )
        self.assertIn(
            "- name: crosspoint-reader/crosspoint-reader\n"
            "    url: https://github.com/crosspoint-reader/crosspoint-reader\n"
            "    class: archive-nested",
            artifact,
        )
        self.assertIn(
            "- name: dezren39/private-side\n    url: https://github.com/dezren39/private-side\n"
            "    class: owned\n    watch: issues",
            artifact,
        )
        self.assertIn(
            "- name: dezren39/private-side\n    url: https://github.com/dezren39/private-side\n"
            "    class: owned\n    watch: issues",
            artifact,
        )
        self.assertIn(
            "- name: roc-lang/roc\n    url: https://github.com/roc-lang/roc\n"
            "    class: upstream-dependency\n    watch: inventory-only",
            artifact,
        )


if __name__ == "__main__":
    unittest.main()
