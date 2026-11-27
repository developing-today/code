#!/usr/bin/env python3
"""apply.py against an independent model of the Cloudflare account.

Checked: the repository state is valid; a check run changes nothing
and reports each change it would make; a second apply changes
nothing; and refusing states fail before anything is asked of Cloudflare.
Tunnel ingress is tested separately by test_expose.py.
"""

import copy
import io
import json
import os
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
import apply as applier
import cloudflare
import test_model

Account = test_model.Account

HOST = "pm.security.cab"
OWNER = "owner@example.com"
BASE_ENV = {
    "PMCF_CLOUDFLARE_TOKEN": "token",
    "PMCF_OWNER_EMAIL": OWNER,
    "PMCF_GOOGLE_CLIENT_ID": "",
    "PMCF_GOOGLE_CLIENT_SECRET": "",
    "PMCF_GITHUB_TOKEN": "github-value",
    "PMCF_QUEUE_TOKEN": "queue-value",
    "PMCF_BRIDGE_TOKEN": "bridge-value",
}


def run(account, *args, env=None, state=None):
    """One apply.py run. What it printed and the changes it recorded
    are left on the account."""
    argv = ["apply.py", *args]
    if state is not None:
        argv += ["--state", str(state)]
    full_env = dict(BASE_ENV)
    full_env.update(env or {})

    def health(url):
        if account.health_misses > 0:
            account.health_misses -= 1
            return None
        if (
            not account.worker_up
            or url != f"https://{cloudflare.WORKER}.acct.workers.dev/console/health"
        ):
            return None
        return {"ok": True, "edge": True, "host": account.worker_host}

    with (
        mock.patch.object(applier.expose, "Cloudflare", lambda token, check: account),
        mock.patch.object(applier.expose, "fetch_json", health),
        mock.patch.object(applier.expose.time, "sleep", lambda seconds: None),
        mock.patch.dict(os.environ, full_env, clear=False),
        mock.patch("builtins.print") as printed,
    ):
        account.check = "--check" in args
        account.changes = []
        try:
            code = applier.main(argv[1:])
        finally:
            account.printed = [str(c.args[0]) for c in printed.call_args_list]
        return code


def write_state(changes):
    """A state file that is the checked-in one with `changes` applied."""
    state = json.loads((HERE / "state.json").read_text())
    for keys, value in changes.items():
        node = state
        for key in keys[:-1]:
            node = node[key]
        node[keys[-1]] = value
    path = pathlib.Path(tempfile.mkdtemp()) / "state.json"
    path.write_text(json.dumps(state))
    return path


