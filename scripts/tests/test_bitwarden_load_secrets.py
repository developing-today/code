#!/usr/bin/env python3
"""Contract tests for lib/bitwarden-load-secrets.sh multiline values (#5650).

A multiline secret value (private key, certificate) must survive
end-to-end in every output mode: the loaders, the PAIRS store, and the
output dispatchers must never split it into bogus variables.
Oracle: the exact stubbed values below, compared byte-for-byte.
"""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "lib" / "bitwarden-load-secrets.sh"

MULTILINE = "BEGIN CERT\nPRIVATE KEY\nEND CERT"
CONTROL = "hello"

STUB = """#!/usr/bin/env python3
import json, sys
name = sys.argv[0].rsplit("/", 1)[-1]
args = sys.argv[1:]
if name == "bws":
    if args[:2] == ["secret", "list"]:
        print(json.dumps([
            {"key": "TLS_CERT", "value": "BEGIN CERT\\nPRIVATE KEY\\nEND CERT"},
            {"key": "PLAIN", "value": "hello"},
        ]))
    elif args[:2] == ["secret", "get"]:
        print(json.dumps({"key": "TLS_CERT",
                          "value": "BEGIN CERT\\nPRIVATE KEY\\nEND CERT"}))
    else:
        sys.exit(1)
elif name == "bw":
    if args[:1] == ["status"]:
        print(json.dumps({"status": "unlocked"}))
    elif args[:2] == ["get", "item"]:
        print(json.dumps({"name": "Item", "fields": [
            {"name": "TLS_CERT", "value": "BEGIN CERT\\nPRIVATE KEY\\nEND CERT"},
            {"name": "PLAIN", "value": "hello"},
        ]}))
    else:
        sys.exit(1)
"""


class TestBitwardenLoadSecrets(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="bw-load-")
        bindir = Path(self.tmp.name) / "bin"
        bindir.mkdir()
        for tool in ("bws", "bw"):
            stub = bindir / tool
            stub.write_text(STUB)
            stub.chmod(0o755)
        self.env = os.environ.copy()
        self.env["PATH"] = f"{bindir}:{self.env['PATH']}"
        self.env["BWS_ACCESS_TOKEN"] = "test-token"
        self.env["BW_SESSION"] = "test-session"

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, *args):
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            env=self.env, capture_output=True, text=True, check=True,
        ).stdout

    def eval_format(self, fmt, source_args):
        # Evaluate the script's export/env output (or source it) in a
        # fresh shell and print the resulting variables byte-exactly.
        if fmt == "source":
            cmd = (
                f'source "{SCRIPT}" {source_args} >/dev/null; '
                'printf "%s\\x00%s" "$TLS_CERT" "$PLAIN"'
            )
        else:
            cmd = (
                f'eval "$("{SCRIPT}" {source_args})"; '
                'printf "%s\\x00%s" "$TLS_CERT" "$PLAIN"'
            )
        out = subprocess.run(
            ["bash", "-c", cmd], env=self.env,
            capture_output=True, text=True, check=True,
        ).stdout
        cert, plain = out.split("\x00")
        self.assertEqual(cert, MULTILINE)
        self.assertEqual(plain, CONTROL)

    def test_bws_list_json(self):
        data = json.loads(self.run_script("--bws", "--format", "json"))
        self.assertEqual(data, {"TLS_CERT": MULTILINE, "PLAIN": CONTROL})

    def test_bws_list_export_env_source(self):
        for fmt in ("export", "env"):
            with self.subTest(fmt=fmt):
                self.eval_format(fmt, f"--bws --format {fmt}")
        self.eval_format("source", "--bws")

    def test_bws_single_secret_json(self):
        data = json.loads(
            self.run_script("--bws", "--secret", "x", "--format", "json"))
        self.assertEqual(data, {"TLS_CERT": MULTILINE})

    def test_vault_fields_json_and_source(self):
        data = json.loads(
            self.run_script("--item", "Item", "--format", "json"))
        self.assertEqual(data, {"TLS_CERT": MULTILINE, "PLAIN": CONTROL})
        self.eval_format("source", "--item Item")


if __name__ == "__main__":
    unittest.main()
