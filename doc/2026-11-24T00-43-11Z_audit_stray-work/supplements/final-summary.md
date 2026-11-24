## Slice 2 + final summary — Lane 7 stray-work audit (2026-11-23)

### Trees built (all verified by readback: parent sub_issues list + child labels)

Batch 1 (20, claim-checked individually, 0 claims):
- epic: main-broken #8976 ← #13849, #13843, #13619
- epic(id): maintenance #12462 ← #13864, #13821, #12050
- epic(fleet): maintenance #12460 ← #13078, #13073, #12498, #12235, #5504
- PM platform tracker #13504 ← #13503, #13502, #13497, #13496, #13495
- epic: scheduled tasks #8979 ← #12664, #12467, #12249, #12250

Batch 2 (14, claim-checked per item in-script, 0 skips):
- epic: scheduled tasks #8979 ← #12260, #12133, #11355, #11166, #11165, #10937, #10628, #10538, #10387, #9404
- epic: design register #8986 ← #10389, #10390, #10392, #10393

**Total: 34 orphaned issues linked into parents; 102 label assignments added across them (every child got `stray-work` + its parent's `epic/*` and, where the taxonomy has one, `primary-parent/*`). Titles/descriptions corrected: 0 — none of the items read in full had a wrong or empty title or description; the hygiene gap is linkage/labels, not prose.**

### Labels created (all four repos, verified by creation readback)
`stray-work`, `stray/needs-parent`, `audit-7` — in developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet. (Note: label writes used `fleet gh api` POST /labels because `~/tools/gh-post` has no label verb; flagged to the coordinator.)

### Report
Dated report + full log + verbatim brief + repeatable procedure: `doc/2026-11-24T00-43-11Z_audit_stray-work/2026-11-24T00-43-11Z_audit_stray-work.md`, README line added, PR #13904 merged — merge SHA `bf514e56dc0ef0cc71c977191f35d774588bdc5f` (verified by a second read of the PR).

### Trends (headline; full evidence in the report)
1. Forks are fix streams: "fix" in 66% of opencode and openchamber window titles vs 23% in code.
2. kind/bug is the largest kind everywhere (code 1,900; openchamber 646; opencode 630).
3. Main-broken CI failure is a permanent generator (745 CI/Actions titles, 343 flake titles in code) whose filings land parentless despite epic #8976 existing.
4. Quota/routing fixes recur in both forks (249 + 201 titles) — the top convergence candidate: one shared implementation.
5. Generated recurring work (scheduled reports ×14, design-mill passes ×7) lands parentless by construction — fix at the generator, not by sweeps.
6. 95% of code PRs carry no labels (6,102/6,443); issue labelling is 96% complete — enforcement belongs at PR creation/merge.
7. dezren39/fleet's own tracker is nearly bare (15 labels, 106/113 unlabeled); its work is tracked here in code.

### Remaining slices — BLOCKED on the queue broker, packets preserved
Slices A (521 text-linked open code issues), B (~49 remaining orphans + needs-parent triage), C (80 open fork/fleet items) were prepared as `feature_slice` packets (kind `feature_slice`, `payload.slice` A/B/C, dedupe keys `audit7:lane7:feature_slice:code:13874:A|B|C`) for queue `repo-health-code-pilot`, but every enqueue attempt failed: `repo-queue: broker: OSError: proxy closed during CONNECT` (one attempt timed out), persisting after `~/tools/init-ensure.sh` and a successful laptop-ssh check. The packet files are preserved with the lane's working papers (`~/workspace/lane7-audit7/packet-A.json`, `packet-B.json`, `packet-C.json` on the Muse sandbox) and their full text is specified in the report; the coordinator can enqueue them verbatim when the broker is back. Premise + singular-ownership check was done before preparing them: this issue (#13874) is the only stray-work audit, and slices A/B/C cover disjoint remaining sets.

### Per-batch counts
- Examined (census, metadata + classification): 18,594 items in window; 1,199 open code issues classified from full bodies; 612 unparented analysed for candidate parents; 83 orphans reviewed.
- Read in full incl. comments and claim-checked: 34 edited items (+ parents #8976/#12462/#12460/#13504/#8979/#8986).
- Linked into a parent/epic: 34. Labels added: 102 assignments on issues + 9 labels created (3 × 4 repos = 12 creations, code's `audit-7` confirmed new). Titles/descriptions corrected: 0.
