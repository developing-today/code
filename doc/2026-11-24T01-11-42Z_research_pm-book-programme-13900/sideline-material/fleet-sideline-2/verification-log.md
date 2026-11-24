# FLEET-SIDELINE-2 — verification log, 2026-11-23

- Worktree: ~/wt-fleet-gaps-book (laptop), branch book/fleet-gaps-additions,
  created off origin/main 13794eaa44c; later merged current main (incl.
  Lab Book PR #13968) before final build. Flagged worktrees
  ~/wt-pm-book-records and ~/wt-pm-records-2 never entered. No flag tripped.
- Draft transfer: tar+base64 in 6000-char argument chunks via
  laptop-ssh.sh; sha256 2a7f57dc9ccadb1543e76b9f98287f56766329b1e02bfa4faa09eabc3b3b355a
  matched both sides. Lab cross-links rewrite transferred singly,
  sha256 1390943d278dd999bc221ea7a3ee37c753bc299fa60c80902db9c6d6149d1415 matched.
- Audits (python script, /tmp/audit.py on laptop): H1 == SUMMARY entry
  for all 6 new chapters; all internal .md links resolve; hedge grep
  2 deliberate keeps (sources' own terms).
- mdbook build (pkgs/fleet, dest /tmp/fleet-book-render-fs2*): exit 0,
  0 warnings, three runs (pre-treefmt, post-treefmt, post-main-merge).
  Rendered HTML spot checks passed (window_start placeholder, alert keys).
- treefmt: applied to the 6 new chapters (table padding). SUMMARY.md
  unchanged by formatter. program-issue-map.md: formatter would re-pad
  the pre-existing 203-row census table (not treefmt-clean on main);
  formatting reverted for that file, addendum kept in the file's style.
- nix build .#fleet-docs --no-link: green twice —
  /nix/store/3hm1x1dv625yfc021iwifzcn4fdcfgjg-fleet-docs (first commit),
  /nix/store/1jafkk0mggq0p17vp5g6wk14aisasvip-fleet-docs (final tree).
- Commits: aa43762ff5f (main content), 98dd481c783 (Lab Book rewrite).
- PR #13978 merged via laptop gh: merge commit
  00c4873dd11bc3233f0f05ed7bf6da7cc21f80c3, merged_at 2026-11-24T01:29:31Z
  (read back via fleet gh api).
- Disagreements D36–D38 filed as issue #13977 via gh-post (posted user);
  D36 correction comment 7119159587. Results comment on #13900:
  comment 7119161311.
- Coordinator state re-verified at merge time: git grep placement_coord
  on fresh origin/main (20620a5f5ef) — still absent. Designed, not built.
- Live Pages fetch of the new chapters: see final report (deploy runs
  on main via the combined mcpx-docs workflow; not gate-checked here
  beyond the nix build, which is the same derivation the deploy stages).
