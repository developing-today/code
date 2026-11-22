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
# Triage verdict notes (#12233 F2a): the same verdict recorded by two
# different queue tasks, in the HTML and YAML shapes. Each records a
# fingerprint of the issue body it was triaged against (see
# body_fingerprint); ISSUE_BODY is that body.
ISSUE_BODY = "Triage me: still blocked on F12.4 (#1061).\n\nDetails here."
HTML_TRIAGED = (
    lg.note_header("11111", "triaged", verdict="still_blocked", body=ISSUE_BODY)
    + "\nTriaged: still blocked on F12.4 (#1061)."
)
YAML_TRIAGED = (
    "agent-note:v1\nstate: triaged\ntask: 22222\nverdict: still_blocked\n"
    f"body_fp: {lg.body_fingerprint(ISSUE_BODY)}\n\n"
    "Verdict: still blocked on F12.4 (#1061)."
)
# A verdict note recorded before body fingerprints existed: no
# body_fp field, so a body edit since it cannot be ruled out.
HTML_TRIAGED_LEGACY = (
    "<!-- agent-note:v1 state=triaged task=11111 verdict=still_blocked -->\n"
    "Triaged: still blocked on F12.4 (#1061)."
)
TRIAGED_NO_VERDICT = "agent-note:v1\nstate: triaged\ntask: 22222\n\nStill live."


def at(minutes_ago):
    return (NOW - timedelta(minutes=minutes_ago)).isoformat().replace("+00:00", "Z")


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

    def test_verdict_field_is_parsed_and_normalised(self):
        body = "agent-note:v1\nstate: triaged\ntask: 22222\nverdict: Still Blocked!\n"
        self.assertEqual(
            lg.parse_note(body),
            {"state": "triaged", "task": "22222", "verdict": "still_blocked"},
        )

    def test_canonical_header_with_verdict_round_trips(self):
        self.assertEqual(
            lg.parse_note(
                lg.note_header("20028", "triaged", verdict="still blocked")
                + "\nTriaged."
            ),
            {"state": "triaged", "task": "20028", "verdict": "still_blocked"},
        )

    def test_body_fingerprint_round_trips_in_header(self):
        header = lg.note_header(
            "20028", "triaged", verdict="still blocked", body=ISSUE_BODY
        )
        parsed = lg.parse_note(header + "\nTriaged.")
        self.assertEqual(
            parsed,
            {
                "state": "triaged",
                "task": "20028",
                "verdict": "still_blocked",
                "body_fp": lg.body_fingerprint(ISSUE_BODY),
            },
        )

    def test_body_fingerprint_is_stable(self):
        # 16 hex chars; surrounding whitespace and CRLF do not change
        # the fingerprint, so a trivial reformat does not re-triage.
        fp = lg.body_fingerprint(ISSUE_BODY)
        self.assertRegex(fp, r"^[0-9a-f]{16}$")
        self.assertEqual(lg.body_fingerprint("  " + ISSUE_BODY + "\n\n"), fp)
        self.assertEqual(
            lg.body_fingerprint(ISSUE_BODY.replace("\n", "\r\n")), fp
        )
        self.assertNotEqual(lg.body_fingerprint(ISSUE_BODY + " edited"), fp)

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

    def test_24_hour_boundary_posts_at_but_not_before_window(self):
        exactly_at_boundary = [comment(YAML_PARKED, 24 * 60)]
        just_inside_window = [comment(YAML_PARKED, 24 * 60 - 1)]
        self.assertTrue(
            lg.note_verdict(exactly_at_boundary, "20028", "parked", now=NOW)["post"]
        )
        self.assertFalse(
            lg.note_verdict(just_inside_window, "20028", "parked", now=NOW)["post"]
        )

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


