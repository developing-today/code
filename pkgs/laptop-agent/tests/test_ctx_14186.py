#!/usr/bin/env python3
"""Build 1/5 (#14186): init sourcing contract + laptop job-spool ctx.

Oracle: a POST /jobs carrying FLEET_CTX_* env persists exactly that
subset (ids/hashes only) in the spool's meta.json, serves it on poll,
and survives a daemon restart; pre-contract spools without ctx restore
with ctx == {}; a malformed ctx skips the entry per the established
skip contract. Non-FLEET_CTX_* env never lands in ctx.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests
"""

import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"
INIT_SH = Path(__file__).resolve().parents[3] / "pkgs" / "fleet" / "fleet-ctx-init.sh"


def _clean_env(**overrides):
    """Env without any FLEET_CTX_* record plus PATH/HOME, with overrides."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("FLEET_CTX_")}
    env.update(overrides)
    env.setdefault("PATH", "/usr/bin:/bin")
    env.setdefault("HOME", str(Path.home()))
    # Scrub origin signals the caller did not explicitly set.
    for sig in (
        "OPENCODE_SESSION_ID",
        "OPENCODE",
        "AI_AGENT",
        "MCPX_HARNESS",
        "CLAUDECODE",
        "CLAUDE_CODE",
        "CLAUDE_SESSION_ID",
        "ANTHROPIC_SESSION",
        "SSH_TTY",
        "SSH_CLIENT",
        "SSH_CONNECTION",
        "INVOCATION_ID",
        "SYSTEMD_EXEC_PID",
        "JOURNAL_STREAM",
    ):
        if sig not in overrides:
            env.pop(sig, None)
    return env


def _sourced_env(clean_env, extra_bash_args=()):
    """Source INIT_SH in bash, then print the FLEET_CTX_* record as JSON."""
    probe = (
        'source "$INIT_SH"; '
        "rc=$?; "
        'printf \'{"rc":%s,\' "$rc"; '
        'printf \'"ORIGIN":"%s",\' "$FLEET_CTX_ORIGIN"; '
        'printf \'"HOST":"%s",\' "$FLEET_CTX_HOST"; '
        'printf \'"TASK":"%s",\' "${FLEET_CTX_TASK_ID:-}"; '
        'printf \'"ID":"%s"}\' "$FLEET_CTX_ID"'
    )
    p = subprocess.run(
        ["bash", *extra_bash_args, "-c", probe],
        env={**clean_env, "INIT_SH": str(INIT_SH)},
        capture_output=True,
        text=True,
        timeout=30,
    )
    return p


def _import_agent():
    with tempfile.NamedTemporaryFile("w", suffix="-token", delete=False) as f:
        f.write("ctx-14186-test-token\n")
        token_path = f.name
    os.environ.setdefault("LAPTOP_AGENT_TOKEN_FILE", token_path)
    sys.path.insert(0, str(AGENT.parent))
    import laptop_agent as la

    return la


class CtxExtractTest(unittest.TestCase):
    def test_extract_ctx_filters_prefix_and_strings(self):
        la = _import_agent()
        env = {
            "FLEET_CTX_TASK_ID": "23046",
            "FLEET_CTX_ORIGIN": "opencode",
            "PATH": "/bin",
            "SECRET_TOKEN": "shh",
            "FLEET_CTX_NUM": 123,
        }
        ctx = la._extract_ctx(env)
        self.assertEqual(
            ctx, {"FLEET_CTX_TASK_ID": "23046", "FLEET_CTX_ORIGIN": "opencode"}
        )

    def test_extract_ctx_non_dict_is_empty(self):
        la = _import_agent()
        self.assertEqual(la._extract_ctx(None), {})
        self.assertEqual(la._extract_ctx("x"), {})


class CtxPersistTest(unittest.TestCase):
    def _load(self, la, state_dir):
        la.JOBS.clear()
        err = io.StringIO()
        with patch.object(la, "STATE_DIR", Path(state_dir)):
            with contextlib.redirect_stderr(err):
                la.load_persisted_jobs()
        return err.getvalue()

    def tearDown(self):
        la_mod = sys.modules.get("laptop_agent")
        if la_mod is not None:
            la_mod.JOBS.clear()

    def test_ctx_round_trip(self):
        la = _import_agent()
        job = {
            "id": "ctx-job",
            "proc": None,
            "cwd": "/",
            "cmd_repr": "echo hi",
            "started_at": 100.0,
            "ended_at": 150.0,
            "exit": 0,
            "state": "done",
            "interrupted": False,
            "ctx": {
                "FLEET_CTX_TASK_ID": "23046",
                "FLEET_CTX_ORIGIN": "opencode",
                "FLEET_CTX_HOST": "amd",
                "FLEET_CTX_ID": "task-23046",
            },
            "_files": (),
        }
        with tempfile.TemporaryDirectory() as state_dir:
            with patch.object(la, "STATE_DIR", Path(state_dir)):
                la._write_meta(job)
            raw = json.loads(
                (Path(state_dir) / "jobs" / "ctx-job" / "meta.json").read_text()
            )
            self.assertEqual(raw["ctx"], job["ctx"])
            stderr = self._load(la, state_dir)
            self.assertEqual(stderr, "")
            restored = la.JOBS["ctx-job"]
            self.assertEqual(restored["ctx"], job["ctx"])

    def test_pre_contract_spool_restores_empty_ctx(self):
        la = _import_agent()
        meta = {
            "id": "old-job",
            "cwd": "/",
            "cmd": "echo old",
            "started_at": 100.0,
            "ended_at": 150.0,
            "exit": 0,
            "state": "done",
            "interrupted": False,
        }
        with tempfile.TemporaryDirectory() as state_dir:
            spool = Path(state_dir) / "jobs" / "old-job"
            spool.mkdir(parents=True)
            (spool / "meta.json").write_text(json.dumps(meta))
            (spool / "stdout").write_bytes(b"")
            (spool / "stderr").write_bytes(b"")
            stderr = self._load(la, state_dir)
            self.assertEqual(stderr, "")
            self.assertEqual(la.JOBS["old-job"]["ctx"], {})

    def test_malformed_ctx_skips(self):
        la = _import_agent()
        for bad in ("x", 123, ["a"], {"FLEET_CTX_TASK_ID": 123}):
            meta = {
                "id": "bad-job",
                "cwd": "/",
                "cmd": "test",
                "started_at": 100.0,
                "ended_at": 150.0,
                "exit": 0,
                "state": "done",
                "interrupted": False,
                "ctx": bad,
            }
            with tempfile.TemporaryDirectory() as state_dir:
                spool = Path(state_dir) / "jobs" / "bad-job"
                spool.mkdir(parents=True)
                (spool / "meta.json").write_text(json.dumps(meta))
                (spool / "stdout").write_bytes(b"")
                (spool / "stderr").write_bytes(b"")
                stderr = self._load(la, state_dir)
                self.assertNotIn("bad-job", la.JOBS)
                self.assertIn("bad-job", stderr)


class InitSourcingTest(unittest.TestCase):
    """Acceptance 1: sourcing sets the record; children inherit it."""

    def test_source_sets_vars_and_child_inherits(self):
        script = (
            'source "$INIT_SH"; '
            'parent="$FLEET_CTX_ORIGIN|$FLEET_CTX_HOST|$FLEET_CTX_ID"; '
            "child=$(bash -c 'echo \"$FLEET_CTX_ORIGIN|$FLEET_CTX_HOST|$FLEET_CTX_ID\"'); "
            'test -n "$FLEET_CTX_ORIGIN" && test -n "$FLEET_CTX_HOST" && test -n "$FLEET_CTX_ID"; '
            "rc=$?; "
            'test "$parent" = "$child"; '
            "eq=$?; "
            'printf \'{"rc":%s,"eq":%s,"parent":"%s"}\' "$rc" "$eq" "$parent"'
        )
        p = subprocess.run(
            ["bash", "-c", script],
            env={**_clean_env(), "INIT_SH": str(INIT_SH)},
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads(p.stdout)
        self.assertEqual(rec["rc"], 0, rec)
        self.assertEqual(rec["eq"], 0, rec)
        self.assertTrue(rec["parent"].count("|") == 2, rec)

    def test_execute_refuses_fail_closed(self):
        p = subprocess.run(
            ["bash", str(INIT_SH)],
            env=_clean_env(),
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("source", p.stderr.lower())

    def test_inherited_task_id_kept_and_id_derived(self):
        p = _sourced_env(_clean_env(FLEET_CTX_TASK_ID="23046"))
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads(p.stdout)
        self.assertEqual(rec["TASK"], "23046")
        self.assertEqual(rec["ID"], "task-23046")
        # Re-sourcing never invents or overwrites an inherited task id.
        child = subprocess.run(
            [
                "bash",
                "-c",
                'source "$INIT_SH"; source "$INIT_SH"; echo "$FLEET_CTX_TASK_ID|$FLEET_CTX_ID"',
            ],
            env={**_clean_env(FLEET_CTX_TASK_ID="23046"), "INIT_SH": str(INIT_SH)},
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(child.stdout.strip(), "23046|task-23046")

    def test_partial_record_without_task_id_never_errors(self):
        p = _sourced_env(_clean_env())
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads(p.stdout)
        self.assertEqual(rec["rc"], 0)
        self.assertEqual(rec["TASK"], "")
        self.assertTrue(rec["ID"].startswith(f"{rec['ORIGIN']}-"))


class OriginTaxonomyTest(unittest.TestCase):
    """Acceptance 2: laptop origin distinction; absence is partial, not error."""

    def _origin(self, **overrides):
        p = _sourced_env(_clean_env(**overrides))
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads(p.stdout)
        self.assertEqual(rec["rc"], 0)
        return rec["ORIGIN"]

    def test_opencode(self):
        self.assertEqual(self._origin(OPENCODE_SESSION_ID="ses_test"), "opencode")

    def test_claude(self):
        self.assertEqual(self._origin(CLAUDECODE="1"), "claude")

    def test_drew_interactive_ssh(self):
        self.assertEqual(
            self._origin(SSH_CONNECTION="1.2.3.4 1 5.6.7.8 2"), "drew-interactive"
        )

    def test_systemd_unit_wins_with_qualifier(self):
        self.assertEqual(
            self._origin(INVOCATION_ID="abc123", OPENCODE_SESSION_ID="ses_test"),
            "systemd-unit:opencode",
        )
        self.assertEqual(self._origin(INVOCATION_ID="abc123"), "systemd-unit")

    def test_unknown_when_no_signals(self):
        # Non-interactive `bash -c` has no `i` in $- and no signals set.
        self.assertEqual(self._origin(), "unknown")


if __name__ == "__main__":
    unittest.main()
