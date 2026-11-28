# Sender notes — queue task 14440

Sent by Muse session `bfa19dac-bbe0-42f2-b441-230beac266f8` via send_to_opencode.py (#8210).
Pass-off reason: Live acceptance of the #8210 Muse->opencode send: bounded effort/s dedupe (one comparator, two call sites) fits a single persistent opencode session; sender lane is a one-shot validation session ending after the send.

---

SENDER NOTES — Muse lane M (repo-worker-code, session bfa19dac-bbe0-42f2-b441-230beac266f8) handing task 14440 / issue #6977 to you via the new Muse->opencode send (#8210). This is the send path's live acceptance run; the task itself is a real small fix.

WHAT TO DO
Issue #6977: the #3719 total-order comparator for mcpx stats exists as two verbatim copies — one in eventstore/stats.go and one in logstore/stats.go (mcpx tree; locate the exact paths first, they are under the mcpx module). Extract the comparator into ONE shared location both packages can import, point both call sites at it, and delete the duplicate. Acceptance: exactly one comparator implementation remains; both packages use it; ordering semantics byte-for-byte unchanged; the mcpx eventstore + logstore stats tests pass.

CONTEXT
- The danger the issue names: a future tie-break change applied to only one copy silently makes the two stores disagree on ordering. Your dedupe removes that class.
- Choose the shared home by the repo's existing layout (a package both stores already depend on, or the lower-level one of the two if it does not create an import cycle). Do not invent a new top-level package if an existing shared one fits.

WARNINGS
- Do NOT change any comparison semantics, field order, or tie-breaks while moving the code. Move first, verify tests, then stop.
- Work ONLY in the provided worktree (branch queue/14440-6977-comparator-dedupe). Primary clones stay on main + clean.
- SEND_NOTES.md in the worktree root is this same note file; it is not part of the fix — do not commit it.
- When the fix is verified, open the PR per the normal lane rules and complete queue task 14440 through the claimant machinery with the PR as evidence.