class VerdictNote(unittest.TestCase):
    """#12233 F2a: a triage verdict is posted once per verdict per
    unchanged issue, across task ids."""

    def verdict(self, cs, task="99999", verdict="still_blocked", changes=(),
                body=ISSUE_BODY):
        return lg.note_verdict(
            cs, task, "triaged", now=NOW, verdict=verdict, changes=changes,
            body=body,
        )

    def test_same_verdict_other_task_unchanged_issue_is_skipped(self):
        # The F2a done-test: two tasks, one unchanged issue — the
        # second task's identical verdict note is skipped.
        for body in (HTML_TRIAGED, YAML_TRIAGED):
            with self.subTest(body=body[:30]):
                v = self.verdict([comment(body, 40)])
                self.assertFalse(v["post"])
                self.assertIn("duplicate verdict", v["reason"])

    def test_verdict_spelling_is_normalised(self):
        v = self.verdict([comment(HTML_TRIAGED, 40)], verdict="Still blocked!")
        self.assertFalse(v["post"])

    def test_new_commit_after_verdict_posts(self):
        # The other half of the F2a done-test: a third note, after a
        # new commit lands on the issue, is posted.
        changes = [{"kind": "commit", "at": at(10)}]
        v = self.verdict([comment(HTML_TRIAGED, 40)], changes=changes)
        self.assertTrue(v["post"])
        self.assertIn("commit", v["reason"])

    def test_label_change_after_verdict_posts(self):
        changes = [{"kind": "label", "at": at(10)}]
        v = self.verdict([comment(HTML_TRIAGED, 40)], changes=changes)
        self.assertTrue(v["post"])
        self.assertIn("label", v["reason"])

    def test_body_edit_after_verdict_posts(self):
        # The review finding: the timeline never reports body edits,
        # so the fingerprint is the signal. A changed body re-opens
        # the verdict even with no comment, label or commit since.
        v = self.verdict(
            [comment(HTML_TRIAGED, 40)], body=ISSUE_BODY + "\nEdited: new scope."
        )
        self.assertTrue(v["post"])
        self.assertIn("body", v["reason"])

    def test_body_unchanged_after_verdict_skips(self):
        v = self.verdict([comment(HTML_TRIAGED, 40)], body=ISSUE_BODY)
        self.assertFalse(v["post"])
        self.assertIn("duplicate verdict", v["reason"])

    def test_verdict_without_body_fingerprint_posts_once(self):
        # A legacy verdict note records no fingerprint, so a body
        # edit since it cannot be ruled out: post once, and the new
        # note (via the CLI header) records one.
        v = self.verdict([comment(HTML_TRIAGED_LEGACY, 40)])
        self.assertTrue(v["post"])
        self.assertIn("fingerprint", v["reason"])

    def test_change_before_the_verdict_does_not_reopen_it(self):
        changes = [{"kind": "commit", "at": at(50)}]
        v = self.verdict([comment(HTML_TRIAGED, 40)], changes=changes)
        self.assertFalse(v["post"])

    def test_different_verdict_posts(self):
        v = self.verdict([comment(HTML_TRIAGED, 40)], verdict="needs_slice")
        self.assertTrue(v["post"])
        self.assertIn("needs_slice", v["reason"])

    def test_verdict_has_no_dedupe_window(self):
        # A verdict stands until the issue changes: three days on,
        # the repeat is still skipped (park notes expire in 24h).
        v = self.verdict([comment(HTML_TRIAGED, 3 * 24 * 60)])
        self.assertFalse(v["post"])

    def test_human_comment_after_verdict_posts(self):
        cs = [comment(HTML_TRIAGED, 40), comment("Drew: take another look.", 10)]
        self.assertTrue(self.verdict(cs)["post"])

    def test_bot_comment_after_verdict_still_skips(self):
        cs = [comment(HTML_TRIAGED, 40), comment("CI summary", 10, user_type="Bot")]
        self.assertFalse(self.verdict(cs)["post"])

    def test_newest_note_not_a_verdict_posts(self):
        v = self.verdict([comment(YAML_PARKED, 10)])
        self.assertTrue(v["post"])
        self.assertIn("not a triage verdict", v["reason"])

    def test_state_only_verdicts_match_across_tasks(self):
        # Notes recorded before --verdict existed carry only the
        # state; two of those still dedupe.
        v = lg.note_verdict(
            [comment(TRIAGED_NO_VERDICT, 40)], "99999", "triaged", now=NOW
        )
        self.assertFalse(v["post"])

    def test_recorded_verdict_does_not_match_unrecorded(self):
        v = lg.note_verdict(
            [comment(HTML_TRIAGED, 40)], "99999", "triaged", now=NOW
        )
        self.assertTrue(v["post"])

    def test_first_verdict_posts(self):
        self.assertTrue(self.verdict([])["post"])


