#!/usr/bin/env python3
"""Tests for scripts/export-turso.sh — no network, stub curl on PATH.

Run:  python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
  or: python3 pkgs/cold-store-reaper/tests/test_export_turso.py

The guard (#1027): no Turso credential (platform token or minted
per-database JWT) may appear in any curl argv — the process list is
world-readable for up to the 300 s dump window. Credentials must
travel in a curl config fed on stdin (--config -), the pattern
cold_store.py's _curl_fetch established for #983.
"""

import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "export-turso.sh"
REPAIR = Path(__file__).resolve().parent.parent / "scripts" / "repair_dump.py"

PLATFORM_TOKEN = "fake-platform-token-SECRET"
DB_JWT = "fake-db-jwt-SECRET"

STUB_CURL = r"""#!/usr/bin/env bash
# Stub curl: capture argv + stdin, serve canned Turso responses.
set -u
cap="${STUB_CAPTURE_DIR:?}"
n=0
[[ -f "${cap}/count" ]] && n="$(cat "${cap}/count")"
n=$((n + 1))
echo "${n}" > "${cap}/count"
printf '%s\n' "$@" > "${cap}/argv-${n}.log"
cat > "${cap}/stdin-${n}.log"
out=""
url=""
prev=""
for a in "$@"; do
  if [[ "${prev}" == "-o" ]]; then out="${a}"; fi
  case "${a}" in http*) url="${a}" ;; esac
  prev="${a}"
done
body=""
case "${url}" in
  */auth/tokens) body='{"jwt":"__JWT__"}' ;;
  */dump) body='BEGIN TRANSACTION;
CREATE TABLE t (x INTEGER);
INSERT INTO t VALUES (1);
COMMIT;
' ;;
  *) body='{"databases":[{"Name":"db1","Hostname":"db1.example.test"}]}' ;;
esac
if [[ -n "${out}" ]]; then
  printf '%s' "${body}" > "${out}"
else
  printf '%s' "${body}"
fi
exit 0
"""

STUB_COLD_STORE = """#!/usr/bin/env bash
printf '%s\\n' "$@" >> "${STUB_CAPTURE_DIR:?}/cold-store.log"
exit 0
"""


