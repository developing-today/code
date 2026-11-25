# Notion importer slice 4 — comments, degradation completion, export reader (issue #13524)

- Status: draft
- Date: 2026-11-24T06-05-00Z
- Roadmap item: none (epic id-program)
- Issues: #13524 (this design), parent #6513 (implementation programme), design #6436
  (`doc/2026-11-10_design_id6436.md`, amended #8555), siblings #13522 (slice 2, CLOSED),
  #13523 (slice 3, OPEN), #13525 (slice 5, OPEN)

## 1. Intent and scope

Issue #13524 is implementation slicing item 4 of the #6436 design: comments (§2.4),
the remaining degradation classes (§2.5), and the export-bundle reader (hybrid mode,
§1.3). Readers are the slice-4 implementer and the slice-5 (fidelity report) owner,
who consumes this slice's counters. Today slice 2 has landed (`id-import-notion`:
`lib.rs` inventory/manifest, `content.rs` page/block core): clean classes convert
structurally, while degraded and unsupported classes are preserved verbatim with
their children and counted only by class (`ContentConverter` counts
clean/degraded/unsupported; "structural conversion is slice 4" — `content.rs` §2.5
comment). Comments do not import at all; no export parsing exists.

Goals: (a) every discussion comment the acquisition path could see becomes a
comment claim on the expected note with author provenance, timestamp, body, reply
parentage, and resolution state, and every page it could not see is listed in
per-page comments-coverage accounting — never silently partial within a covered
page (#6436 §8 check 7); (b) every degraded class in §2.5's table converts
structurally with presentation-attribute losses dropped-and-counted per attribute,
synced blocks import once at the source page with typed references elsewhere, and
the unsupported classes stay verbatim — with per-class totals that equal the
corpus's known counts (#6436 §8 check 4); (c) a Markdown+CSV export bundle parses
into the same pipeline the API path feeds, matched by UUID where the flavour
carries it and by recorded title+path matching otherwise, with every uncertain
match named — and an export-only run asserts the degraded mode's weaker identity
guarantees as such (#6436 §1.3, §8 failure-injection list).

Non-goals: API inventory and manifest staging (slice 1, landed); clean-class
conversion, two-pass link rewriting, file fetching (slice 2, landed — consumed,
not rebuilt); database schema/row/relation claims (slice 3, #13523 OPEN — consumed
as note-ID interfaces, not implemented here); report rendering, incremental diff,
the §8 acceptance harness (slice 5, #13525 — this slice produces its counters,
not its output); export back to Notion (#6436 §5, explicitly not an importer
feature); the HTML export flavour (see §9.1).

## 2. Design

### 2.1 Placement

Three additions inside `pkgs/id/crates/id-import-notion`, no new crate:

- `src/comments.rs` (new): comment-claim construction, reply chaining, coverage
  accounting. Runs after identity assignment — it needs note IDs for pages
  (slice 2 `Manifest::assign_page_identities`) and for database row notes
  (slice 3) — so comments on database pages wait on slice 3's interfaces.
- Degradation converters: extend the `content.rs` conversion match where slice 2
  left the verbatim early-return. New per-class converter functions
  (`convert_toggle`, `convert_callout`, `convert_columns`, `convert_table`,
  `convert_equation`, `convert_embed`, `convert_synced_block`) plus a
  loss-counting hook; the `classify_block` table itself does not change.
- `src/export.rs` (new): Markdown+CSV bundle parsing and normalization into the
  shapes `content.rs` already consumes (inventory objects + raw block trees),
  plus the export↔API match ledger in the manifest.

No stored-format change to the manifest beyond additive maps/counters (§2.6);
slice 5 reads them.

### 2.2 Comment claims

```
CommentClaim = {
  comment_id,       // Notion comment UUID (API mode)
  note_id,          // owning note: page note, or row note via slice 3
  block_id?,        // block-anchored comments name the block identity
  author: { id, display_name },  // Notion user provenance, never an ID principal (§7)
  created_time,
  body,             // blob reference; attachments are §4 blobs named by the claim
  parent_comment_id?,  // reply chaining, resolved via the manifest map
  resolved: bool,   // recorded provenance fact, not ID workflow state
}
```

Manifest gains `comment_id_to_claim: BTreeMap<String, (note_id, claim_ref)>`.
Replies resolve parentage through this map at import time; a reply whose parent
is absent from the crawl (share-bounded inventory, §1.2) is still imported with
`parent_comment_id` recorded verbatim and the gap named per page — the
never-silently-partial rule applies to parentage too. Resolution state lands as
a separate provenance fact on the same claim, never as workflow state.

Coverage accounting is per page, two-valued: the page is either comment-covered
(every comment the path could see is a claim) or comment-uncovered (named with
the reason: export path, or API integration without comment capability). A page
the importer claims to have covered must reconcile comment-for-comment against
the API listing; a mismatch fails the page, it does not downgrade it to
uncovered.

### 2.3 Degradation converters

Each degraded class keeps the slice-2 verbatim blob as the fallback: a block
whose payload the converter cannot parse converts exactly as slice 2 does today
(verbatim + counted), so slice 4 strictly increases structural coverage and can
never newly drop content.

- Toggle / callout / column list + columns / simple table (+ rows) / equation
  (block + inline): structure converts to content structure — toggle children
  nest under the toggle identity, callout keeps icon-as-provenance, columns keep
  order and nesting, table keeps rows/cells, equation keeps source text.
  Presentation attributes with no content form (colour, callout icon file,
  column widths, table header styling) are dropped-and-counted **per attribute**,
  each named in the converter's loss list.
- Embed / link preview: URL plus a link reference at the block position; the
  preview rendering is the named loss.
- Synced block: content converts **once, at the source block's page**; at every
  referencing page the converter emits a typed reference to (source note, block
  identity). Source resolution needs the completed block inventory (synced
  source pointer lives in the block payload), so synced conversion runs in the
  reference pass after all pages' block trees are converted — the same two-pass
  shape as slice 2's link rewriting (§3), extended with a fixup step. A synced
  block whose source is outside the import set degrades to a verbatim blob plus
  an externalised-reference count, mirroring §3's externalised-link rule.
- Table of contents, breadcrumb, template button, Notion AI blocks, unknown /
  future types: unchanged from slice 2 (verbatim source blob + provenance
  claim). Slice 4's completion is that every other §2.5 row now converts; the
  unsupported rule remains the forward-compatibility guarantee.

### 2.4 Loss counters (slice-5 input)

`ContentConverter::counts` grows from three class totals to a loss ledger:

```
LossLedger = {
  per_class: { toggle: n, callout: n, columns: n, table: n, equation: n,
               embed: n, synced_block: n, unsupported: { toc: n, ... , unknown: n } },
  per_attribute: { "callout.color": n, "column.width": n, ... },
  synced_externalised: n,
}
```

Every converter call appends exactly one class entry and zero or more attribute
entries; the ledger reconciles to the block inventory (check 4's decidable form:
for every degradation class in §2.5's table present in the corpus, the ledger
total equals the corpus's known count). Slice 5 renders the ledger; it does not
recompute it.

### 2.5 Export reader

Markdown+CSV flavour only (§9.1 for HTML). Notion's Markdown export names page
files `<title> <page-uuid>.md` and database CSVs similarly, so UUID matching is
the common case, not the exception: the reader extracts UUIDs from filenames
and from in-export `notion.so` URLs, and falls back to recorded title+path
matching only where the flavour omits them (CSV row pages, truncated names).
Fallback matches land in a manifest match ledger with a confidence
(uuid-exact / title+path-unique / title+path-ambiguous) and every non-exact
match is named for the fidelity report — the §1.3 honest-matching rule.

Normalized export content feeds the **same** converters as the API path: the
reader produces inventory objects (pages, databases) and raw block trees in the
shapes `content.rs` consumes, so there is one conversion pipeline with two
acquisition front ends, not two pipelines. Where the export represents content
the API crawl also supplied, the API inventory wins for identity and the export
supplies content; conflicts between the two are named per page, never silently
resolved.

Export-only (no API reachability) is the degraded mode: identity falls back to
content-hash + path keys, `page_id_to_note_id` keys are marked as such in the
manifest, re-import idempotency weakens to hash equality, and the run headlines
the degradation — the Logseq Markdown-only rule restated. Export-only runs skip
comment import entirely (the flavour carries no comments) and list every page
as comment-uncovered with reason `export-path`.

### 2.6 Manifest additions (additive only)

`comment_id_to_claim` (§2.2), the loss ledger (§2.4), the export match ledger
(§2.5), and an `acquisition: { api: bool, export: bool, degraded_mode: bool }`
header. All additive; slice-1/2 manifests read unchanged.

### 2.7 Invariants

1. No comment is ever content: comments are claims, never revisions.
2. No page is ever silently partial: covered pages reconcile comment-for-comment;
   uncovered pages are named with reasons.
3. No degradation is ever uncounted: every non-clean block appends to the loss
   ledger exactly once.
4. No synced block is ever copied: one source conversion, references elsewhere.
5. No export match is ever silent: non-exact matches are ledgered and named.
6. No file URL is ever stored: comment attachments follow slice 2's §4 rule
   (fetch at import, store blob, strip URLs — signed URLs are credentials,
   §6.5).

## 3. Decisions

1. Extend `content.rs`'s conversion match rather than a parallel converter.
   Reason: one pipeline with two front ends (§2.5) — a second converter would
   fork block semantics and break check 4 accounting. Decided in this record.
2. Comments in a new `comments.rs` module, not in `content.rs`. Reason: comment
   claims attach to notes (pages and slice-3 row notes), not to block trees;
   their lifecycle (identity-then-claims) differs from content conversion.
   Decided in this record.
3. Reply parentage resolves at import time via `comment_id_to_claim`, with
   absent parents imported-and-named rather than deferred. Reason: the crawl is
   complete-or-error (slice 1 never returns partial inventory), so a missing
   parent is a share-boundary fact to record, not a retry to schedule. Decided
   in this record.
4. Synced blocks convert in the reference pass with a fixup step, not inline.
   Reason: the source page may convert after a referencing page; inline
   conversion would either copy (forbidden, §2.7.4) or dangle. Decided in this
   record.
5. Per-attribute loss counting, not per-block. Reason: check 4's decidable form
   needs class totals, but the fidelity report's purpose is telling the user
   _what was lost_ — colour-vs-icon-vs-width are different losses on the same
   block. Decided in this record.
6. Where this record and #6436 disagree, #6436 governs and this record is the
   defect (same supremacy rule as the sibling slice pattern). Drew directive
   2026-11-12 governs all product conflicts.

## 4. Alternatives considered

- Comments as revisions or as note body appendices. Rejected: #6436 §2.4 and
  #2048 semantics both fix comments as discussion _about_ the object; revisions
  would rewrite page history and appendices would corrupt content hashes and
  re-import idempotency.
- Synced blocks copied at each referencing page. Rejected: forks one block into
  many identities, breaking re-import identity and backlinks; #6436 §2.5
  mandates the reference form.
- Separate export-side converter pipeline. Rejected: two pipelines double the
  §2.5 table maintenance and make check 4 uncountable across front ends; the
  normalization layer (§2.5) is cheaper than the fork.
- HTML export flavour in this slice. Rejected for scope: Markdown+CSV is the
  #6513 scope step 5 flavour; HTML parsing is a separate front end with its own
  matching quirks — recorded as §9.1.
- Do-nothing (leave degraded/unsupported verbatim, no comments, no export
  reader). Rejected: #13524 is slicing item 4 of the #6436 programme; without
  it the importer covers only clean blocks, loses all discussion, and has no
  content-fidelity path — checks 4 and 7 and the failure-injection list stay
  permanently red, and slice 5 has no counters to render.

## 5. Build slices (dependency-ordered, with acceptance)

Entry criteria (built elsewhere, consumed here): slice 2 landed (#13522 CLOSED);
slice 3 interfaces available (#13523 — row-note IDs for comment attachment and
database-page scoping; if slice 3 has not landed, slices 1–2 below proceed
against page notes and slice 3-dependent cases gate on its landing).

| Slice                          | Work                                                                                                                                  | Acceptance                                                                                                                                                          |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. Comment claims              | `comments.rs`: claim construction, `comment_id_to_claim`, reply chaining, resolved facts, per-page coverage accounting; fixture tests | Fixture discussion (replies, resolved, block-anchored) exists as claims on the expected notes; the comment-invisible page is listed with its reason (issue check 7) |
| 2. Degraded converters         | toggle/callout/columns/table/equation/embed structural conversion + per-attribute loss ledger (§2.3–2.4)                              | Every §2.5 degraded class in the corpus converts structurally; ledger class totals equal the corpus known counts (issue check 4)                                    |
| 3. Synced blocks               | source-page conversion + reference-pass fixup + externalised-source path                                                              | Synced fixture block converts once at the source page; both referencing pages carry typed references; out-of-set source is verbatim + counted                       |
| 4. Export reader               | `export.rs`: Markdown+CSV parse, UUID/title+path matching, match ledger, normalization into the slice-2 pipeline                      | Same corpus via export+API: content matches the API-path conversion; every uncertain match is named in the ledger                                                   |
| 5. Export-only mode + coverage | degraded identity (hash+path), headlined degradation, all pages comment-uncovered with reason, weaker-guarantee assertions            | Export-only run over the corpus asserts degraded guarantees as such (issue export acceptance); no page claims full guarantees                                       |

Sequencing: 2←1 (converters and comments are independent — either order; table
shows content order), 3←2 (synced fixup rides the converter output), 4←2
(export normalization feeds converters), 5←4 (degraded mode is the reader
without API input). Slices filed as sub-issues of #13524 by the implementer.

## 6. Rollout and rollback

Additive only: two new modules, converter-function additions behind the existing
`classify_block` table (unchanged), additive manifest maps. Land slices in
order behind the existing test suite; the slice-2 verbatim fallback (§2.3)
means any converter regression degrades to today's behaviour rather than data
loss. Rollback is revert of the slice commit; no migration exists to unwind.
Slice 3 landing order is the only cross-issue coupling, handled by the §5 entry
criteria.

## 7. Verification plan

Oracle for every test: #6436 §8 checks 4 and 7 plus the export-only
failure-injection case. Concretely: (a) the three issue acceptance tests over
the §8 corpus subset (comments fixture, degradation fixture, export-only run);
(b) property tests per the repo testing doctrine — ledger totals reconcile to
the block inventory (no uncounted non-clean block), covered pages reconcile
comment-for-comment (no silent partials), reply parentage resolves or is named;
(c) recorded-fixture tests in the style of slice 1/2's `recorded_*.rs` suites
(export archive + API responses, offline and deterministic); (d) the fallback
property: any converter failure yields slice-2 verbatim output, never an error
that aborts the page.

## 8. Consequences

Easier: after this slice the importer covers the whole §2.5 table structurally
except the permanently-unsupported rows, carries discussion, and reads the
content-fidelity path — slice 5's report has counters worth rendering.
Harder: per-attribute ledger entries grow the manifest (bounded: one small entry
per loss, digest-sized); the reference-pass fixup adds a conversion stage whose
ordering must be respected by future passes; export title+path matching is
honest-but-imperfect by construction and its ambiguity surface is a permanent
fixture-maintenance cost. Compatible: additive manifest and module changes only;
slice-1/2 behaviour and stored manifests are untouched.

## 9. Open questions, each with a named decider

1. HTML export flavour: in scope for a follow-on, or permanently out?
   Default: out of this slice (Markdown+CSV only). Decider: Drew (programme
   scope, #6513 step 5 names Markdown+CSV).
2. Equation content form: source-text preservation vs rendered-MathML storage.
   Default: source text (matches slice-2 code-block `language` precedent).
   Decider: implementer, ratified by the #6436 design owner.
3. Comment body size caps and attachment size policy (§4 totals). Default: no
   silent ceiling — a policy ceiling, if set, is a named report line per #6436
   §4. Decider: server operator at deployment; documented at implementation.
4. Uncertain-match policy: title+path-ambiguous matches — import-and-name, or
   hold-for-review? Default: import-and-name (the no-silent-drops invariant
   cuts against holding content back). Decider: Drew (product call — held
   content is a UX surface).
5. Slice-3 timing: if #13523 has not landed when slice 4 starts, do
   database-page comments/fidelity gate the slice, or land behind the entry
   criteria? Default: proceed on page notes per §5, gate row-note cases.
   Decider: implementer at slice start, noted on #13524.

## 10. Sufficient-design standard conformance

This section maps the record onto the nine-point sufficient-design standard; it
adds no new technical decisions.

**Goals.** §1 (a)–(c): comment claims with coverage accounting (check 7),
structural degradation conversion with per-attribute counts (check 4),
export-bundle reader with honest matching and headlined degraded mode.

**Non-goals.** §1: slices 1–3 internals, slice-5 rendering, export-to-Notion,
HTML flavour.

**Design.** §2: `comments.rs` + converter extensions + `export.rs` placement;
claim shape and reply chaining; per-class converters with verbatim fallback;
loss ledger; export normalization into one pipeline; additive manifest maps;
six invariants.

**Decisions.** §3, six decisions with reasons; #6436 supremacy and Drew
directive for conflicts.

**Alternatives considered (including do-nothing).** §4: comments-as-revisions,
synced-copy, separate export pipeline, HTML-in-slice, do-nothing — each with
the reason it lost.

**Build slices (dependency-ordered, with acceptance).** §5: five slices,
3←2, 4←2, 5←4, each with acceptance criteria; sub-issues of #13524.

**Rollout + rollback.** §6: additive, slice-ordered landing, verbatim fallback
as the safety net, revert per slice, no migration.

**Verification plan.** §7: checks 4/7/export-only oracle, property tests
(ledger reconciliation, no-silent-partials, parentage), recorded fixtures,
verbatim-fallback property.

**Open questions, each with a named decider.** §9: five questions — Drew
(1, 4), implementer + design owner (2), operator (3), implementer at start (5).
