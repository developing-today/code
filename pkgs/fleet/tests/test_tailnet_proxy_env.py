#!/usr/bin/env python3
"""Tests for laptop-agent.py's recorded tailnet proxy resolution
(code#12426 — the Python mirror of fleet-config's #12645 rule: the
seat's recorded fleet-proxy.env beats the ambient HTTPS_PROXY for
tailnet CONNECTs).

Run with: python3 -m unittest discover -s pkgs/fleet/tests
"""

import importlib.util
import os
import socket
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

CLIENT = Path(__file__).resolve().parents[1] / "laptop-agent.py"


def _load_client():
    spec = importlib.util.spec_from_file_location("laptop_agent_client", CLIENT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


la = _load_client()


class AgentSlugTest(unittest.TestCase):
    """Same cases as fleet-config's agent_slug_matches_the_bootstrap_rule."""

    def test_cases(self):
        self.assertEqual(la._agent_slug("felix"), "felix")
        self.assertEqual(la._agent_slug("Muse Felix"), "felix")
        self.assertEqual(la._agent_slug("muse-archer"), "archer")
        self.assertEqual(la._agent_slug("-a_b-"), "a-b")
        self.assertEqual(la._agent_slug("muse-"), "muse")
        self.assertIsNone(la._agent_slug(""))
        self.assertEqual(len(la._agent_slug("x" * 80)), 55)


class CandidatesTest(unittest.TestCase):
    """Same order as fleet-config's
    tailnet_proxy_env_candidates_follow_the_bootstrap_layout."""

    def test_order(self):
        home = Path("/home/a")
        state = home / ".local/state/tailscale"
        self.assertEqual(
            la._tailnet_proxy_env_candidates(home, None, None),
            [state / "fleet-proxy.env"],
        )
        self.assertEqual(
            la._tailnet_proxy_env_candidates(home, "Muse-Felix", "x/p.env"),
            [
                home / "x/p.env",
                state / "agents/felix/fleet-proxy.env",
                state / "fleet-proxy.env",
            ],
        )
        # A name with nothing usable gets no agent path.
        self.assertEqual(
            la._tailnet_proxy_env_candidates(home, "--", None),
            [state / "fleet-proxy.env"],
        )


class ParseTest(unittest.TestCase):
    """Same cases as fleet-config's
    parse_tailnet_proxy_env_reads_the_bootstrap_line."""

    def test_cases(self):
        self.assertEqual(
            la._parse_tailnet_proxy_env("export HTTPS_PROXY=http://127.0.0.1:3130\n"),
            "http://127.0.0.1:3130",
        )
        self.assertEqual(
            la._parse_tailnet_proxy_env(
                "# c\nHTTPS_PROXY='http://a:1'\nexport HTTPS_PROXY=\"http://b:2\"\n"
            ),
            "http://b:2",
        )
        self.assertIsNone(la._parse_tailnet_proxy_env("HTTPS_PROXY=\n"))
        self.assertIsNone(la._parse_tailnet_proxy_env("OTHER=1\n"))


class ResolveTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name)
        self.state = self.home / ".local/state/tailscale"
        self.state.mkdir(parents=True)
        env = {"HOME": str(self.home)}
        self.env_patch = patch.dict(os.environ, env, clear=False)
        self.env_patch.start()
        for key in ("FLEET_AGENT", "FLEET_PROXY_ENV"):
            os.environ.pop(key, None)

    def tearDown(self):
        self.env_patch.stop()
        self.tmp.cleanup()

    def _write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_no_file_leaves_ambient_in_charge(self):
        self.assertIsNone(la.tailnet_https_proxy())

    def test_shared_file(self):
        self._write(
            self.state / "fleet-proxy.env",
            "export HTTPS_PROXY=http://127.0.0.1:3130\n",
        )
        self.assertEqual(la.tailnet_https_proxy(), "http://127.0.0.1:3130")

    def test_agent_file_beats_shared(self):
        self._write(
            self.state / "fleet-proxy.env",
            "export HTTPS_PROXY=http://127.0.0.1:3130\n",
        )
        self._write(
            self.state / "agents/felix/fleet-proxy.env",
            "export HTTPS_PROXY=http://127.0.0.1:9999\n",
        )
        with patch.dict(os.environ, {"FLEET_AGENT": "Muse Felix"}):
            self.assertEqual(la.tailnet_https_proxy(), "http://127.0.0.1:9999")

    def test_explicit_path_beats_both(self):
        self._write(
            self.state / "fleet-proxy.env",
            "export HTTPS_PROXY=http://127.0.0.1:3130\n",
        )
        explicit = self.home / "elsewhere.env"
        self._write(explicit, "export HTTPS_PROXY=http://127.0.0.1:7777\n")
        with patch.dict(os.environ, {"FLEET_PROXY_ENV": str(explicit)}):
            self.assertEqual(la.tailnet_https_proxy(), "http://127.0.0.1:7777")


class _FakeSocket:
    def __init__(self):
        self._resp = b"HTTP/1.1 200 OK\r\n\r\n"
        self.sent = b""

    def sendall(self, data):
        self.sent += data

    def recv(self, _n):
        if not self._resp:
            return b""
        out, self._resp = self._resp[:1], self._resp[1:]
        return out

    def settimeout(self, _t):
        pass


class TunnelSocketPreferenceTest(unittest.TestCase):
    """The tailnet CONNECT goes to the recorded proxy when one exists,
    and to the ambient proxy's host otherwise."""

    def _run(self, env):
        fake = _FakeSocket()
        calls = []

        def fake_create_connection(addr, timeout=None):
            calls.append(addr)
            return fake

        with patch.dict(os.environ, env, clear=False):
            for key in ("FLEET_AGENT", "FLEET_PROXY_ENV"):
                if key not in env:
                    os.environ.pop(key, None)
            with patch.object(socket, "create_connection", fake_create_connection):
                la.tunnel_socket()
        return calls

    def test_recorded_proxy_wins_over_ambient_egress(self):
        with tempfile.TemporaryDirectory() as td:
            home = Path(td)
            env_file = home / "fleet-proxy.env"
            env_file.write_text("export HTTPS_PROXY=http://127.0.0.1:3130\n")
            calls = self._run(
                {
                    "HOME": str(home),
                    "FLEET_PROXY_ENV": str(env_file),
                    "HTTPS_PROXY": "http://broker:broker@hatch-egress-proxy:3128",
                }
            )
        self.assertEqual(calls, [("127.0.0.1", 3130)])

    def test_ambient_used_when_no_recorded_proxy(self):
        with tempfile.TemporaryDirectory() as td:
            calls = self._run(
                {
                    "HOME": str(Path(td)),
                    "HTTPS_PROXY": "http://broker:broker@127.0.0.1:3130",
                }
            )
        self.assertEqual(calls, [("127.0.0.1", 3130)])


if __name__ == "__main__":
    unittest.main()
