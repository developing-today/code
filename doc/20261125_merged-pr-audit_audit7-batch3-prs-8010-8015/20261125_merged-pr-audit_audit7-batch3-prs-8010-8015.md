# AUDIT-7 merged-PR audit — batch 3 continuation (#8010–#8015)

## Brief

Continue the AUDIT-7 Lane 4 batch 3/14 audit for `developing-today/code`: read each selected PR's body and complete diff, compare it with the spawning issue and comments, check later merged work for any remainder, apply one Lane 4 outcome label, and record the ledger on #13879. The five payload entries at this boundary are #8010, #8011, #8013, #8014 and #8015.

## Method and evidence

- Verified all five payload entries resolve to merged PRs. Read full PR bodies, changed-file responses and PR comments; counts are #8010 (1 file/4 comments), #8011 (2/3), #8013 (5/3), #8014 (4/5), and #8015 (1/4). No inline review threads were returned for this slice.
- Read source issue bodies and comments: #7856 (4 comments), #6469 (18), #7071 (5), and #7949 (5). The full #1790 source issue evidence (31 comments) and PR #8013 body are captured in the preceding #7984–#7998 slice; re-read the issue's closure evidence and PR #8013's description of the remaining slices here.
- #8010 removes two dangling §8 references from the #2039 attestation design and closes #7856.
- #8011 makes `SignedReplicationEnvelope::sign` refuse every envelope-local shape that `ReplicationTracker::accept` would reject; receiver-state checks stay receiver-only per #6469's scope.
- #8013 completes #1790 slices 2–3: UI wiring for tooltip modes, pin/minimize and restore, plus client/server lifecycle logging and liveness-based fade. Together with slice 1 in #7993, it closes the remaining acceptance.
- #8014 adds delegation-grant scenarios that reach `Engine::void`, and a planted mutation that the test runner catches; it closes #7071's test-oracle gap.
- #8015 corrects the pkarr and cold-store test-suite registry entries and stale pkarr reason; venue wiring remains a separate issue (#6093), outside #7949's registry-correction scope.
- Applied and read back `audit:complete` on all five PRs.

## Per-entry ledger

| Payload number | Spawning issue | Verdict | Finding |
|---:|---:|---|---|
| #8010 | #7856 | `audit:complete` | Replaces two dangling §8 references with the explicit Phased implementation plan section; the source issue closes with the correction. |
| #8011 | #6469 | `audit:complete` | Adds envelope-local validation at signing for all receiver-refused shapes listed in the issue, with eight negative shapes and a valid control. |
| #8013 | #1790 | `audit:complete` | Implements the remaining UI and lifecycle slices after #7993's state-machine slice; the source acceptance is complete and #1790 closes. |
| #8014 | #7071 | `audit:complete` | Adds strong/weak removal and causal-past scenarios reaching `Engine::void`, plus a planted inverted-gate mutation to prove the oracle catches the defect. |
| #8015 | #7949 | `audit:complete` | Adds omitted pkarr and cold-store test paths and corrects the stale pkarr reason; the registry-correction issue closes, with venue wiring tracked separately by #6093. |

## Slice state

Five payload entries were processed: five actual merged PRs audited and labelled, with no invalid references. All five are complete. Across all slices to this point, **201 valid PRs are audited from 205 processed payload identifiers**; #7667, #7668, #7693 and #7694 are issues rather than PRs. There are 45 payload identifiers remaining; next is #8016. Batch 3 remains partial.

Tracking progress comment: [#13879 comment 7161400471](https://github.com/developing-today/code/issues/13879#issuecomment-7161400471).

## Raw supplements

`raw/` contains all five PR bodies, complete changed-file responses, PR comments, issue bodies/comment pages and label readbacks. The #1790 source issue comments and its slice-1 PR #7993 evidence are in the preceding `doc/20261125_merged-pr-audit_audit7-batch3-prs-7984-7998/raw/` artifact.
