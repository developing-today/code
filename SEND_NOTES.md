# Sender notes — queue task 20361

Sent by Muse session `12e6fc7e-c815-4852-bdaa-daceb0ac6938` via send_to_opencode.py (#8210).
Pass-off reason: Dispatch-eligible per runbook 1d: self-contained second_pass packet with a recorded resume plan and precise done-state (all nine conformance legs green on main); dedicated opencode session grinds the suite loop in the existing worktree while the Muse lane supervises

---

Task 20361 (second_pass remainder fix for issue #8922, mcpx conformance). RESUME from the recorded progress — do not restart:

- Existing worktree: /home/user/git/fleet/20361 on branch queue/20361-mcpx-conformance, clean at 7dcb69952d when parked; rebase/refresh onto current origin/main first (PR #10985 merged 2026-11-17T05:06:04Z — the writePolicy=allow harness fix; do NOT revert or re-litigate those settings).
- Reproduce with the pinned official suite ref 7169291ec0b68eb370fddcd9947313ab0d5e4156: LEGS='<leg>' bash scripts/conformance.sh with CONFORMANCE_DIR at the pinned ref, per family, then all nine legs.
- Diagnose/fix, in this order: (1) elicitation refusal loop — routes_proto.handleAskAnswers ignoring problems returned from /v1/ask/{id}/answers and daemonAsker.Reply; a refused answer must not be re-pushed until AskRounds; ADD a regression test at the layer you change (mcpserver ask loop and/or daemon ask-answers route). Also the unasked sep1034-defaults / sep1330-enums scenarios. (2) SEP-2322 input_required first-call result path (internal/mcpserver ask.go/state.go/server.go envelope). (3) tasks Mcp-Method header handling changing tool result content. Separately confirm client legs client-2025-11-25 and client-2026-07-28 against the workflow gate.
- Hard constraints (packet acceptance): NO conformance coverage weakened — no scenario skipped, no EXPECT loosened, no check deleted without a recorded reason on #8922. Record the root cause of EACH family on #8922 as an agent-note:v1 comment BEFORE its PR opens.
- Deliverable: PR in developing-today/code with 'Fixes #8922' in the final PR; acceptance is every leg of the mcpx conformance workflow passing its gate on main after merge (got == EXPECT, non-OAuth failures == 0, all nine legs).
- Heavy builds on the laptop go through ~/tools/fleet-build-lock. Never work in the primary checkout. Checkpoint the queue task as you go; if you cannot finish, checkpoint exact state + next step and park — never leave the claim silent.
