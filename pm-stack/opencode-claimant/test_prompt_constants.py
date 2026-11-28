#!/usr/bin/env python3
"""Pin prompt.md's restated constants to claimant.py / poll.py (#8366).

Route chosen: pin-by-check (the issue's option b). prompt.md is
hand-tuned prose; generating it would churn the text operators read,
while this suite fails the moment any machine-checkable restatement
drifts from the code — the exact failure the #7772 Phase 4 audit had
to repair by hand (prompt.md's queue-CLI path was stale in two
places against REPO_QUEUE_CMD).

Inventory of prompt.md's machine-checkable restatements — each is
either pinned below or exempted here with a reason:

PINNED (prompt.md restatement -> code constant):
- the four queue names -> claimant.QUEUES == poll.QUEUES
- queue CLI path ~/.local/bin/repo-queue -> claimant.REPO_QUEUE_CMD
  == poll.REPO_QUEUE_CMD
- concurrency cap 36 -> claimant.CONCURRENCY_CAP == poll.CAP
- OpenCode server URL http://127.0.0.1:13420 -> claimant.OPENCODE_URL
  == poll.OPENCODE_URL
- launched-by recovery markers (zero-message-deleted, gone-session)
  -> claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE /
  LAUNCHED_BY_RECOVERY_GONE_SESSION
- 30-minute activity window -> poll.ACTIVE_WINDOW_MS
- 2-hour zombie threshold -> poll.ACTIVE_SET_STALE_MS
- mode file path -> claimant.MODE_FILE
- fire-log path -> claimant.FIRES_FILE
- worker name opencode-claimant -> claimant.CLAIMANT_WORKER
- route defaults (provider opencode-route, model workhorse)
  -> claimant.LAUNCH_MODEL_PROVIDER / LAUNCH_MODEL_ID
- held-issue labels -> claimant.HELD_ISSUE_LABELS
- primary clone ~/git/code -> claimant.PRIMARY_CLONE
  == poll.PRIMARY_CLONE

EXEMPT (in the code, deliberately not restated in prompt.md):
- LAUNCHED_BY_RECOVERY_PROVIDER_WALL: an internal recovery marker;
  the prompt's recovery flow covers only the zero-message and
  gone-session cases an operator session performs by hand.
- BROKER_URL_FILE / BROKER_TOKEN_FILE / AUTH_FILE paths:
  credential-adjacent; the prompt states the rule (no credential
  value or path enters a prompt) instead of the paths.
- LIST_LIMIT, SESSION_FETCH_LIMITS, LEDGER_DB, TURSO_EXEC, the
  UUID-lane regex: poll.py implementation tuning with no
  operator-facing restatement to drift.

Run:  python3 -m unittest pm-stack/opencode-claimant/test_prompt_constants.py
Also wired into run_hermetic_tests.py.
"""

import importlib.util
import re
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


claimant = _load("claimant_for_prompt_check", "claimant.py")
poll = _load("poll_for_prompt_check", "poll.py")

# Whitespace-normalized prompt text: line wrapping in the prose must
# not matter, only the stated values.
PROMPT = re.sub(r"\s+", " ", (HERE / "prompt.md").read_text())


def home_rel(path):
    """Render a HOME_DIR-based path the way prompt.md states it."""
    return "~/" + str(path.relative_to(claimant.HOME_DIR))


class PromptConstantsTest(unittest.TestCase):
    def test_queue_list_matches_both_modules(self):
        self.assertEqual(claimant.QUEUES, poll.QUEUES)
        stated = set(re.findall(r"repo-health-[a-z]+-pilot", PROMPT))
        self.assertEqual(stated, set(claimant.QUEUES))

    def test_queue_cli_path(self):
        self.assertEqual(claimant.REPO_QUEUE_CMD, poll.REPO_QUEUE_CMD)
        self.assertIn(home_rel(claimant.REPO_QUEUE_CMD), PROMPT)

    def test_concurrency_cap(self):
        cap = claimant.CONCURRENCY_CAP
        self.assertEqual(cap, poll.CAP)
        self.assertEqual(cap, poll.CONCURRENCY_CAP)
        m = re.search(r"Concurrency cap: \*\*(\d+)\*\*", PROMPT)
        self.assertIsNotNone(m, "prompt.md no longer states the cap")
        self.assertEqual(int(m.group(1)), cap)
        m = re.search(r"Never exceed the (\d+)-slot claimant cap", PROMPT)
        self.assertIsNotNone(m, "prompt.md hard rules no longer state the cap")
        self.assertEqual(int(m.group(1)), cap)

    def test_server_url(self):
        self.assertEqual(claimant.OPENCODE_URL, poll.OPENCODE_URL)
        self.assertIn(claimant.OPENCODE_URL, PROMPT)

    def test_launched_by_recovery_markers(self):
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_ZERO_MESSAGE, PROMPT)
        self.assertIn(claimant.LAUNCHED_BY_RECOVERY_GONE_SESSION, PROMPT)

    def test_activity_windows(self):
        self.assertEqual(poll.ACTIVE_WINDOW_MS, 30 * 60 * 1000)
        self.assertIn("30 min", PROMPT)
        self.assertIn("30-minute", PROMPT)
        self.assertEqual(poll.ACTIVE_SET_STALE_MS, 2 * 60 * 60 * 1000)
        self.assertIn("silent past 2 h", PROMPT)

    def test_state_paths(self):
        self.assertIn(home_rel(claimant.MODE_FILE), PROMPT)
        self.assertIn(home_rel(claimant.FIRES_FILE), PROMPT)

    def test_worker_name(self):
        self.assertIn(f"--worker {claimant.CLAIMANT_WORKER}", PROMPT)

    def test_route_defaults(self):
        self.assertIn(f'providerID: "{claimant.LAUNCH_MODEL_PROVIDER}"', PROMPT)
        self.assertIn(f'modelID: "{claimant.LAUNCH_MODEL_ID}"', PROMPT)

    def test_held_labels(self):
        for label in claimant.HELD_ISSUE_LABELS:
            self.assertIn(f"`{label}`", PROMPT)

    def test_primary_clone(self):
        self.assertEqual(claimant.PRIMARY_CLONE, poll.PRIMARY_CLONE)
        self.assertIn(home_rel(claimant.PRIMARY_CLONE), PROMPT)


if __name__ == "__main__":
    unittest.main()
