"""Offline tests for lane_guard.py (no network, no gh)."""

import importlib.util
import io
import json
import unittest
from contextlib import redirect_stdout
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


lg = _load("lane_guard", HERE / "lane_guard.py")

NOW = datetime(2026, 11, 19, 18, 0, tzinfo=timezone.utc)
MAINTAINER = [{"login": "dezren39"}]


def issue(labels=(), assignees=MAINTAINER, state="open"):
    return {
        "state": state,
        "labels": [{"name": n} for n in labels],
        "assignees": list(assignees),
    }


def comment(body, minutes_ago, user_type="User"):
    return {
        "body": body,
        "created_at": (NOW - timedelta(minutes=minutes_ago))
        .isoformat()
        .replace("+00:00", "Z"),
        "user": {"login": "dezren39", "type": user_type},
    }


# Note bodies in the two shapes lanes post on #7616.
YAML_PARKED = (
    "agent-note:v1\nstate: parked\ntask: 20028\nlane: oc-code-05 (OpenChamber scheduled lane)\n"
    "worker: repo-worker-code\n\nResume only after explicit ownership release/handoff."
)
HTML_PARKED = (
    "<!-- agent-note:v1 agent=repo-worker-code lane=oc-code-03 state=parked task=20028 "
    "origin=openchamber-scheduled-lane -->\n\nParked queue task 20028 after fresh revalidation."
)
HTML_PICKED = (
    "<!-- agent-note:v1 agent=repo-worker-code lane=oc-code-03 state=picked-up task=20028 -->\n"
    "Picked up task 20028 and checkpointed it."
)
TRANSITION_PARKED = (
    "agent-note:v1 transition: picked-up -> parked worker: repo-worker-code "
    "lane: oc-code-07 task: 20028 summary: Fresh verification"
)
PR_OPENED = "agent-note:v1\nstate: pr_opened\ntask: 20028\nlane: oc-code-04\n\nOpened PR #12100."


class HoldVerdict(unittest.TestCase):
    def test_maintainer_assignment_alone_is_not_a_hold(self):
        v = lg.hold_verdict(issue(["status/parked", "queue/parked", "app/id"]))
        self.assertFalse(v["held"])
        self.assertIn("assignees:dezren39", v["ignored"])

    def test_stale_work_assigned_label_is_not_a_hold(self):
        v = lg.hold_verdict(issue(["work/assigned", "status/parked"]))
        self.assertFalse(v["held"])
        self.assertIn("label:work/assigned", v["ignored"])

    def test_explicit_hold_labels_hold(self):
        for lab in ("held", "user/owned", "do-not-auto-edit"):
            with self.subTest(lab=lab):
                v = lg.hold_verdict(issue([lab]))
                self.assertTrue(v["held"])
                self.assertTrue(v["merge_held"])
                self.assertIn(lab, v["reason"])

    def test_do_not_merge_blocks_merge_not_work(self):
        v = lg.hold_verdict(issue(["do-not-merge"]))
        self.assertFalse(v["held"])
        self.assertTrue(v["merge_held"])

    def test_closed_item_is_not_workable(self):
        self.assertTrue(lg.hold_verdict(issue(state="closed"))["held"])

    def test_unassigned_issue_is_available(self):
        v = lg.hold_verdict(issue(assignees=()))
        self.assertFalse(v["held"])
        self.assertEqual(v["ignored"], [])

    def test_gh_cli_shape_is_accepted(self):
        # `gh issue view --json labels,assignees` uses the same keys.
        v = lg.hold_verdict(
            {
                "state": "OPEN",
                "labels": [{"name": "held"}],
                "assignees": [{"login": "x"}],
            }
        )
        self.assertTrue(v["held"])

    def test_hold_labels_match_claimant_hard_stop(self):
        src = (HERE.parent / "opencode-claimant" / "launch_one.py").read_text()
        line = next(l for l in src.splitlines() if l.startswith("HARD_STOP"))
        hard_stop = set(eval(line.split("=", 1)[1]))  # a literal set
        self.assertEqual(hard_stop, set(lg.HOLD_LABELS | lg.MERGE_ONLY_HOLD_LABELS))


class ParseNote(unittest.TestCase):
    def test_shapes(self):
        self.assertEqual(
            lg.parse_note(YAML_PARKED), {"state": "parked", "task": "20028"}
        )
        self.assertEqual(
            lg.parse_note(HTML_PARKED), {"state": "parked", "task": "20028"}
        )
        self.assertEqual(
            lg.parse_note(HTML_PICKED), {"state": "picked_up", "task": "20028"}
        )
        self.assertEqual(
            lg.parse_note(TRANSITION_PARKED), {"state": "parked", "task": "20028"}
        )
        self.assertEqual(
            lg.parse_note(PR_OPENED), {"state": "pr_opened", "task": "20028"}
        )

    def test_free_form_shapes_lanes_also_post(self):
        cases = {
            "<!-- agent-note:v1 -->\n**Pickup — queue task 20028, lane any-g** (follow-on)": "picked_up",
            "agent-note:v1\n- picked_up: code-f\n- task_id: 20028\n- stage: verified": "picked_up",
            "agent-note:v1 — picked up by repo-worker-code (lane any-n), task 20028.": "picked_up",
            "agent-note:v1\nkind: parked\ntask: 20028 (lane code-j)\nstate: Blockers #11208 closed": "parked",
            "agent-note-v1\nstate: parked\ntask: 20028": "parked",
            "agent-note:v1\nstate: parked - preserved partial renderer\ntask: 20028": "parked",
        }
        for body, state in cases.items():
            with self.subTest(body=body[:40]):
                self.assertEqual(lg.parse_note(body), {"state": state, "task": "20028"})

    def test_unrecognised_state_never_counts_as_duplicate(self):
        body = "<!-- agent-note:v1 -->\n**Resume check — queue task 20028: part 1 still dep-blocked**"
        self.assertEqual(lg.parse_note(body)["state"], None)
        cs = [comment(body, 5)]
        self.assertTrue(lg.note_verdict(cs, "20028", "parked", now=NOW)["post"])

    def test_canonical_header_round_trips(self):
        self.assertEqual(
            lg.parse_note(lg.note_header("#20028", "parked") + "\nParked."),
            {"state": "parked", "task": "20028"},
        )

    def test_plain_comment_is_not_a_note(self):
        self.assertIsNone(lg.parse_note("Looks good, ship it."))
        self.assertIsNone(lg.parse_note(None))


