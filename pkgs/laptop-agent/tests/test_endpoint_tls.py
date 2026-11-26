#!/usr/bin/env python3
"""Tests for the daemon endpoint's Broker TLS Phase 1 (BT-4, #14402).

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests
Needs the openssl CLI to mint a synthetic CA and `broker-server` leaf;
the tests skip without it.
"""

import json
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"
OPENSSL = shutil.which("openssl")
TOKEN = "t" * 32


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _openssl(*args, cwd):
    subprocess.run(
        [OPENSSL, *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        timeout=60,
    )


def mint_pki(d):
    """A synthetic CA, a `broker-server` leaf for 127.0.0.1 signed by
    it, and an unrelated key that does not match the leaf."""
    _openssl(
        "req",
        "-x509",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-nodes",
        "-keyout",
        "ca.key",
        "-out",
        "ca.pem",
        "-days",
        "2",
        "-subj",
        "/CN=test fleet CA",
        "-addext",
        "basicConstraints=critical,CA:TRUE",
        "-addext",
        "keyUsage=critical,keyCertSign,cRLSign",
        cwd=d,
    )
    _openssl(
        "req",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-nodes",
        "-keyout",
        "leaf.key",
        "-out",
        "leaf.csr",
        "-subj",
        "/CN=broker-server",
        cwd=d,
    )
    Path(d, "leaf.ext").write_text(
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature\n"
        "extendedKeyUsage=serverAuth\n"
        "subjectAltName=IP:127.0.0.1,DNS:localhost\n"
    )
    _openssl(
        "x509",
        "-req",
        "-in",
        "leaf.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-out",
        "leaf.pem",
        "-days",
        "2",
        "-extfile",
        "leaf.ext",
        cwd=d,
    )
    _openssl(
        "genpkey",
        "-algorithm",
        "EC",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-out",
        "other.key",
        cwd=d,
    )
    return {
        "ca": str(Path(d, "ca.pem")),
        "cert": str(Path(d, "leaf.pem")),
        "key": str(Path(d, "leaf.key")),
        "other_key": str(Path(d, "other.key")),
    }


def agent_env(token_path, port, **extra):
    env = {k: v for k, v in os.environ.items() if not k.startswith("LAPTOP_AGENT_TLS_")}
    env.update(
        LAPTOP_AGENT_TOKEN_FILE=token_path,
        LAPTOP_AGENT_HOST="127.0.0.1",
        LAPTOP_AGENT_PORT=str(port),
        LAPTOP_AGENT_STATE=tempfile.mkdtemp(),
    )
    env.update(extra)
    return env


@unittest.skipUnless(OPENSSL, "openssl CLI not on PATH")
class EndpointTlsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp()
        cls.pki = mint_pki(cls.dir)
        cls.token_path = str(Path(cls.dir, "token"))
        Path(cls.token_path).write_text(TOKEN + "\n")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def _run(self, **extra):
        return subprocess.run(
            [sys.executable, str(AGENT)],
            env=agent_env(self.token_path, free_port(), **extra),
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )

    def _start(self):
        port = free_port()
        proc = subprocess.Popen(
            [sys.executable, str(AGENT)],
            env=agent_env(
                self.token_path,
                port,
                LAPTOP_AGENT_TLS_CERT_FILE=self.pki["cert"],
                LAPTOP_AGENT_TLS_KEY_FILE=self.pki["key"],
            ),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.addCleanup(self._stop, proc)
        ctx = self._client_ctx()
        deadline = time.time() + 15
        while time.time() < deadline:
            if proc.poll() is not None:
                self.fail(f"agent exited {proc.returncode}: {proc.stdout.read()}")
            try:
                self._get(port, "/health", ctx=ctx, token=TOKEN)
                return proc, port
            except (urllib.error.URLError, ConnectionError, ssl.SSLError):
                time.sleep(0.1)
        self.fail("agent did not start")

    @staticmethod
    def _stop(proc):
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=10)

    def _output(self, proc):
        self._stop(proc)
        return proc.stdout.read()

    def _client_ctx(self, ca=None):
        ctx = ssl.create_default_context(cafile=ca or self.pki["ca"])
        ctx.minimum_version = ssl.TLSVersion.TLSv1_2
        return ctx

    def _get(self, port, path, ctx=None, token=None):
        scheme = "https" if ctx is not None else "http"
        headers = {"X-Agent-Token": token} if token is not None else {}
        req = urllib.request.Request(
            f"{scheme}://127.0.0.1:{port}{path}", headers=headers
        )
        with urllib.request.urlopen(req, timeout=5, context=ctx) as r:
            return r.status, json.loads(r.read())

    def test_tls_request_validates_against_the_ca_and_keeps_token_auth(self):
        _, port = self._start()
        status, body = self._get(port, "/health", ctx=self._client_ctx(), token=TOKEN)
        self.assertEqual(status, 200)
        self.assertTrue(body["ok"])
        with self.assertRaises(urllib.error.HTTPError) as cm:
            self._get(port, "/health", ctx=self._client_ctx())
        self.assertEqual(cm.exception.code, 401)
        with self.assertRaises(urllib.error.HTTPError) as cm:
            self._get(port, "/health", ctx=self._client_ctx(), token="wrong" * 8)
        self.assertEqual(cm.exception.code, 401)

    def test_post_with_a_body_round_trips_over_tls(self):
        _, port = self._start()
        req = urllib.request.Request(
            f"https://127.0.0.1:{port}/exec",
            data=json.dumps(
                {
                    "cmds": [
                        {
                            "cwd": self.dir,
                            "argv": [sys.executable, "-c", "print('over tls')"],
                        }
                    ]
                }
            ).encode(),
            headers={"X-Agent-Token": TOKEN, "Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=30, context=self._client_ctx()) as r:
            self.assertEqual(r.status, 200)
            body = json.loads(r.read())
        self.assertIn("over tls", json.dumps(body))

    def test_plaintext_phase0_is_still_accepted_and_logged(self):
        proc, port = self._start()
        status, body = self._get(port, "/health", token=TOKEN)
        self.assertEqual(status, 200)
        self.assertTrue(body["ok"])
        out = self._output(proc)
        self.assertIn("(TLS (dual-accept, Phase 1))", out)
        self.assertIn("plaintext Phase 0 accept from 127.0.0.1", out)

    def test_a_client_trusting_another_ca_is_refused(self):
        _, port = self._start()
        other = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, other, True)
        other_pki = mint_pki(other)
        with self.assertRaises(urllib.error.URLError) as cm:
            self._get(
                port, "/health", ctx=self._client_ctx(other_pki["ca"]), token=TOKEN
            )
        self.assertIsInstance(cm.exception.reason, ssl.SSLCertVerificationError)

    def test_a_bad_handshake_is_dropped_and_the_daemon_keeps_serving(self):
        proc, port = self._start()
        with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
            s.sendall(b"\x16\x03\x01\x00\x05garbage")
            s.settimeout(5)
            try:
                rest = s.recv(4096)
            except (ConnectionError, TimeoutError):
                rest = b""
            self.assertNotIn(b"HTTP/", rest)
        status, _ = self._get(port, "/health", ctx=self._client_ctx(), token=TOKEN)
        self.assertEqual(status, 200)
        self.assertIn("TLS handshake from 127.0.0.1 failed", self._output(proc))

    def test_cert_without_key_refuses_to_start(self):
        p = self._run(LAPTOP_AGENT_TLS_CERT_FILE=self.pki["cert"])
        self.assertEqual(p.returncode, 3, p.stderr)
        self.assertIn("LAPTOP_AGENT_TLS_KEY_FILE is not", p.stderr)

    def test_key_without_cert_refuses_to_start(self):
        p = self._run(LAPTOP_AGENT_TLS_KEY_FILE=self.pki["key"])
        self.assertEqual(p.returncode, 3, p.stderr)
        self.assertIn("LAPTOP_AGENT_TLS_CERT_FILE is not", p.stderr)

    def test_mismatched_key_refuses_to_start(self):
        p = self._run(
            LAPTOP_AGENT_TLS_CERT_FILE=self.pki["cert"],
            LAPTOP_AGENT_TLS_KEY_FILE=self.pki["other_key"],
        )
        self.assertEqual(p.returncode, 3, p.stderr)
        self.assertIn("is unusable", p.stderr)

    def test_missing_cert_file_refuses_to_start(self):
        p = self._run(
            LAPTOP_AGENT_TLS_CERT_FILE=str(Path(self.dir, "absent.pem")),
            LAPTOP_AGENT_TLS_KEY_FILE=self.pki["key"],
        )
        self.assertEqual(p.returncode, 3, p.stderr)
        self.assertIn("LAPTOP_AGENT_TLS_CERT_FILE=", p.stderr)

    def test_unset_tls_serves_plaintext_as_before(self):
        port = free_port()
        proc = subprocess.Popen(
            [sys.executable, str(AGENT)],
            env=agent_env(self.token_path, port),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.addCleanup(self._stop, proc)
        deadline = time.time() + 15
        while True:
            try:
                status, _ = self._get(port, "/health", token=TOKEN)
                break
            except (urllib.error.URLError, ConnectionError):
                if time.time() > deadline:
                    self.fail("agent did not start")
                time.sleep(0.1)
        self.assertEqual(status, 200)
        out = self._output(proc)
        self.assertIn("(plaintext)", out)
        self.assertNotIn("Phase 0 accept", out)


if __name__ == "__main__":
    unittest.main()
