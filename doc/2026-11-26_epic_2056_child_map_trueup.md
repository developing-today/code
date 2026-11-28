# Epic #2056 — ID platform expansion: child-map true-up (2026-11-26)

Fresh GitHub API state after the design reviews of #2036 and #2039 on
2026-11-26. This updates the 2026-11-25 census in
[`2026-11-25_epic_2056_child_map_trueup.md`](2026-11-25_epic_2056_child_map_trueup.md).

## Measured state

- Original named children #2026–#2054: **29 of 29 CLOSED**, up from 27 of 29
  in the 2026-11-25 census. The last two design issues are closed with their
  conclusions recorded: #2036 and #2039.
- Follow-on batch #2057–#2066: **10 of 10 CLOSED** (unchanged).
- All six v1 synthesis issues are closed: core #2028, messaging/relay #2036,
  currencies #2043, serverless #2046, chat #2051, and boards #2054. The
  Zettelkasten design #2048 is also closed.

## Design conclusions and live follow-ons

### #2036 — messaging/relay protocol v1

Closed after review on 2026-11-26. The design is merged in PR #3833; all four
design phases (#3794–#3797) are closed, and the five-node bridge PoC is merged
in PR #13454. The review decision is recorded on #2036. Follow-on work remains
separate and visible:

- #5534 OPEN — implementation of the protocol, currently `status:in-review`.
- #5674 OPEN — wire format and transport adapters design, `status/triaged`,
  decomposed into its own design tickets. Its postcard schema and versioning
  records landed in PRs #10668 and #8199.

These are follow-on implementation and expansion issues; they do not undo the
written v1 conclusion in #2036.

### #2039 — match attestation and audit v1

Closed after review on 2026-11-26. The corrected design is merged in PR #3807,
the sufficient-design review in PR #14974, and implementation phases 1–7 in
PRs #15319, #15358, #15363, #15376, and #15374. Combined remote verification
run [45777590792](https://github.com/developing-today/code/actions/runs/45777590792)
completed successfully. The design's phase 8 (observed metadata / L3) is
explicitly optional and non-blocking. The review decision is recorded on
#2039.

## Epic acceptance state

- Criterion 1, original LIST 6–15 children reach a written conclusion or
  synthesis: **29/29 original children are closed**; the two previously open
  design children now have review decisions and closed states.
- Criterion 2, v1 syntheses for core, chat, boards, hosting and currency:
  **complete**; all five named designs are closed with merged records.
- Criterion 3, stalled children named with their blocker: the only live
  follow-ons from the two design parents are #5534 and #5674, listed above
  with their current state. Neither is waiting on an owner decision.

The epic can close once the remaining follow-on work it tracks is complete;
this census does not close #5534 or #5674.