class Apply(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp()

    def test_the_state_is_valid_and_applies_cleanly_twice(self):
        account = Account()
        code = run(account, "--check")
        self.assertEqual(code, 1)
        self.assertTrue(account.changes, "an empty account would change")
        code = run(account, "--apply")
        self.assertEqual(code, 0)
        second = Account()
        second.apps = copy.deepcopy(account.apps)
        second.policies = copy.deepcopy(account.policies)
        second.ingress = copy.deepcopy(account.ingress)
        second.dns = copy.deepcopy(account.dns)
        second.routes = copy.deepcopy(account.routes)
        second.worker_secrets = copy.deepcopy(account.worker_secrets)
        second.idps = copy.deepcopy(account.idps)
        code = run(second, "--apply")
        self.assertEqual(code, 0)
        self.assertEqual(
            second.changes,
            [],
            f"a second apply changes nothing, changed: {second.changes}",
        )

    def test_check_reports_and_changes_nothing(self):
        account = Account()
        before = (copy.deepcopy(account.apps), copy.deepcopy(account.ingress))
        code = run(account, "--check")
        self.assertEqual(code, 1)
        self.assertTrue(
            any(line.startswith("would change: ") for line in account.printed)
        )
        self.assertEqual((account.apps, account.ingress), before)

    def test_an_up_to_date_account_changes_nothing_and_says_so(self):
        account = Account()
        run(account, "--apply")
        account.changes = []
        code = run(account, "--check")
        self.assertEqual(code, 0)
        self.assertTrue(any("is as declared" in line for line in account.printed))

    def test_apply_uses_workflow_owned_cloudflare_module(self):
        self.assertIs(applier.expose, cloudflare)
        self.assertNotEqual(
            pathlib.Path(applier.expose.__file__).resolve(),
            (HERE.parent.parent / "pm-stack" / "console" / "scripts" / "expose.py").resolve(),
        )

    def test_a_renamed_tunnel_is_refused_before_anything_is_asked(self):
        account = Account()
        path = write_state({("tunnel",): "other-tunnel"})
        with self.assertRaises(SystemExit):
            run(account, "--apply", state=path)
        self.assertEqual(account.order, [])

    def test_an_unknown_key_is_refused(self):
        account = Account()
        path = write_state({("worker", "deploys"): True})
        with self.assertRaises(SystemExit):
            run(account, "--apply", state=path)
        self.assertEqual(account.order, [])

    def test_a_missing_secret_fails_before_anything_is_asked(self):
        account = Account()
        with self.assertRaises(SystemExit):
            run(account, "--apply", env={"PMCF_GITHUB_TOKEN": ""})
        self.assertEqual(account.order, [])

    def test_missing_optional_google_pair_preserves_existing_provider(self):
        account = Account()
        google = {
            "id": "idp-google",
            "type": "google",
            "name": "Google",
            "config": {
                "client_id": "configured-id",
                "client_secret": "configured-secret",
            },
        }
        account.idps.append(copy.deepcopy(google))

        code = run(account, "--apply")

        self.assertEqual(code, 0)
        actual = next(idp for idp in account.idps if idp["type"] == "google")
        self.assertEqual(actual, google)
        self.assertNotIn(
            "PUT /accounts/acc/access/identity_providers/idp-google",
            account.order,
        )

    def test_partial_optional_google_pair_refuses_before_cloudflare_writes(self):
        account = Account()

        with self.assertRaises(SystemExit):
            run(account, "--apply", env={"PMCF_GOOGLE_CLIENT_ID": "configured-id"})

        self.assertEqual(account.order, [])

    def test_refresh_tokens_rotates_existing_worker_secrets(self):
        account = Account()
        run(account, "--apply")
        account.worker_secrets["GITHUB_TOKEN"] = "old-github"
        account.worker_secrets["QUEUE_TOKEN"] = "old-queue"
        account.worker_secrets["BRIDGE_TOKEN"] = "old-bridge"
        account.order = []
        code = run(account, "--apply", "--refresh-tokens")
        self.assertEqual(code, 0)
        self.assertEqual(
            account.worker_secrets,
            {
                "GITHUB_TOKEN": "github-value",
                "QUEUE_TOKEN": "queue-value",
                "EDGE_DIRECT": "on",
                "BRIDGE_TOKEN": "bridge-value",
            },
        )
        self.assertEqual(
            account.order,
            ["PUT /accounts/acc/workers/scripts/pm-console-edge/secrets"] * 4,
        )

    def test_refresh_tokens_is_refused_when_direct_mode_is_off(self):
        account = Account()
        state = write_state({("worker", "direct"): "off"})
        with self.assertRaisesRegex(SystemExit, "worker.direct to be on"):
            run(account, "--apply", "--refresh-tokens", state=state)
        self.assertEqual(getattr(account, "asked", 0), 0)

    def test_the_bridge_token_rides_along_when_the_state_holds_it(self):
        account = Account()
        run(account, "--apply")
        self.assertEqual(
            account.worker_secrets,
            {
                "GITHUB_TOKEN": "github-value",
                "QUEUE_TOKEN": "queue-value",
                "EDGE_DIRECT": "on",
                "BRIDGE_TOKEN": "bridge-value",
            },
        )
        # A state without it leaves a Worker without it alone, in silence.
        account2 = Account()
        run(account2, "--apply", env={"PMCF_BRIDGE_TOKEN": ""})
        self.assertNotIn("BRIDGE_TOKEN", account2.worker_secrets)
        self.assertNotIn("bridge", "\n".join(account2.changes))
        account = Account()
        run(account, "--apply")
        self.assertIn("GITHUB_TOKEN", account.worker_secrets)
        path = write_state({("worker", "direct"): "off"})
        code = run(account, "--apply", state=path)
        self.assertEqual(code, 0)
        self.assertNotIn("GITHUB_TOKEN", account.worker_secrets)
        self.assertNotIn("QUEUE_TOKEN", account.worker_secrets)
        self.assertNotIn("EDGE_DIRECT", account.worker_secrets)


if __name__ == "__main__":
    unittest.main()