class ChangesFromTimeline(unittest.TestCase):
    def test_maps_change_events(self):
        # No `edited` event: GitHub's REST timeline exposes no such
        # event type, so a body edit must never be expected here —
        # it is detected by body fingerprint instead.
        events = [
            {"event": "labeled", "created_at": at(50)},
            {"event": "commented", "created_at": at(45)},
            {"event": "referenced", "commit_id": "abc123", "created_at": at(40)},
            {"event": "referenced", "created_at": at(35)},  # cross-ref, no commit
            {"event": "edited", "created_at": at(30)},  # not a real timeline event
            {"event": "unlabeled", "created_at": at(25)},
            {"event": "assigned", "created_at": at(20)},
        ]
        self.assertEqual(
            lg.changes_from_timeline(events),
            [
                {"kind": "label", "at": at(50)},
                {"kind": "commit", "at": at(40)},
                {"kind": "label", "at": at(25)},
            ],
        )

    def test_junk_events_are_ignored(self):
        self.assertEqual(lg.changes_from_timeline([]), [])
        self.assertEqual(lg.changes_from_timeline([None, {}, {"event": "labeled"}]), [])


class Cli(unittest.TestCase):
    def run_cli(self, argv, gh, now=NOW):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = lg.main(argv, gh=gh, now=now)
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

    def test_note_cli_verdict_fetches_timeline_and_skips_duplicate(self):
        seen = []

        def gh(path):
            seen.append(path)
            if "timeline" in path:
                return []
            if "comments" in path:
                return [comment(HTML_TRIAGED, 5)]
            return {"body": ISSUE_BODY}

        code, out = self.run_cli(
            [
                "note",
                "--repo",
                "o/r",
                "--issue",
                "1062",
                "--task",
                "20028",
                "--state",
                "triaged",
                "--verdict",
                "still_blocked",
            ],
            gh=gh,
        )
        self.assertEqual(
            seen,
            [
                "repos/o/r/issues/1062/comments?per_page=100",
                "repos/o/r/issues/1062",
                "repos/o/r/issues/1062/timeline?per_page=100",
            ],
        )
        self.assertEqual(code, lg.EXIT_STOP)
        self.assertFalse(out["post"])
        self.assertEqual(
            out["header"],
            "<!-- agent-note:v1 state=triaged task=20028 "
            f"verdict=still_blocked body_fp={lg.body_fingerprint(ISSUE_BODY)} -->",
        )

    def test_note_cli_verdict_posts_after_a_commit(self):
        def gh(path):
            if "timeline" in path:
                return [
                    {"event": "referenced", "commit_id": "abc123", "created_at": at(1)}
                ]
            if "comments" in path:
                return [comment(HTML_TRIAGED, 5)]
            return {"body": ISSUE_BODY}

        code, out = self.run_cli(
            [
                "note",
                "--repo",
                "o/r",
                "--issue",
                "1062",
                "--task",
                "20028",
                "--state",
                "triaged",
                "--verdict",
                "still_blocked",
            ],
            gh=gh,
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertTrue(out["post"])
        self.assertIn("commit", out["reason"])

    def test_note_cli_verdict_posts_after_a_body_edit(self):
        def gh(path):
            if "timeline" in path:
                return []
            if "comments" in path:
                return [comment(HTML_TRIAGED, 5)]
            return {"body": ISSUE_BODY + "\nEdited: new scope."}

        code, out = self.run_cli(
            [
                "note",
                "--repo",
                "o/r",
                "--issue",
                "1062",
                "--task",
                "20028",
                "--state",
                "triaged",
                "--verdict",
                "still_blocked",
            ],
            gh=gh,
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertTrue(out["post"])
        self.assertIn("body", out["reason"])

    def test_gh_failure_is_exit_2(self):
        def boom(path):
            raise RuntimeError("HTTP 502")

        code, out = self.run_cli(["hold", "--repo", "o/r", "--issue", "1"], gh=boom)
        self.assertEqual(code, lg.EXIT_USAGE)
        self.assertIn("502", out["error"])


if __name__ == "__main__":
    unittest.main()
