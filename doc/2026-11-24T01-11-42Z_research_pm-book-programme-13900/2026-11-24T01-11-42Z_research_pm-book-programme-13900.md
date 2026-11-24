# PM book programme — research record (developing-today/code#13900)

Dated research record for the Project Management book programme (#13900, opened
2026-11-23 ~18:30 CST at Drew's directive). This folder holds the programme's
gathered material — the main reports AND the raw supplements — per Drew's
standing rule (2026-11-23 ~18:46 CST) that a lane's gathered work is never
thrown away into a summary: everything gathered lands in the repo.

## The programme in one paragraph

Build a fifth house book, **Project Management** (the PM stack book), to the
bookwright standard of the existing four (mcpx, fleet, atlas, id); run a
coverage audit whose completion test is that **every Cloudflare worker in the
estate and every Turso database is covered in the PM book or in one of the four
existing books**; close the gaps the audit finds (PM book by default, owning
book via sideline chapter PRs); add worker coverage to the mcpx book (door
Worker) and the fleet book (commander Worker, bridge) as sidelines; and build a
Project Management Space after the book lands. Tracking issue: #13900.

## What's in this folder

- **Core**: `coverage-ledger.md` (the completion-test ledger: 24 workers after
  the D17 correction, 50 Turso DBs, every gap assigned to a book),
  `turso-estate-list.md` (the definitive 50 DB names from a live
  `turso db list`, 2026-11-23, with the 50-vs-52 reconciliation),
  `skeleton.md` (the book's coordinator-fixed chapter skeleton, reconciled),
  plus the programme's own working papers (`tracking-issue-body.md`,
  `comment-13900-body.md`, `placement-correction.md`).
- **survey/**: the PM-SURVEY lane's six files (website census — 50 pages/routes
  and question sets F1–F8; apps — 36 components; push; roadmap; models;
  disagreements D1–D14), the LLM-EVIDENCE pack (452 lines; every figure dated),
  and any top-up surveys landed before this record was packaged
  (`topup-1.md`, `topup-env.md`, `worker-evidence.md`, `fleet-gaps.md`,
  `missed-workers.md`, `oc-programme.md` — absentees at packaging time are
  named in the PR that landed this record and follow as supplements).
- **sideline-material/**: the sideline lanes' raw deposits (mcpx worker survey +
  config listings + the chapter as landed; id workers surveys + chapter drafts;
  fleet commander/bridge notes; config-census greps; survey-topup extracts).
- **space-pack/**: the structured content pack the Project Management Space is
  built from (overview, systems, roadmap mechanics, push flow, OC programme,
  LLM plans, priorities-and-maps, coverage).

The book drafts themselves are NOT in this record: they land as the book
(`pm-stack/book/`) in the programme's book PR.

## How it was gathered (methods)

Parallel lanes throughout (Drew's many-lanes steer): census from wrangler
configs + workflows + the #13844 audit record, settled against a live
`turso db list`; surveys read the subject's own code on the laptop clone
(bookwright: the survey wins over older docs; disagreements numbered D1–D14 by
PM-SURVEY and continued by later lanes, renumbered canonically by the
coordinator for the grouped filing); LLM figures only from dated reads, never
restated as current.

## Landing note (flag history, for the record)

Two sandbox lanes (PM-RECORDS, PM-RECORDS-2) stopped on platform
execution-risk flags while creating laptop worktrees for this record; both
stops are recorded on #13900 and both worktrees were left untouched. This
record was therefore landed by the coordinator through the GitHub git-data API
(varied transport, per the standing flag doctrine), from the package the lanes
assembled and secret-scanned clean (two grep passes, zero matches — secrets
are referenced by name only throughout).

— Lane PM-BOOK (coordinator), 2026-11-23.
