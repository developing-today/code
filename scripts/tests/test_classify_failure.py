"""Contract tests for scripts/classify_failure.py (#2176 scout leg).

Each classification gets at least one fixture tail; rule ORDER is part
of the contract (quota/runner/network signals must beat a generic
failing-test summary printed by the same job).
"""

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "classify_failure.py"
spec = importlib.util.spec_from_file_location("classify_failure", SCRIPT)
cf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cf)


def classify(text, job="basic: scripts-test (3.12)"):
    return cf.classify({job: text})


class QuotaOrSecretTests(unittest.TestCase):
    def test_bad_credentials(self):
        r = classify("Run gh api ...\nHttpError: Bad credentials\n")
        self.assertEqual(r["classification"], "quota-or-secret")
        self.assertEqual(r["route"], "escalate")
        self.assertTrue(r["evidence"])

    def test_missing_secret_names_detail(self):
        r = classify("Error: The secret JULES_API_KEY was not set in this repo\n")
        self.assertEqual(r["classification"], "quota-or-secret")
        self.assertIn("JULES_API_KEY", r["detail"])

    def test_rate_limit(self):
        r = classify("API rate limit exceeded for installation\n")
        self.assertEqual(r["classification"], "quota-or-secret")

    def test_quota_beats_test_failure_summary(self):
        r = classify("--- FAIL: TestThing\nBad credentials\n")
        self.assertEqual(r["classification"], "quota-or-secret")


class InfraRunnerTests(unittest.TestCase):
    def test_runner_shutdown(self):
        r = classify("The runner has received a shutdown signal.\n")
        self.assertEqual(r["classification"], "infra-runner")
        self.assertEqual(r["route"], "rerun")

    def test_no_space_left(self):
        r = classify("write /tmp/x: no space left on device\n")
        self.assertEqual(r["classification"], "infra-runner")

    def test_runner_beats_regression(self):
        r = classify("test result: FAILED\nThe runner has received a shutdown signal\n")
        self.assertEqual(r["classification"], "infra-runner")


class InfraNetworkTests(unittest.TestCase):
    def test_dns_failure(self):
        r = classify("curl: (6) Could not resolve host: registry.npmjs.org\n")
        self.assertEqual(r["classification"], "infra-network")
        self.assertEqual(r["route"], "rerun")

    def test_registry_502(self):
        r = classify("error sending request for url (https://crates.io): 502 Bad Gateway\n")
        self.assertEqual(r["classification"], "infra-network")


class FlakeSuspectTests(unittest.TestCase):
    def test_test_timeout(self):
        r = classify("panic: test timed out after 10m0s\n")
        self.assertEqual(r["classification"], "flake-suspect")
        self.assertEqual(r["route"], "rerun")

    def test_nix_flake_check_is_not_flake_suspect(self):
        # The bare word "flake" in "nix flake check" output is Nix
        # terminology, not an intermittency signal (Codex, PR #7806).
        r = classify("+ nix flake check ./pkgs/id/ -L --keep-going\nerror[E0308]: mismatched types\n")
        self.assertEqual(r["classification"], "real-regression")

    def test_nix_flake_check_opaque_failure_is_unknown(self):
        r = classify("+ nix flake check ./pkgs/id/\nsome opaque failure\n")
        self.assertEqual(r["classification"], "unknown")

    def test_flake_beats_plain_failure_word(self):
        r = classify("FAILURES:\nflaky: TestRetries timed out waiting for server\n")
        self.assertEqual(r["classification"], "flake-suspect")


class RealRegressionTests(unittest.TestCase):
    def test_go_test_failure(self):
        r = classify("=== RUN TestParse\n--- FAIL: TestParse (0.01s)\nFAIL\n")
        self.assertEqual(r["classification"], "real-regression")
        self.assertEqual(r["route"], "fix")

    def test_rust_test_result(self):
        r = classify("test result: FAILED. 3 passed; 1 failed\n")
        self.assertEqual(r["classification"], "real-regression")

    def test_compile_error(self):
        r = classify("error[E0308]: mismatched types\n")
        self.assertEqual(r["classification"], "real-regression")


class UnknownAndShapeTests(unittest.TestCase):
    def test_unknown_fallback(self):
        r = classify("some opaque failure\nsecond line\n")
        self.assertEqual(r["classification"], "unknown")
        self.assertEqual(r["route"], "fix")
        self.assertTrue(r["evidence"])  # last lines still carried

    def test_evidence_capped_at_five(self):
        logs = {f"job{i}": "Bad credentials\n" for i in range(9)}
        r = cf.classify(logs)
        self.assertLessEqual(len(r["evidence"]), 5)

    def test_tail_only(self):
        # A quota signal older than the tail window must not classify.
        text = "Bad credentials\n" + ("noise line\n" * 500)
        r = cf.classify({"job": text}, tail=50)
        self.assertEqual(r["classification"], "unknown")

    def test_empty_log_is_unknown(self):
        r = classify("")
        self.assertEqual(r["classification"], "unknown")
        self.assertEqual(r["evidence"], [])


if __name__ == "__main__":
    unittest.main()