class NoteVerdict(unittest.TestCase):
    def test_repeat_park_within_window_is_skipped(self):
        cs = [comment(HTML_PICKED, 70), comment(YAML_PARKED, 40)]
        v = lg.note_verdict(cs, "20028", "parked", now=NOW)
        self.assertFalse(v["post"])
        self.assertIn("duplicate", v["reason"])

    def test_pick_up_after_park_is_skipped(self):
        cs = [comment(HTML_PARKED, 30)]
        self.assertFalse(lg.note_verdict(cs, "20028", "picked-up", now=NOW)["post"])

    def test_first_park_posts(self):
        self.assertTrue(lg.note_verdict([], "20028", "parked", now=NOW)["post"])

    def test_park_after_window_posts(self):
        cs = [comment(YAML_PARKED, 25 * 60)]
        self.assertTrue(lg.note_verdict(cs, "20028", "parked", now=NOW)["post"])

    def test_human_comment_since_last_note_posts(self):
        cs = [comment(YAML_PARKED, 40), comment("Drew: go ahead and take this.", 10)]
        self.assertTrue(lg.note_verdict(cs, "20028", "parked", now=NOW)["post"])

    def test_bot_comment_does_not_reset_dedupe(self):
        cs = [comment(YAML_PARKED, 40), comment("CI summary", 10, user_type="Bot")]
        self.assertFalse(lg.note_verdict(cs, "20028", "parked", now=NOW)["post"])

    def test_other_task_posts(self):
        cs = [comment(YAML_PARKED, 40)]
        self.assertTrue(lg.note_verdict(cs, "99999", "parked", now=NOW)["post"])

    def test_progress_states_always_post(self):
        cs = [comment(YAML_PARKED, 5)]
        for st in ("pr_opened", "blocked", "completed"):
            with self.subTest(st=st):
                self.assertTrue(lg.note_verdict(cs, "20028", st, now=NOW)["post"])

    def test_park_after_progress_posts(self):
        cs = [comment(YAML_PARKED, 90), comment(PR_OPENED, 30)]
        self.assertTrue(lg.note_verdict(cs, "20028", "parked", now=NOW)["post"])

    def test_7616_churn_collapses_to_one_note_per_day(self):
        # Replay a day of 30-minute pick-up/park cycles: only the first
        # park of the day would have been posted.
        posted = []
        t0 = NOW - timedelta(hours=23)
        for i in range(46):
            at = t0 + timedelta(minutes=30 * i)
            state = "picked_up" if i % 2 == 0 else "parked"
            if lg.note_verdict(posted, "20028", state, now=at)["post"]:
                body = HTML_PICKED if state == "picked_up" else YAML_PARKED
                posted.append(
                    {
                        "body": body,
                        "created_at": at.isoformat(),
                        "user": {"type": "User"},
                    }
                )
        self.assertEqual(len(posted), 1)


class Cli(unittest.TestCase):
    def run_cli(self, argv, gh):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = lg.main(argv, gh=gh)
        return code, json.loads(buf.getvalue())

    def test_hold_cli_available(self):
        code, out = self.run_cli(
            ["hold", "--repo", "o/r", "--issue", "7616"],
            gh=lambda path: issue(["status/parked", "work/assigned"]),
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertFalse(out["held"])

    def test_hold_cli_held(self):
        code, _ = self.run_cli(
            ["hold", "--repo", "o/r", "--issue", "1"], gh=lambda p: issue(["held"])
        )
        self.assertEqual(code, lg.EXIT_STOP)

    def test_note_cli_requests_issue_comments(self):
        seen = []

        def gh(path):
            seen.append(path)
            return [comment(YAML_PARKED, 5)]

        code, out = self.run_cli(
            [
                "note",
                "--repo",
                "o/r",
                "--issue",
                "7616",
                "--task",
                "20028",
                "--state",
                "parked",
            ],
            gh=gh,
        )
        self.assertEqual(seen, ["repos/o/r/issues/7616/comments?per_page=100"])
        self.assertEqual(code, lg.EXIT_STOP)
        self.assertFalse(out["post"])
        self.assertEqual(
            out["header"], "<!-- agent-note:v1 state=parked task=20028 -->"
        )

    def test_gh_failure_is_exit_2(self):
        def boom(path):
            raise RuntimeError("HTTP 502")

        code, out = self.run_cli(["hold", "--repo", "o/r", "--issue", "1"], gh=boom)
        self.assertEqual(code, lg.EXIT_USAGE)
        self.assertIn("502", out["error"])


if __name__ == "__main__":
    unittest.main()
