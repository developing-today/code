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
# different queue tasks, in the HTML and YAML shapes.
HTML_TRIAGED = (
    "<!-- agent-note:v1 state=triaged task=11111 verdict=still_blocked -->\n"
    "Triaged: still blocked on F12.4 (#1061)."
)
YAML_TRIAGED = (
    "agent-note:v1\nstate: triaged\ntask: 22222\nverdict: still_blocked\n\n"
    "Verdict: still blocked on F12.4 (#1061)."
)
TRIAGED_NO_VERDICT = "agent-note:v1\nstate: triaged\ntask: 22222\n\nStill live."
# Verdict notes carrying the F2b resume condition, in both shapes.
HTML_TRIAGED_RESUME = (
    "<!-- agent-note:v1 state=triaged task=11111 resume=dependency_closed:539 "
    "verdict=still_blocked -->\nTriaged: still blocked on F11.8 (#539)."
)
YAML_TRIAGED_RESUME = (
    "agent-note:v1\nstate: triaged\ntask: 22222\nverdict: still_blocked\n"
    "resume: slice landed #12543\n\nVerdict: still blocked; the F2a slice lands first."
)


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

    def test_resume_field_is_parsed_and_canonicalised(self):
        for body in (HTML_TRIAGED_RESUME, YAML_TRIAGED_RESUME):
            with self.subTest(body=body[:30]):
                note = lg.parse_note(body)
                self.assertEqual(note["state"], "triaged")
                self.assertEqual(note["verdict"], "still_blocked")
                self.assertIn(note["resume"], ("dependency_closed:539", "slice_landed:12543"))

    def test_canonical_header_with_resume_round_trips(self):
        self.assertEqual(
            lg.parse_note(
                lg.note_header(
                    "20028",
                    "triaged",
                    verdict="still_blocked",
                    resume="dependency closed #539",
                )
                + "\nTriaged."
            ),
            {
                "state": "triaged",
                "task": "20028",
                "verdict": "still_blocked",
                "resume": "dependency_closed:539",
            },
        )

    def test_parse_resume_rejects_dates_people_and_bare_kinds(self):
        for raw in ("next week", "2027-01-01", "the owner", "slice_landed", "dependency_closed:"):
            with self.subTest(raw=raw):
                self.assertIsNone(lg.parse_resume(raw))
        self.assertEqual(lg.parse_resume("slice_landed:12543"), "slice_landed:12543")
        self.assertEqual(
            lg.parse_resume("dependency closed #539"), "dependency_closed:539"
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

    def verdict(self, cs, task="99999", verdict="still_blocked", changes=(), resume=None):
        return lg.note_verdict(
            cs, task, "triaged", now=NOW, verdict=verdict, changes=changes,
            resume=resume,
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

    def test_label_and_body_changes_after_verdict_post(self):
        for kind in ("label", "body"):
            with self.subTest(kind=kind):
                changes = [{"kind": kind, "at": at(10)}]
                v = self.verdict([comment(HTML_TRIAGED, 40)], changes=changes)
                self.assertTrue(v["post"])
                self.assertIn(kind, v["reason"])

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

    def test_new_resume_condition_posts_despite_duplicate_verdict(self):
        # Thread 4 (P2, #12565): the stored verdict has no resume
        # condition; the same verdict WITH one still posts, so the
        # caller can upgrade it to F2b coverage.
        v = self.verdict(
            [comment(HTML_TRIAGED, 40)], resume="dependency_closed:539"
        )
        self.assertTrue(v["post"])
        self.assertIn("resume condition", v["reason"])

    def test_same_resume_condition_still_skips(self):
        v = self.verdict(
            [comment(HTML_TRIAGED_RESUME, 40)], resume="dependency_closed:539"
        )
        self.assertFalse(v["post"])

    def test_changed_resume_condition_posts(self):
        v = self.verdict(
            [comment(HTML_TRIAGED_RESUME, 40)], resume="slice_landed:12543"
        )
        self.assertTrue(v["post"])
        self.assertIn("resume condition", v["reason"])

    def test_invalid_resume_condition_still_skips(self):
        # A resume value that is not a checkable condition is not a
        # reason to re-post.
        v = self.verdict([comment(HTML_TRIAGED, 40)], resume="next week")
        self.assertFalse(v["post"])

    def test_first_verdict_posts(self):
        self.assertTrue(self.verdict([])["post"])


class CoverageVerdict(unittest.TestCase):
    """#12233 F2b: coverage accepts a verdict — an open effort/l or
    effort/m issue whose newest agent note is a recorded triage
    verdict with a valid resume condition is covered, and an issue
    with no verdict is still uncovered."""

    def coverage(self, cs, labels=("effort/l",), changes=(), state="open",
                 resume_fulfilled=None):
        return lg.coverage_verdict(
            issue(list(labels), state=state), cs, changes=changes,
            resume_fulfilled=resume_fulfilled,
        )

    def test_verdict_with_resume_covers_large_and_medium(self):
        for body in (HTML_TRIAGED_RESUME, YAML_TRIAGED_RESUME):
            for labels in (("effort/l",), ("effort/m",)):
                with self.subTest(body=body[:30], labels=labels):
                    v = self.coverage([comment(body, 40)], labels=labels)
                    self.assertTrue(v["covered"])
                    self.assertEqual(v["verdict"], "still_blocked")
                    self.assertIn("resume condition", v["reason"])

    def test_no_verdict_is_not_covered(self):
        self.assertFalse(self.coverage([])["covered"])
        v = self.coverage([comment(YAML_PARKED, 10)])
        self.assertFalse(v["covered"])
        self.assertIn("not a recorded triage verdict", v["reason"])

    def test_state_only_verdict_is_not_covered(self):
        # Coverage reads the recorded token (decision record rule 1);
        # a state-only note has none.
        v = self.coverage([comment(TRIAGED_NO_VERDICT, 40)])
        self.assertFalse(v["covered"])

    def test_verdict_without_resume_is_not_covered(self):
        v = self.coverage([comment(HTML_TRIAGED, 40)])
        self.assertFalse(v["covered"])
        self.assertIn("no valid resume condition", v["reason"])

    def test_placeholder_verdict_is_not_covered(self):
        body = (
            "<!-- agent-note:v1 state=triaged task=11111 "
            "resume=dependency_closed:539 verdict=untriaged -->\nTriaged."
        )
        v = self.coverage([comment(body, 40)])
        self.assertFalse(v["covered"])
        self.assertIn("records no outcome", v["reason"])

    def test_small_or_unlabelled_issue_is_not_covered(self):
        for labels in (("effort/s",), ()):
            with self.subTest(labels=labels):
                v = self.coverage([comment(HTML_TRIAGED_RESUME, 40)], labels=labels)
                self.assertFalse(v["covered"])
                self.assertIn("effort/l or effort/m", v["reason"])

    def test_closed_issue_is_not_covered(self):
        v = self.coverage([comment(HTML_TRIAGED_RESUME, 40)], state="closed")
        self.assertFalse(v["covered"])
        self.assertIn("not open", v["reason"])

    def test_human_comment_after_verdict_ends_coverage(self):
        cs = [comment(HTML_TRIAGED_RESUME, 40), comment("Drew: reopen this.", 10)]
        self.assertFalse(self.coverage(cs)["covered"])

    def test_bot_comment_after_verdict_keeps_coverage(self):
        cs = [
            comment(HTML_TRIAGED_RESUME, 40),
            comment("CI summary", 10, user_type="Bot"),
        ]
        self.assertTrue(self.coverage(cs)["covered"])

    def test_newer_progress_note_ends_coverage(self):
        cs = [comment(HTML_TRIAGED_RESUME, 40), comment(PR_OPENED, 10)]
        self.assertFalse(self.coverage(cs)["covered"])

    def test_issue_change_after_verdict_ends_coverage(self):
        changes = [{"kind": "commit", "at": at(10)}]
        v = self.coverage([comment(HTML_TRIAGED_RESUME, 40)], changes=changes)
        self.assertFalse(v["covered"])
        self.assertIn("commit", v["reason"])

    def test_change_before_the_verdict_keeps_coverage(self):
        changes = [{"kind": "label", "at": at(50)}]
        v = self.coverage([comment(HTML_TRIAGED_RESUME, 40)], changes=changes)
        self.assertTrue(v["covered"])

    def test_fulfilled_resume_condition_ends_coverage(self):
        # Thread 1 (P1, #12565): the dependency closed, so the
        # verdict's wait is over and the issue is due for re-triage.
        seen = []
        v = self.coverage(
            [comment(HTML_TRIAGED_RESUME, 40)],
            resume_fulfilled=lambda r: seen.append(r) or True,
        )
        self.assertFalse(v["covered"])
        self.assertIn("fulfilled", v["reason"])
        self.assertEqual(seen, ["dependency_closed:539"])

    def test_unfulfilled_resume_condition_keeps_coverage(self):
        v = self.coverage(
            [comment(HTML_TRIAGED_RESUME, 40)],
            resume_fulfilled=lambda r: False,
        )
        self.assertTrue(v["covered"])

    def test_unknown_resume_state_keeps_coverage(self):
        v = self.coverage(
            [comment(HTML_TRIAGED_RESUME, 40)],
            resume_fulfilled=lambda r: None,
        )
        self.assertTrue(v["covered"])

    def test_resume_checker_error_keeps_coverage(self):
        def boom(r):
            raise RuntimeError("gh down")

        v = self.coverage(
            [comment(HTML_TRIAGED_RESUME, 40)], resume_fulfilled=boom
        )
        self.assertTrue(v["covered"])


class ChangesFromTimeline(unittest.TestCase):
    def test_maps_change_events(self):
        events = [
            {"event": "labeled", "created_at": at(50)},
            {"event": "commented", "created_at": at(45)},
            {"event": "referenced", "commit_id": "abc123", "created_at": at(40)},
            {"event": "referenced", "created_at": at(35)},  # cross-ref, no commit
            {"event": "edited", "created_at": at(30)},
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
            return [comment(HTML_TRIAGED, 5)]

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
            "<!-- agent-note:v1 state=triaged task=20028 verdict=still_blocked -->",
        )

    def test_note_cli_verdict_posts_after_a_commit(self):
        def gh(path):
            if "timeline" in path:
                return [
                    {"event": "referenced", "commit_id": "abc123", "created_at": at(1)}
                ]
            return [comment(HTML_TRIAGED, 5)]

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

    def test_coverage_cli_covered_is_stop(self):
        seen = []

        def gh(path):
            seen.append(path)
            if "timeline" in path:
                return []
            if "comments" in path:
                return [comment(HTML_TRIAGED_RESUME, 5)]
            return issue(["effort/l"])

        code, out = self.run_cli(
            ["coverage", "--repo", "o/r", "--issue", "1062"], gh=gh
        )
        self.assertEqual(
            seen,
            [
                "repos/o/r/issues/1062",
                "repos/o/r/issues/1062/comments?per_page=100",
                "repos/o/r/issues/1062/timeline?per_page=100",
                # The resume target is read live: dependency 539 is
                # open here, so the wait is not over.
                "repos/o/r/issues/539",
            ],
        )
        self.assertEqual(code, lg.EXIT_STOP)
        self.assertTrue(out["covered"])
        self.assertEqual(out["resume"], "dependency_closed:539")

    def test_coverage_cli_fulfilled_resume_is_go(self):
        def gh(path):
            if "timeline" in path:
                return []
            if "comments" in path:
                return [comment(HTML_TRIAGED_RESUME, 5)]
            if path.endswith("/issues/539"):
                return {"state": "closed"}
            return issue(["effort/l"])

        code, out = self.run_cli(
            ["coverage", "--repo", "o/r", "--issue", "1062"], gh=gh
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertFalse(out["covered"])
        self.assertIn("fulfilled", out["reason"])

    def test_coverage_cli_uncovered_is_go(self):
        def gh(path):
            if "timeline" in path:
                return []
            if "comments" in path:
                return [comment(HTML_TRIAGED_RESUME, 5)]
            return issue(["effort/s"])

        code, out = self.run_cli(
            ["coverage", "--repo", "o/r", "--issue", "1062"], gh=gh
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertFalse(out["covered"])

    def test_note_cli_with_resume_writes_it_into_the_header(self):
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
                "--resume",
                "dependency_closed:539",
            ],
            gh=lambda path: [],
        )
        self.assertEqual(code, lg.EXIT_GO)
        self.assertEqual(
            out["header"],
            "<!-- agent-note:v1 state=triaged task=20028 "
            "resume=dependency_closed:539 verdict=still_blocked -->",
        )

    def test_gh_failure_is_exit_2(self):
        def boom(path):
            raise RuntimeError("HTTP 502")

        code, out = self.run_cli(["hold", "--repo", "o/r", "--issue", "1"], gh=boom)
        self.assertEqual(code, lg.EXIT_USAGE)
        self.assertIn("502", out["error"])


if __name__ == "__main__":
    unittest.main()
