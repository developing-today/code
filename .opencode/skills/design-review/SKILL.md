---
name: design-review
description: Exhaustive internal-soundness review of design records — gaps, dependency order, head prioritisation — with a per-design ledger. Use when auditing the design corpus or a programme's designs.
---

# Design review (AUDIT-7 Lane 5 method, 2026-11-24)

Judge a design's **internal soundness** only: gaps in the design itself,
whether it has a dependency order and the order makes sense, and whether
its heads (leading/first slices) are prioritised appropriately. Whether a
design is *sufficient* at all is a different audit (the sufficient-design
standard); refer outright failures there instead of duplicating.

## Procedure

1. **Enumerate the design home.** In the code repo that is `doc/`: every
   dated record whose kind token is `design`, `decision`, or
   `architecture`, plus undated `*design*` files, plus design records in
   subdirectories (`decisions/`, `living/`, `platform/`, `pm-stack/`,
   `fleet/`, `agents/`, `notes/`, `archive/`). Check the other repos for
   design homes too — do not assume.
2. **Full-text structural pass** over every record (never title-skim):
   word count, headings, tracking-issue references, slice/phase/stage
   reference count, dependency vocabulary, priority vocabulary,
   acceptance/verification vocabulary, risks/alternatives vocabulary.
3. **Hand-read every flag class in full**: no tracking reference;
   multi-slice with no dependency vocabulary; thin (<300 words); plus
   every major programme regardless of flags. Heuristic flags are
   hypotheses, never verdicts — in the 2026-11-24 run, 15 of 16 ordering
   flags were vocabulary misses (order carried by stages, phases,
   protocol steps, or a parent epic's breakdown), and all 3 thin records
   were complete small designs.
4. **Verify tracking issues against the API** (`fleet gh api`); never
   trust a brief's list. Check whether each major design's record of
   record is a `doc/` record or only an issue body — the latter is a
   finding.
5. **File as you go**: class findings as one issue with the full list;
   per-design findings as comments on the design's own tracking issue;
   sound verdicts live in the report ledger.
6. **Report** as a dated record in `doc/` with a README line, the full
   per-design ledger, counts, and the prompt verbatim when the review
   was commissioned by one.

## Judgement notes

- A design's "steps" are often protocol/runtime steps, not build slices —
  ordering is N/A for those; say so rather than flagging.
- A decision record correctly has no build order until the decision
  lands; check instead that it names (or should name) its gating
  decision point.
- A good head is smallest-surface, already-specified, or
  highest-value-at-lowest-risk — and the record says why. Absence of
  that sentence is a note, not a defect.
- Acceptance often lives on the tracking issue; its absence from the
  record is a convention gap (suggest a "Verified when:" line), not a
  per-design defect.
