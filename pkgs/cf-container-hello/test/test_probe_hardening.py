#!/usr/bin/env python3
"""Regression tests for #955 — cf-container-hello probe hardening.

Before #955 this package carried a stale, pre-hardening copy of the
crawler (`image/app.py`) whose POST /crawl urlopen()ed any
caller-supplied URL — `file://` included — and the Worker forwarded
/probe to it with no authentication at all.

The fix:
- the local crawler copy is deleted; CrawlerProbe runs the production
  image (`lab-ops-hub-crawlercontainer:v5`, built from
  pkgs/lab-ops-hub/container/app.py), and
- every container-forwarding Worker route sits behind a fail-closed
  API_TOKEN bearer gate (503 unconfigured / 401 unauthenticated).

These tests pin both halves: the production validator the probe now
runs refuses the SSRF target classes, and this package contains no
stale copy and no ungated forward.

Run: npm test  (python3 -m unittest discover -s test -p "test_*.py")
"""

import importlib.util
import json
import re
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
REPO = PKG.parent.parent
PROD_APP = REPO / "pkgs" / "lab-ops-hub" / "container" / "app.py"


def load_prod_app():
    spec = importlib.util.spec_from_file_location("lab_ops_hub_app", PROD_APP)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class ProductionValidatorRefusesSsrfTargets(unittest.TestCase):
    """The crawler the probe now runs must refuse the #955 targets."""

    @classmethod
    def setUpClass(cls):
        cls.app = load_prod_app()

    def assertRefused(self, url):
        with self.assertRaises(ValueError, msg=f"not refused: {url}"):
            self.app.validate_crawl_url(url)

    def test_file_scheme_refused(self):
        self.assertRefused("file:///etc/hostname")
        self.assertRefused("file:///etc/passwd")

    def test_non_http_schemes_refused(self):
        self.assertRefused("ftp://example.com/file")
        self.assertRefused("gopher://example.com/")

    def test_loopback_refused(self):
        self.assertRefused("http://127.0.0.1:8080/")
        self.assertRefused("http://localhost:8080/crawl")
        self.assertRefused("http://[::1]/")

    def test_private_ip_literals_refused(self):
        self.assertRefused("http://10.0.0.5/")
        self.assertRefused("http://172.16.0.9/")
        self.assertRefused("http://192.168.1.1/admin")
        self.assertRefused("http://169.254.169.254/latest/meta-data/")

    def test_redirect_to_private_target_refused(self):
        # Redirect targets are re-validated with the same function (see
        # test_redirects_revalidated below); the target a public URL
        # would bounce to must itself be refused.
        self.assertRefused("http://127.0.0.1/internal")
        self.assertRefused("http://10.1.2.3/internal")

    def test_public_https_accepted(self):
        out = self.app.validate_crawl_url("https://example.com/")
        self.assertTrue(out.startswith("https://example.com"))

    def test_redirects_revalidated_in_source(self):
        src = PROD_APP.read_text()
        # The redirect handler must pass each redirect target back
        # through validate_crawl_url before fetching it.
        self.assertIn("validate_crawl_url(newurl)", src)


class PackageHasNoStaleCopyAndIsGated(unittest.TestCase):
    def test_no_local_crawler_copy(self):
        self.assertFalse((PKG / "image" / "app.py").exists())
        self.assertFalse((PKG / "image").exists())

    def test_wrangler_points_at_hardened_production_image(self):
        wrangler = (PKG / "wrangler.jsonc").read_text()
        self.assertIn("lab-ops-hub-crawlercontainer:v5", wrangler)

    def test_worker_gate_is_fail_closed_and_before_forwards(self):
        src = (PKG / "src" / "index.ts").read_text()
        gate = src.index("isAuthorized(request, env)")
        self.assertGreater(gate, -1)
        # 503 when the secret is unset — fail closed, not open.
        self.assertIn("if (!env.API_TOKEN)", src)
        self.assertIn("503", src)
        # Every container forward happens after the gate.
        for marker in ("CRAWLER_PROBE", "HELLO_CONTAINER", "HELLO_LITE"):
            fwd = src.index(f"env.{marker}")
            self.assertGreater(fwd, gate, f"{marker} forward precedes gate")
        # /worker-marker stays public but touches no container.
        marker_at = src.index('"/worker-marker"')
        self.assertLess(marker_at, gate)

    def test_probe_route_still_exists_behind_gate(self):
        src = (PKG / "src" / "index.ts").read_text()
        self.assertIn('"/probe"', src)


if __name__ == "__main__":
    unittest.main()
