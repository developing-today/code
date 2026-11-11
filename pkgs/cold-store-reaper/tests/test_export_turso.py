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

    def test_dump_max_time_zero_is_rejected(self):
        # 0 is all digits but not positive: curl --max-time 0 disables
        # the timeout, so the dump could hang forever (#1428).
        env = {**self.env, "TURSO_DUMP_MAX_TIME_SECONDS": "0"}
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("must be a positive integer", result.stderr)
        self.assertEqual(list(self.capture.glob("argv-*.log")), [])

    def test_dump_max_time_all_zeros_is_rejected(self):
        env = {**self.env, "TURSO_DUMP_MAX_TIME_SECONDS": "00"}
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("must be a positive integer", result.stderr)
        self.assertEqual(list(self.capture.glob("argv-*.log")), [])

    def test_dump_max_time_leading_zero_positive_is_honored(self):
        env = {**self.env, "TURSO_DUMP_MAX_TIME_SECONDS": "0300"}
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.dump_argv_lines()
        self.assertEqual(lines[lines.index("--max-time") + 1], "0300")

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

    def test_interior_line_ending_in_quote_paren_semicolon_is_not_a_boundary(self):
        # #6531: a multi-line schema statement whose embedded SQL has an
        # interior line ending in "');" (a trigger body's RAISE) is one
        # statement, not a repaired fragment plus a raw tail. Oracle:
        # the filtered stream loads in stock sqlite3 with the full SQL.
        import sqlite3

        sql = (
            "CREATE TRIGGER tr AFTER INSERT ON ft BEGIN SELECT RAISE(ABORT, 'bad');\n"
            " INSERT INTO log VALUES('x'); END"
        )
        raw = (
            "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql) "
            "VALUES('trigger','tr','ft',0,'" + sql + "');\n"
        ).encode()
        escaped = raw.replace(b"'bad'", b"''bad''").replace(b"VALUES('x')", b"VALUES(''x'')")
        for variant in (raw, escaped):
            data = (
                b"PRAGMA writable_schema=ON;\n"
                + variant
                + b"PRAGMA writable_schema=OFF;\n"
            )
            out = self.run_filter(data)
            # Mis-escaped input is repaired (quotes doubled); escaped
            # input passes through byte-identical.
            self.assertEqual(out, data.replace(raw, escaped) if variant is raw else data)
            db = sqlite3.connect(":memory:")
            db.executescript(
                "CREATE TABLE ft(x); CREATE TABLE log(v);"
                "PRAGMA writable_schema=ON;"
                "DELETE FROM sqlite_schema WHERE name='tr';"
            )
            db.executescript(out.decode())
            self.assertEqual(
                db.execute("SELECT sql FROM sqlite_schema WHERE name='tr'").fetchone()[0],
                sql,
            )
            db.close()

    def test_multiline_statement_followed_by_more_statements(self):
        data = (
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','a','a',0,'CREATE TABLE a (x); -- ''c'');\n"
            b"');\n"
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','t','t',0,'CREATE TABLE t (x INTEGER)');\n"
        )
        self.assertEqual(self.run_filter(data), data)

    def run_filter_capped(self, data: bytes, cap: int) -> bytes:
        env = dict(os.environ, REPAIR_DUMP_MAX_CANDIDATE_BYTES=str(cap))
        result = subprocess.run(
            [sys.executable, str(REPAIR)],
            input=data,
            capture_output=True,
            timeout=60,
            env=env,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_over_cap_candidate_streams_through_and_following_statement_is_repaired(self):
        # #6525: a candidate that grows past the buffer cap without
        # terminating is flushed unchanged and streamed, not held;
        # its end is still recognised, so the next statement gets
        # the normal repair treatment.
        opener = (
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','big','big',0,'CREATE TABLE big (\n"
        )
        filler = (b"x" * 100 + b"\n") * 100  # 10 KB, no terminator line
        data = opener + filler + b"');\n" + BROKEN_SCHEMA_INSERT
        expected = opener + filler + b"');\n" + ESCAPED_SCHEMA_INSERT
        self.assertEqual(self.run_filter_capped(data, 4096), expected)

    def test_cap_crossed_on_terminating_line_does_not_leak_passthrough(self):
        # Codex P2 on PR #8415: when the cap is first exceeded ON the
        # candidate's terminating line, the statement is complete --
        # it flushes whole (unrepaired, like any over-cap statement)
        # and must NOT enable pass-through, or the following statement
        # streams through unrepaired even when it is below the cap
        # and needs quote repair.
        opener = (
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','big','big',0,'CREATE TABLE big (\n"
        )
        filler = (b"x" * 100 + b"\n") * 39 + b"x" * 46 + b"\n"
        # opener + filler stays under the 4096 cap; the terminating
        # line itself is what pushes the buffer over it.
        self.assertLessEqual(len(opener) + len(filler), 4096)
        self.assertGreater(len(opener) + len(filler) + len(b"');\n"), 4096)
        data = opener + filler + b"');\n" + BROKEN_SCHEMA_INSERT
        expected = opener + filler + b"');\n" + ESCAPED_SCHEMA_INSERT
        self.assertEqual(self.run_filter_capped(data, 4096), expected)

    def test_over_cap_candidate_emits_before_eof(self):
        # #6525, the streaming half of the oracle: with stdin held
        # open, output from an over-cap candidate must already be
        # flowing (the old code emitted nothing until a terminator
        # or EOF, buffering the whole remainder of the dump).
        import select

        opener = (
            b"INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
            b" VALUES('table','big','big',0,'CREATE TABLE big (\n"
        )
        filler = (b"y" * 100 + b"\n") * 100
        env = dict(os.environ, REPAIR_DUMP_MAX_CANDIDATE_BYTES="4096")
        proc = subprocess.Popen(
            [sys.executable, str(REPAIR)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            env=env,
        )
        try:
            proc.stdin.write(opener + filler)
            proc.stdin.flush()
            ready, _, _ = select.select([proc.stdout], [], [], 15)
            self.assertTrue(ready, "no output before EOF: the candidate is being buffered whole")
            first = os.read(proc.stdout.fileno(), 4096)
            self.assertTrue(first.startswith(b"INSERT INTO sqlite_schema("))
            proc.stdin.write(b"');\nPRAGMA writable_schema=OFF;\n")
            proc.stdin.close()
            rest = proc.stdout.read()
            self.assertEqual(proc.wait(timeout=60), 0)
            self.assertEqual(first + rest, opener + filler + b"');\nPRAGMA writable_schema=OFF;\n")
        finally:
            if proc.poll() is None:
                proc.kill()


if __name__ == "__main__":
    unittest.main()
