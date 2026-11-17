#!/usr/bin/env python3
"""Gate-decision tests for the fleet worktree janitor (code issue #6884).

Oracle: Drew's policy on #6884 (2026-11-07, as corrected twice the same
day) and the provably-safe gate recorded by the 2026-11-10/11 cleanup
pass on that issue. Every test names the policy clause it pins:

- nothing younger than 4 hours is ever touched (Drew's correction
  superseding the body's 48h figure);
- a clean worktree is removed only when ALL of: clean incl. untracked,
  HEAD ancestor of origin/main, zero commits absent from every origin
  ref, no live process cwd inside, last commit and tree mtime both
  >= 4h old — re-verified immediately before removal (the re-check
  itself is executor behaviour; here we pin the decision it feeds);
- an aging dirty worktree on a branch is preserved by Drew's
  mechanism — commit all, push upstream under its own branch name,
  verify, then remove — never archive/* branches, never a local tier;
- a dirty worktree on a detached HEAD is kept (no branch to push
  under; the janitor invents no branch);
- primaries, locked worktrees, submodule trees, and anything outside
  the fleet root are always kept; a vanished registration is pruned.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))

from worktree_janitor import MIN_AGE_SECONDS, WorktreeFacts, decide

NOW = 1_800_000_000
OLD = NOW - 5 * 3600  # 5h ago: past the 4h gate


def facts(**over):
    base = dict(
        path="/home/user/git/fleet/1234-example",
        primary="/home/user/git/code",
        branch="queue/1234-example",
        is_primary=False,
        under_fleet=True,
        locked=False,
        prunable=False,
        dirty=False,
        has_submodules=False,
        head_ancestor_of_origin_main=True,
        commits_not_on_origin=0,
        live_cwd=False,
        last_commit_epoch=OLD,
        tree_mtime_epoch=OLD,
    )
    base.update(over)
    return WorktreeFacts(**base)


class GateTests(unittest.TestCase):
    def test_fully_gated_clean_worktree_is_removed(self):
        d = decide(facts(), NOW)
        self.assertEqual(d.action, "remove")

    def test_dirty_worktree_is_never_on_the_clean_remove_path(self):
        d = decide(facts(dirty=True), NOW)
        self.assertNotEqual(d.action, "remove")

    def test_unpushed_commits_keep_the_worktree(self):
        d = decide(facts(commits_not_on_origin=3), NOW)
        self.assertEqual(d.action, "keep")
        self.assertIn("absent from every origin ref", d.reasons[0])

    def test_non_ancestor_head_keeps_the_worktree(self):
        d = decide(facts(head_ancestor_of_origin_main=False), NOW)
        self.assertEqual(d.action, "keep")

    def test_live_cwd_keeps_the_worktree(self):
        d = decide(facts(live_cwd=True), NOW)
        self.assertEqual(d.action, "keep")

    def test_young_last_commit_keeps_the_worktree(self):
        d = decide(facts(last_commit_epoch=NOW - 3600), NOW)
        self.assertEqual(d.action, "keep")
        self.assertIn("younger than 4h", d.reasons[0])

    def test_young_tree_mtime_keeps_the_worktree(self):
        # The 2026-11-10/11 gate requires BOTH ages past 4h.
        d = decide(facts(tree_mtime_epoch=NOW - 60), NOW)
        self.assertEqual(d.action, "keep")

    def test_exactly_four_hours_is_old_enough(self):
        edge = NOW - MIN_AGE_SECONDS
        d = decide(facts(last_commit_epoch=edge, tree_mtime_epoch=edge), NOW)
        self.assertEqual(d.action, "remove")

    def test_primary_is_always_kept(self):
        d = decide(facts(is_primary=True), NOW)
        self.assertEqual(d.action, "keep")

    def test_outside_fleet_root_is_kept(self):
        d = decide(facts(under_fleet=False), NOW)
        self.assertEqual(d.action, "keep")

    def test_locked_worktree_is_kept(self):
        d = decide(facts(locked=True), NOW)
        self.assertEqual(d.action, "keep")

    def test_submodule_worktree_is_kept(self):
        # git refuses plain removal of worktrees containing submodules;
        # the 2026-11-10/11 pass kept both such trees.
        d = decide(facts(has_submodules=True), NOW)
        self.assertEqual(d.action, "keep")

    def test_vanished_registration_is_pruned(self):
        d = decide(facts(prunable=True), NOW)
        self.assertEqual(d.action, "prune_registration")

    def test_aging_dirty_on_branch_gets_drews_preserve_mechanism(self):
        d = decide(facts(dirty=True), NOW)
        self.assertEqual(d.action, "preserve_then_remove")
        self.assertIn("push origin queue/1234-example", d.reasons[0])

    def test_aging_dirty_detached_is_kept_no_invented_branch(self):
        d = decide(facts(dirty=True, branch=None), NOW)
        self.assertEqual(d.action, "keep")
        self.assertIn("detached", d.reasons[0])

    def test_young_dirty_worktree_is_kept_before_any_preserve(self):
        d = decide(facts(dirty=True, last_commit_epoch=NOW - 600), NOW)
        self.assertEqual(d.action, "keep")

    def test_live_cwd_blocks_even_the_preserve_path(self):
        d = decide(facts(dirty=True, live_cwd=True), NOW)
        self.assertEqual(d.action, "keep")


class MtimeWalkTests(unittest.TestCase):
    def test_young_file_settles_the_walk_early(self):
        # Oracle: the age gate needs only "no file younger than 4h";
        # one young file proves young without walking the whole tree.
        import tempfile

        from worktree_janitor import newest_tree_mtime

        with tempfile.TemporaryDirectory() as td:
            old_file = os.path.join(td, "old.txt")
            young_file = os.path.join(td, "young.txt")
            for path, when in ((old_file, OLD), (young_file, NOW - 60)):
                with open(path, "w") as fh:
                    fh.write("x")
                os.utime(path, (when, when))
            got = newest_tree_mtime(td, young_epoch=NOW - MIN_AGE_SECONDS)
            self.assertEqual(got, NOW - 60)


if __name__ == "__main__":
    unittest.main(verbosity=2)