class TestExportTursoArgv(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.capture = root / "capture"
        self.capture.mkdir()
        stubbin = root / "bin"
        stubbin.mkdir()
        curl = stubbin / "curl"
        curl.write_text(STUB_CURL.replace("__JWT__", DB_JWT))
        curl.chmod(curl.stat().st_mode | stat.S_IEXEC)
        cold = root / "cold-store-stub"
        cold.write_text(STUB_COLD_STORE)
        cold.chmod(cold.stat().st_mode | stat.S_IEXEC)
        self.env = {
            **os.environ,
            "PATH": f"{stubbin}:{os.environ['PATH']}",
            "STUB_CAPTURE_DIR": str(self.capture),
            "TURSO_PLATFORM_API_TOKEN": PLATFORM_TOKEN,
            "TURSO_API_BASE": "http://api.test",
            "TURSO_DUMP_BASE": "http://dump.test",
            "COLD_STORE_BIN": str(cold),
        }

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, env=None):
        return subprocess.run(
            ["bash", str(SCRIPT)],
            env=env or self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_credentials_never_in_argv_but_reach_curl_via_stdin_config(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("all 1 database(s) filed", result.stdout)
        # the script's never-print invariant holds for both secrets
        self.assertNotIn(PLATFORM_TOKEN, result.stdout + result.stderr)
        self.assertNotIn(DB_JWT, result.stdout + result.stderr)

        argv_logs = sorted(self.capture.glob("argv-*.log"))
        stdin_logs = sorted(self.capture.glob("stdin-*.log"))
        # discovery + token mint + dump download
        self.assertEqual(len(argv_logs), 3)
        self.assertEqual(len(stdin_logs), 3)
        all_stdin = ""
        for argv_log, stdin_log in zip(argv_logs, stdin_logs):
            argv = argv_log.read_text()
            # no credential, no Authorization header, in any argv
            self.assertNotIn(PLATFORM_TOKEN, argv)
            self.assertNotIn(DB_JWT, argv)
            self.assertNotIn("Authorization", argv)
            # the header instead arrives via a stdin-fed curl config
            argv_lines = argv.splitlines()
            self.assertIn("--config", argv_lines)
            self.assertEqual(argv_lines[argv_lines.index("--config") + 1], "-")
            all_stdin += stdin_log.read_text()
        self.assertEqual(all_stdin.count(f"Authorization: Bearer {PLATFORM_TOKEN}"), 2)
        self.assertEqual(all_stdin.count(f"Authorization: Bearer {DB_JWT}"), 1)
        # behaviour unchanged: the export was filed with cold-store
        self.assertIn("put-backup", (self.capture / "cold-store.log").read_text())

    def dump_argv_lines(self):
        for argv_log in sorted(self.capture.glob("argv-*.log")):
            lines = argv_log.read_text().splitlines()
            if any("dump.test" in line for line in lines):
                return lines
        self.fail("no dump invocation recorded by the stub curl")

    def test_dump_uses_default_max_time_and_one_retry(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.dump_argv_lines()
        self.assertEqual(lines[lines.index("--max-time") + 1], "1800")
        self.assertEqual(lines[lines.index("--retry") + 1], "1")
        self.assertIn("--retry-all-errors", lines)

    def test_dump_requests_compression(self):
        # --compressed keeps GB-class dumps under the sandbox proxy's
        # mid-stream cut: the wire size shrinks several-fold.
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--compressed", self.dump_argv_lines())

    def test_dump_max_time_env_override_is_honored(self):
        env = {**self.env, "TURSO_DUMP_MAX_TIME_SECONDS": "42"}
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.dump_argv_lines()
        self.assertEqual(lines[lines.index("--max-time") + 1], "42")

    def test_skip_when_token_unset(self):
        env = {k: v for k, v in self.env.items() if k != "TURSO_PLATFORM_API_TOKEN"}
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("skipped (TURSO_PLATFORM_API_TOKEN not set)", result.stdout)
        self.assertEqual(list(self.capture.glob("argv-*.log")), [])


BROKEN_SCHEMA_INSERT = (
    b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
    b" VALUES('table','chunks_fts','chunks_fts',0,"
    b"'CREATE VIRTUAL TABLE chunks_fts\n"
    b"      USING fts5(text, content='chunks', content_rowid='id')');\n"
)
ESCAPED_SCHEMA_INSERT = BROKEN_SCHEMA_INSERT.replace(
    b"content='chunks'", b"content=''chunks''"
).replace(b"content_rowid='id'", b"content_rowid=''id''")


class TestDumpRepairFilter(unittest.TestCase):
    """scripts/repair_dump.py (#1391): the streaming filter between the
    /dump download and the sqlite3 rebuild. Only a verifiably
    mis-escaped sqlite_schema literal is rewritten; everything else —
    including correctly-escaped statements — is byte-identical."""

    def run_filter(self, data: bytes) -> bytes:
        result = subprocess.run(
            [sys.executable, str(REPAIR)],
            input=data,
            capture_output=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_broken_literal_is_rewritten_with_doubled_quotes(self):
        data = (
            b"PRAGMA writable_schema=ON;\n"
            + BROKEN_SCHEMA_INSERT
            + b"PRAGMA writable_schema=OFF;\n"
        )
        expected = (
            b"PRAGMA writable_schema=ON;\n"
            + ESCAPED_SCHEMA_INSERT
            + b"PRAGMA writable_schema=OFF;\n"
        )
        self.assertEqual(self.run_filter(data), expected)

    def test_correctly_escaped_literal_passes_byte_identical(self):
        data = (
            b"PRAGMA writable_schema=ON;\n"
            + ESCAPED_SCHEMA_INSERT
            + b"PRAGMA writable_schema=OFF;\n"
        )
        self.assertEqual(self.run_filter(data), data)

    def test_quoteless_schema_literal_passes_byte_identical(self):
        data = (
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','t','t',0,'CREATE TABLE t (x INTEGER)');\n"
        )
        self.assertEqual(self.run_filter(data), data)

    def test_unrelated_lines_pass_byte_identical(self):
        data = (
            b"PRAGMA foreign_keys=OFF;\n"
            b"BEGIN TRANSACTION;\n"
            b"CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);\n"
            b"INSERT INTO notes VALUES(1,'it''s a ''quoted'' note');\n"
            b"COMMIT;\n"
        )
        self.assertEqual(self.run_filter(data), data)

    def test_unterminated_statement_at_eof_flushes_unchanged(self):
        # A truncated dump must not be mangled on its way to the
        # caller's COMMIT; guard — it passes through exactly as cut.
        data = b"BEGIN TRANSACTION;\n" + BROKEN_SCHEMA_INSERT[:-20]
        self.assertEqual(self.run_filter(data), data)


if __name__ == "__main__":
    unittest.main()
