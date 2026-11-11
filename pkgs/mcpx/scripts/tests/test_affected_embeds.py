#!/usr/bin/env python3
"""Regression tests for //go:embed ownership in affected-tests.py."""

import os
import shutil
import subprocess
import sys
import tempfile
import unittest

GIT = shutil.which("git")
TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
SELECTOR = os.path.normpath(os.path.join(TESTS_DIR, "..", "affected-tests.py"))
MODULE = "github.com/developing-today/code/pkgs/mcpx"


def _git(*args, cwd):
    env = dict(
        os.environ,
        GIT_AUTHOR_NAME="fixture",
        GIT_AUTHOR_EMAIL="fixture@example.com",
        GIT_COMMITTER_NAME="fixture",
        GIT_COMMITTER_EMAIL="fixture@example.com",
    )
    return subprocess.run(
        [GIT, *args], cwd=cwd, capture_output=True, text=True, check=True, env=env
    )


@unittest.skipUnless(GIT, "git is required to build selector fixtures")
class EmbeddedOwnerFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="affected-embed-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.mcpx = os.path.join(self.tmp, "pkgs", "mcpx")
        _git("init", "-q", "-b", "main", cwd=self.tmp)
        self.write("go.mod", "module %s\n\ngo 1.23\n" % MODULE)

    def write(self, rel, content):
        full = os.path.join(self.mcpx, rel)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as f:
            f.write(content)

    def commit(self):
        _git("add", "-A", cwd=self.tmp)
        _git("-c", "commit.gpgsign=false", "commit", "-qm", "base", cwd=self.tmp)
        return _git("rev-parse", "HEAD", cwd=self.tmp).stdout.strip()

    def run_selector(self, base):
        env = dict(os.environ, GOPROXY="off", GOTOOLCHAIN="local")
        env["PATH"] = os.pathsep.join(
            [os.path.dirname(GIT), os.environ.get("PATH", "")]
        )
        proc = subprocess.run(
            [sys.executable, SELECTOR, "--base", base],
            cwd=self.tmp,
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertEqual(proc.returncode, 0, msg=proc.stderr)
        return proc.stdout

    def assert_selected(self, output, pkg):
        self.assertIn(f"(cd pkgs/mcpx && go vet ./{pkg}/)", output)
        self.assertIn(f"(cd pkgs/mcpx && go test -count=1 ./{pkg}/)", output)

    def test_all_prefixed_skills_glob_selects_vet_and_tests(self):
        self.write(
            "plugin/opencode/skills/skills.go",
            'package skills\n\nimport "embed"\n\n'
            "//go:embed all:mcpx-*\nvar FS embed.FS\n",
        )
        self.write(
            "plugin/opencode/skills/mcpx-basics/SKILL.md", "# Original skill\n"
        )
        base = self.commit()

        self.write(
            "plugin/opencode/skills/mcpx-basics/SKILL.md", "# Updated skill\n"
        )
        output = self.run_selector(base)

        self.assert_selected(output, "plugin/opencode/skills")
        self.assertIn("owning package(s) selected: plugin/opencode/skills", output)
        self.assertNotIn("no tier commands selected", output)

    def test_all_prefix_supports_exact_directory_and_glob_patterns(self):
        self.write(
            "internal/allprefix/embed.go",
            'package allprefix\n\nimport "embed"\n\n'
            "//go:embed all:exact.txt all:assets all:*.tmpl\n"
            "var FS embed.FS\n",
        )
        self.write("internal/allprefix/exact.txt", "original\n")
        self.write("internal/allprefix/assets/icon.svg", "<svg>original</svg>\n")
        self.write("internal/allprefix/example.tmpl", "original\n")
        base = self.commit()

        self.write("internal/allprefix/exact.txt", "changed\n")
        self.write(
            "internal/allprefix/assets/icon.svg", "<svg>changed</svg>\n"
        )
        self.write("internal/allprefix/example.tmpl", "changed\n")
        output = self.run_selector(base)

        self.assert_selected(output, "internal/allprefix")

    def test_plain_embed_controls_keep_their_owners(self):
        fixtures = {
            "internal/dashboard": ("dashboard.html", "<html>original</html>\n"),
            "internal/mcpspec": ("schema/mcp.schema.json", '{"v":1}\n'),
            "internal/defaults": ("defaults.json", '{"v":1}\n'),
            "internal/plain-glob": ("example.txt", "original\n"),
        }
        patterns = {
            "internal/dashboard": "dashboard.html",
            "internal/mcpspec": "schema",
            "internal/defaults": "defaults.json",
            "internal/plain-glob": "*.txt",
        }
        for pkg, (rel, content) in fixtures.items():
            self.write(
                f"{pkg}/embed.go",
                f'package embeds\n\nimport "embed"\n\n'
                f"//go:embed {patterns[pkg]}\nvar FS embed.FS\n",
            )
            self.write(f"{pkg}/{rel}", content)
        base = self.commit()

        for pkg, (rel, content) in fixtures.items():
            changed = content.replace("original", "changed").replace("1", "2")
            self.write(f"{pkg}/{rel}", changed)
        output = self.run_selector(base)

        for pkg in fixtures:
            self.assert_selected(output, pkg)


if __name__ == "__main__":
    unittest.main()
