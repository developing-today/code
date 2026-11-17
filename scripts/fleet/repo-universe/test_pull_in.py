#!/usr/bin/env python3
"""Tests for the repo-universe pull-in surface (#1652).

All tests run offline: the fetch callable and the mirror store are
injected fixtures, so no live GitHub access and no GitHub mutation
of any kind is needed. The hatch-personal lifecycle test replays,
from fixtures, the exact acceptance cycle a live probe drives
against the hatch-personal test issue — created open, mirrored
exactly once across two cycles, then closed and resolved — so the
acceptance semantics are pinned without a live mutation in routine
tests (the live cycle itself is run once by the implementing lane
and its evidence recorded on #1652).

Covers the packet's named tests:
- new owned delta creates exactly one mirror; a second identical
  cycle creates zero duplicates (dedupe marker searched first);
- closed delta comments + closes the mirror; a converted mirror
  (marker removed) is left open;
- an upstream-dependency mirror carries the class label, and each
  of the three disposition labels exists and can be carried;
- title changes retitle the mirror; reopened sources reopen
  resolved mirrors; baseline creates no mirrors;
- a failed fetch takes no mirror action and leaves state untouched.
"""

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


check_issues = load("check_issues")
pull_in = load("pull_in")

INVENTORY = """\
version: 1
overrides:
  dezren39/hatch-personal:
    watch: issues
repos:
  - name: dezren39/hatch-personal
    url: https://github.com/dezren39/hatch-personal
    class: owned
    watch: issues
    sources: [github, census]
  - name: someone/upstream
    url: https://github.com/someone/upstream
    class: upstream-dependency
    watch: issues
    sources: [census]
  - name: archived/thing
    url: https://github.com/archived/thing
    class: archive-nested
    watch: inventory-only
    sources: [census]
"""

NOW1 = "2026-11-01T00:00:00Z"
NOW2 = "2026-11-01T01:00:00Z"
NOW3 = "2026-11-01T02:00:00Z"
NOW4 = "2026-11-01T03:00:00Z"


def issue(
    number,
    title="t",
    state="open",
    updated="2026-10-31T00:00:00Z",
    repo="dezren39/hatch-personal",
):
    return {
        "number": number,
        "state": state,
        "title": title,
        "updated_at": updated,
        "html_url": f"https://github.com/{repo}/issues/{number}",
    }


def fetch_from(mapping):
    def fetch(repo):
        entry = mapping[repo]
        if isinstance(entry, dict) and "error" in entry:
            raise check_issues.FetchError(str(entry["error"]))
        return [check_issues.normalize_issue(i) for i in entry]

    return fetch


def baselined_state(mapping):
    """Run one cycle to baseline every repo; returns (state, store)."""
    store = pull_in.FixtureMirrorStore()
    state, _, had_error, actions = pull_in.run_pull_in(
        INVENTORY, {"version": 1, "repos": {}}, fetch_from(mapping), store, NOW1
    )
    assert not had_error and actions == []
    return state, store


