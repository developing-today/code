# Publication and live audit — PM orchestration granular issue batch

Date: 2026-11-13. Executor: Muse lane code-c, queue task 15381 (resumed after the
bound Summit session ses_e5e2d10e06effeEylSDDVaN0PL was cleared via expire_leases
at 2026-11-13T11:00:04Z; lane assignment recorded direct per #8187).

This record supersedes the "not ready to publish" verdict in
reviews/final-package.md (2026-11-13 00:42 local). That review's gates were met
in order: its two offline defects (ADAPTER-01 publisher schema fields; 255
unsequenced writable-file collisions) were already fixed in the 01:09–01:10
package revision — fresh runs in this session confirm publish_one.py --check
returns draft-ready / apply_ready with 0 unserialized collisions, and
coverage/check_package.py --offline returns ok with complete_package_verdict
true and 0 problems. The remaining gates were live materialization/readback and
queue coverage, which this pass performed.

## Publisher defects found and fixed during this pass

1. Queue packet size (all issues): make_packet embedded the full source_refs
   and design_refs lists (up to 55 KB) into the triage packet; repo-queue
   validate rejects payloads over the 16,384-byte hard ceiling. First apply
   (AUTH-01) created issue #9429 but failed queueing. Fix: evidence and
   context_refs now carry pointers (issue URL, parent, #9196, PR #9225, body
   path) plus requirement_ids and command_keys; a 15,000-byte hard guard
   raises before enqueue. Regression test: every canonical spec's packet
   measured (14 publisher tests, all passing).
2. GitHub issue-body limit (21 issues): bodies over 65,536 characters were
   rejected at create with "Body is too long" (deterministic; the affected
   keys were AUTH-02, CEN-01, CLI-01, CLI-05, DEP-02, DOC-01, DSP-05, DSP-06,
   HYD-05, HYD-06, HYD-07, HYD-08, PRI-03, REL-01, REL-03, REL-04, REL-05,
   REL-06, REL-07, REL-08, REL-09 — exactly the 21 files over the limit).
   Absence was proven before any retry: the 70 successful creates occupy
   issue numbers 9429–9498 with zero gaps, each failed row recorded
   matches: [] from the publisher's complete marker search, and an
   independent body-marker search counted 71 hits (70 issues + epic #9196).
   Only then were the 21 operation rows reset by the parent. Fix: the
   "Curated requirement trace" section of each oversize body was compacted
   to requirement ID + status + occurrence IDs, with the full
   per-occurrence citations and outcome text preserved in
   census/requirements-crosswalk.jsonl in this directory. New regression
   test asserts every canonical body fits the 65,536-character limit.
   All publisher gates were re-run green after the change.

## Live results (verified by readback, not receipts alone)

- 91 canonical issues created: #9429–#9519 (contiguous), each a native
  sub-issue of its domain starter (#9198–#9209 set), each carrying marker
  pmo-9196:<KEY> and an exact body match to issues/bodies/<KEY>.md.
- 24 alias/composite rows were not duplicated as issues, per the manifest;
  CEN-03 reuses existing research issue #9241.
- Native blocked-by graph applied and verified: 245 edges (203 local
  hard edges from the manifest plus external blockers such as #9073/#9074).
- Queue coverage: every issue has a live triage task in
  repo-health-code-pilot at priority 30 (task ids 15891–16023 range;
  per-issue coverage was verified by the publisher's read-only coverage
  helper at enqueue time, and each index operation records its task_id).
- Independent live audit (this session, separate script): all 91 issues
  re-read — state open, title exact, marker present, body hash equal to
  the recorded readback hash, parent exact, blocked_by set equal to the
  manifest — 91/91 checked, 245 edges, zero failures. Sandbox-side
  cross-checks agree (marker search total 92 including epic #9196;
  spot readbacks on #9499/#9519).

## Honesty boundaries

- Coverage: 389/389 curated requirement IDs allocated; 215/215 command
  keys allocated; 3,571 source dispositions preserved. Design is not
  implementation: nothing here is deployed or live-verified behavior.
- Queue Table Improvements remains unlocated by name; #9241 stays the
  open research item and no equivalence with #3424 or #8579/#8602 is
  asserted.
- coverage/check_package.py --live remains a fail-closed stub by design;
  the live pass above was performed directly against the GitHub API and
  the queue coverage helper, as the stub's own text prescribes.
- Epic #9196 stays open for the program; this pass completes the design
  and issue-publication deliverables only.