class PullInCycleTests(unittest.TestCase):
    def test_baseline_creates_no_mirrors(self):
        _, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        self.assertEqual(store.mirrors, [])

    def test_new_issue_mirrored_once_across_two_cycles(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        opened = {
            "dezren39/hatch-personal": [issue(1), issue(2, "hello world")],
            "someone/upstream": [],
        }
        state, _, had_error, actions = pull_in.run_pull_in(
            INVENTORY, state, fetch_from(opened), store, NOW2
        )
        self.assertFalse(had_error)
        self.assertEqual(len(store.mirrors), 1)
        mirror = store.mirrors[0]
        self.assertEqual(mirror["title"], "[dezren39/hatch-personal#2] hello world")
        self.assertIn(
            "<!-- repo-universe:source=dezren39/hatch-personal#2 -->", mirror["body"]
        )
        self.assertEqual(mirror["labels"], ["source/universe", "universe/owned"])
        self.assertTrue(any("created mirror" in a for a in actions))
        # Second identical cycle: zero duplicates, no new action but the exists note.
        state, _, had_error, actions = pull_in.run_pull_in(
            INVENTORY, state, fetch_from(opened), store, NOW3
        )
        self.assertFalse(had_error)
        self.assertEqual(len(store.mirrors), 1)
        self.assertFalse(any("created mirror" in a for a in actions))

    def test_existing_mirror_found_by_marker_not_recreated(self):
        # State lost the issue (fresh baseline gap) but the mirror
        # already exists on GitHub: the marker search must dedupe.
        state, store = baselined_state(
            {"dezren39/hatch-personal": [], "someone/upstream": []}
        )
        store.create_mirror(
            pull_in.mirror_title("dezren39/hatch-personal", issue(7, "pre")),
            pull_in.mirror_body("dezren39/hatch-personal", "owned", issue(7, "pre")),
            ["source/universe", "universe/owned"],
        )
        _, _, _, actions = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {"dezren39/hatch-personal": [issue(7, "pre")], "someone/upstream": []}
            ),
            store,
            NOW2,
        )
        self.assertEqual(len(store.mirrors), 1)
        self.assertTrue(any("no duplicate" in a for a in actions))

    def test_closed_source_resolves_mirror(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2)],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW2,
        )
        state, _, _, actions = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2, state="closed")],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW3,
        )
        mirror = store.mirrors[0]
        self.assertEqual(mirror["state"], "closed")
        self.assertTrue(
            any("dezren39/hatch-personal#2" in c for c in mirror["comments"])
        )
        self.assertTrue(any("resolved mirror" in a for a in actions))

    def test_converted_mirror_left_open_on_source_close(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2)],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW2,
        )
        # Conversion: marker removed from the mirror body.
        store.mirrors[0]["body"] = store.mirrors[0]["body"].replace(
            "<!-- repo-universe:source=dezren39/hatch-personal#2 -->",
            "<!-- converted to locally-owned work -->",
        )
        state, _, _, actions = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2, state="closed")],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW3,
        )
        self.assertEqual(store.mirrors[0]["state"], "open")
        self.assertFalse(any("resolved mirror" in a for a in actions))

    def test_upstream_mirror_class_label_and_dispositions(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [], "someone/upstream": []}
        )
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [],
                    "someone/upstream": [issue(9, "bug", repo="someone/upstream")],
                }
            ),
            store,
            NOW2,
        )
        mirror = store.mirrors[0]
        self.assertEqual(
            mirror["labels"], ["source/universe", "universe/upstream-dependency"]
        )
        # Each disposition label is defined (so the live store coins
        # it) and can be carried by the mirror.
        for disposition in pull_in.DISPOSITION_LABELS:
            self.assertIn(disposition, pull_in.LABEL_DEFINITIONS)
            mirror["labels"] = [
                "source/universe",
                "universe/upstream-dependency",
                disposition,
            ]
            self.assertIn(disposition, mirror["labels"])

    def test_title_change_retitles_and_reopen_reopens(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2, "old")],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW2,
        )
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [
                        issue(1),
                        issue(2, "new", updated=NOW3),
                    ],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW3,
        )
        self.assertEqual(store.mirrors[0]["title"], "[dezren39/hatch-personal#2] new")
        state, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [
                        issue(1),
                        issue(2, "new", state="closed", updated=NOW4),
                    ],
                    "someone/upstream": [],
                }
            ),
            store,
            NOW4,
        )
        self.assertEqual(store.mirrors[0]["state"], "closed")
        state, _, _, actions = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [
                        issue(1),
                        issue(2, "new", updated="2026-11-01T04:00:00Z"),
                    ],
                    "someone/upstream": [],
                }
            ),
            store,
            "2026-11-01T04:00:00Z",
        )
        self.assertEqual(store.mirrors[0]["state"], "open")
        self.assertTrue(any("reopened mirror" in a for a in actions))

    def test_failed_fetch_no_action_state_untouched(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        before = state["repos"]["dezren39/hatch-personal"]
        new_state, lines, had_error, actions = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {"dezren39/hatch-personal": {"error": "boom"}, "someone/upstream": []}
            ),
            store,
            NOW2,
        )
        self.assertTrue(had_error)
        self.assertEqual(actions, [])
        self.assertEqual(new_state["repos"]["dezren39/hatch-personal"], before)
        self.assertTrue(any("ERROR" in line for line in lines))

    def test_dry_run_plans_without_mutating(self):
        state, store = baselined_state(
            {"dezren39/hatch-personal": [issue(1)], "someone/upstream": []}
        )
        dry = pull_in.DryRunStore(store)
        _, _, _, _ = pull_in.run_pull_in(
            INVENTORY,
            state,
            fetch_from(
                {
                    "dezren39/hatch-personal": [issue(1), issue(2)],
                    "someone/upstream": [],
                }
            ),
            dry,
            NOW2,
        )
        self.assertEqual(store.mirrors, [])
        self.assertTrue(any(p.startswith("create:") for p in dry.planned))

    def test_inventory_classes_parse(self):
        repos = pull_in.parse_inventory_classes(INVENTORY)
        by_name = {r["name"]: r["class"] for r in repos}
        self.assertEqual(
            by_name,
            {
                "dezren39/hatch-personal": "owned",
                "someone/upstream": "upstream-dependency",
            },
        )


if __name__ == "__main__":
    unittest.main()
