# Decision: KDL presentation surfaces — export, hand upload, agent statements

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5910](https://github.com/developing-today/code/issues/5910),
  "id: design: KDL presentation surfaces (from #5688)".
- **Parent design:** [#5688](https://github.com/developing-today/code/issues/5688)
  ("KDL as human presentation format + triples + ANSI graph SQL"). Its
  §1.4 names the three surfaces this record settles, and §2.2 names the
  pipeline they must share. The parent's decisions — KDL as the human
  presentation format, atomic triples as the canonical data model,
  ANSI SQL/PGQ as the query model, postcard as the wire format — are
  **not re-decided here**.
- **Inputs cited:**
  - [#5906](https://github.com/developing-today/code/issues/5906) —
    closed 2026-11-08 with
    `doc/2026-11-08_decision_id5906_kdl_json_contract.md`. That record
    fixes the canonical JSON shape (§3), the two-tier
    lossless-enough promise (§2), and the round-trip corpus (§6). This
    record **consumes** those and does not restate them.
  - [#5907](https://github.com/developing-today/code/issues/5907)
    (triple decomposition rules) and
    [#5908](https://github.com/developing-today/code/issues/5908)
    (payload boundary) — successors; §7 states what this record leaves
    to them.
  - [#5753](https://github.com/developing-today/code/issues/5753)
    (forked KDL Rust stack; universal/multiple/structured annotations)
    — still open and blocked on #5911. §8 records why the surface
    design does not depend on it.
  - [KDL 2.0 specification](https://kdl.dev/spec/) (IETF draft
    `draft-marchan-kdl2`, 2.0.0 released 2024-12-21), §3.2–§3.17, read
    while drafting.
  - In-tree evidence at the baseline: `pkgs/id/README.original.md`
    (the KDL ancestor scratchpad reference) and the absence of any KDL
    crate in `pkgs/id` — see §1.
- **Evidence baseline:** `origin/main` at `cc09dd4f5f`, worktree
  `~/git/fleet/12394`, 2026-11-08. Verified at that baseline:
  `grep -ril kdl pkgs/id` returns only `README.original.md` and build
  artefacts — **no KDL parser, validator, or ingest code exists in
  `pkgs/id`**. This record is the first design for the presentation
  surfaces.

## Decision summary

| Question | Decision |
| --- | --- |
| How many pipelines | **One.** All three surfaces call the same five-stage pipeline of §2.2. A surface may add a *front-door step* that produces KDL text, and a *presentation step* after decomposition; it may never skip, reorder, or re-implement a pipeline stage (§2) |
| Export surface | A **read-only, derived view**: KDL is rendered from the triple store through the canonical JSON (triples → canonical JSON → KDL). Export never writes; it is not an ingest path (§3) |
| Hand upload | A **document** submitted to the pipeline with an author identity. Multi-node documents are accepted and decomposed as one compound statement set (§4) |
| Agent statements | **The same document path**, with a machine author identity and a required provenance stamp. No agent-only endpoint, no privileged validation (§5) |
| Trust | Trust is a **property of the author**, evaluated by one policy stage; it never changes which pipeline runs. Untrusted authors (human or agent) go to staging, not to a separate pipeline (§6) |
| Round trip | **Canonical JSON → KDL → canonical JSON** has #5906's Tier A guarantee. Store/triples → KDL reconstruction, reimport idempotency, and statement identity are **not** promised here; #5907 must settle them (§3.3) |
| Surface-specific steps | Exactly three, all outside the shared pipeline: authoring/upload UX (§4.2), the export renderer's grouping choices (§3.2), and the agent provenance stamp (§5.2) |
| What is deferred | Decomposition rules (#5907), payload boundary (#5908), re-composition grouping heuristics (§3.4), and the annotation syntax (#5753) |

## 1. What the parent already fixed (not re-decided here)

#5688 fixes, and this record takes as given:

1. **KDL is the human presentation format** — not the wire format
   (postcard stands, §1.3) and not the query substrate (§1.3).
2. **The pipeline is parse → canonical JSON → atomic triples →
   resolve through IDs** (§2.2), with canonical JSON as the audit /
   provenance anchor.
3. **§1.4 names three presentation uses** — human-readable exports,
   hand-authored upload, agent-authored statements — and states the
   governing constraint verbatim: *"Same pipeline, no agent-only back
   door."*
4. **Qualifiers and namespaces are triples, never special cases**
   (§2.3). This record does not introduce a surface-specific store,
   column, or parser branch.

#5906 fixes, and this record consumes:

5. **Canonical JSON is the fixed point**; KDL is the human
   presentation, YAML and XML are renderings.
6. **Lossless-enough is two-tier**: Tier A (tree + typed scalars)
   required, Tier B (comments, whitespace, quoting) not promised.
7. **The canonical JSON document for one authored compound statement
   is the provenance anchor**, and its content hash is the statement
   node's identity candidate.

## 2. One pipeline, three front doors

The parent's §1.4 constraint is the spine of this record. Stated as a
shape:

```
                 ┌──────────── export read path ────────────────────────┐
export  ────────►│ stored canonical JSON anchor ─► KDL     [render]     │
                 └──────────────────────────────────────────────────────┘

upload  ──► [front door: author + document] ──┐
agent   ──► [front door: author + document] ──┤
                                              ▼
                 ┌──────────── shared pipeline (parent §2.2) ───────────┐
                 │ 1 parse+validate ─► 2 canonical JSON ─► 3 decompose  │
                 │ ─► 4 resolve through IDs ─► 5 trust/policy gate      │
                 └──────────────────────────────────────────────────────┘
```

### 2.1 The shared stages, named once

| Stage | Input | Output | Where it lives |
| --- | --- | --- | --- |
| 1. Parse + validate | KDL text | validated KDL document + actionable errors | shared |
| 2. Canonical JSON | validated document | canonical JSON (per #5906 §3) | shared |
| 3. Decompose | canonical JSON | atomic triples + statement node | shared (#5907 owns rules) |
| 4. Resolve through IDs | triples | ID-referenced triples | shared |
| 5. Trust / policy gate | statement + author | accepted, staged, or rejected | shared (§6; additive to the parent) |

Every **write** surface enters at stage 1. The export read path starts
from a retained canonical JSON anchor and renders it as KDL; it is not
a reversed write pipeline. **No write surface may enter at stage 3 or
later.** That is the concrete meaning of "no agent-only back door":
there is no entry point that takes pre-decomposed triples from a
surface and skips validation and canonicalisation.

### 2.2 Why the gate is stage 5, not stage 0

An untrusted author's document must still be *validated and
canonicalised* before anyone decides whether to trust it — otherwise
the trust decision is made about text nobody has parsed, and a
malformed document reaches a human reviewer as an ambiguous error.
Validation first, policy second, and the policy decision is recorded
against the canonical JSON anchor either way.

### 2.3 Front-door steps are additive, and exactly three

A front door may do work **before** stage 1 or **after** stage 2, and
that work is limited to:

1. **Authoring / upload UX** (§4.2) — how the text was produced or
   submitted, and by whom.
2. **Export grouping** (§3.2) — how triples were chosen and ordered
   into a readable document.
3. **Agent provenance stamp** (§5.2) — the machine author's identity
   and the tool/run that produced the statement.

Anything else a surface wants to do to the data is a pipeline change
and belongs in the shared stages, not in a front door.

## 3. Human-readable export

### 3.1 Export is a derived, read-only rendering

Export is **not** a data path. For an authored statement it reads the
retained canonical JSON provenance anchor (#5688 §2.2; #5906 §5) and
renders that JSON as KDL. It does **not** infer the source document by
recomposing triples. Consequences:

- Export never mutates the store and never mints IDs.
- Export output is **not** an ingest artifact by itself; feeding it
  back in goes through the ordinary upload path (§3.3).
- The anchor→KDL conversion inherits #5906's JSON↔KDL round-trip
  contract. Exporting a projection that has no source anchor is a
  different operation, and #5907 must define whether and how to
  synthesise it.

### 3.2 Grouping: preserve the authored statement boundary

For anchored authored statements the export uses the canonical JSON
document's existing tree and statement boundary; it does not group
triples heuristically. This preserves what was authored rather than
inventing a new compound statement. Exporting an arbitrary triple
projection is deferred to #5907; until then, an export request that
cannot select complete source anchors must report that limitation
rather than silently dropping or regrouping facts.

- **The authored anchor is the boundary.** One authored compound
  statement is one canonical JSON document and one anchor (#5906 §5),
  so it renders as one KDL document.
- **Ordering follows #5906's Tier A rules** — the renderer does not
  sort facts or invent fallback nodes.

### 3.3 Round-trip expectations (acceptance criterion 3)

Stated explicitly, with the follow-on named where deferred:

- **Guaranteed by #5906:** `canonical JSON → KDL → canonical JSON`
  returns Tier A semantic equality. The oracle is canonical-JSON
  equality, not KDL text equality (Tier B is not promised).
- **Guaranteed by this surface design:** exporting an authored
  statement from its retained canonical JSON anchor does not require
  reconstructing that authored tree from triples.
- **Not claimed:** `triples/store → KDL → upload → the same triples`.
  Decomposition and re-composition rules are not settled, and #5906 §5
  calls the content hash an identity *candidate*, not a decision that
  reimport deduplicates. #5907 owns grouping, identity, and the
  store-level round-trip contract. Until then, export/reimport must not
  be represented as an idempotent store operation.
- **Deferred:** whether an edited export is a new statement, a revision
  linked to its predecessor, or an update is #5907's identity and
  provenance decision. This record promises no store mutation
  behaviour.
- **Deferred:** exporting arbitrary triple projections, or composing
  several statement anchors into one KDL document, is #5907's
  re-composition/grouping decision.

### 3.4 What export is not

Export is not a backup format (postcard/database dump is), not a query
surface (§1.3), and not a format the store accepts without going
through stage 1.

## 4. Hand-authored upload

### 4.1 One authored compound statement, with an author

The unit of hand upload is **one authored compound statement**,
submitted as a KDL document by an identified author. It is parsed as a
whole (stage 1), canonicalised as a whole (stage 2), and decomposed
(stage 3). This follows #5906 §5: one authored compound statement is
one canonical JSON document and one anchor. How a batch of independent
statements is represented or split is #5907's; this record does not
turn several independent statements into one anchor.

### 4.2 The front-door step: authoring and submission

The surface-specific work of hand upload is:

- **Authoring affordance** — how the person writes or edits KDL
  (editor, form, template, or plain text). This is UX and adds no
  validation.
- **Submission** — the document plus the author identity plus an
  optional note describing intent.
- **Nothing else.** Client-side linting is a convenience that must
  call the same validator (stage 1) rather than reimplement it; a
  client-side check that disagrees with the server is a bug in the
  client.

### 4.3 Errors must be actionable for a non-developer

The parent requires rejection "with errors a non-developer author can
act on" (§2.2 stage 1). This record fixes the error contract:

- Every error names **line and column**, the **KDL construct** it
  applies to, and **what to do** — not a parser internal.
- Errors are reported for the whole document where possible, not
  first-error-only, so an author fixes one pass rather than many.
- Schema errors (valid KDL, wrong statement shape) are distinguished
  from syntax errors, because the fixes differ.

## 5. Agent-authored statements

### 5.1 The same door

An agent emits KDL and submits it through **the same upload path**
with a machine author identity. There is no agent endpoint, no
"trusted internal" fast path, and no pre-decomposed-triples input.
This is the parent's "same pipeline, no agent-only back door" applied
literally.

Rationale: an agent is an author whose output is reviewable. The value
of KDL for agents — readable in logs, reviewable by a human before
commit — depends on the agent's output being *the same artifact a
human would submit*, so a reviewer reads exactly what the pipeline
will parse.

### 5.2 The front-door step: a provenance stamp

The only agent-specific addition is a **required provenance stamp**
recorded with the submission: the agent/tool identity, the run or
session identifier, and (where available) the model or version. The
stamp is stored as provenance on the canonical JSON anchor, not as a
KDL construct the agent must author — so an agent cannot forge or omit
it by writing KDL differently, and a human upload simply carries a
human author stamp.

### 5.3 Untrusted agents

The parent's open question — "same trust path as hand uploads, or
sandboxed staging until approved?" — is answered by §6: **the same
trust path, with the trust decision made by author policy rather than
by surface.** An untrusted agent and an untrusted human take the same
staging route.

## 6. Trust as one policy stage, not two pipelines

The trust gate (stage 5) evaluates **the author**, not the surface:

| Author class | Route |
| --- | --- |
| Verified human | accepted on validation success |
| Verified agent (a known, approved tool identity) | accepted on validation success |
| Unverified human | staged: canonical JSON anchor recorded, triples held for approval |
| Unverified agent | staged, same as unverified human |
| Rejected author | refused before stage 1 runs, with a clear reason |

Two invariants:

1. **Trust never changes the pipeline.** A staged submission has still
   been parsed, canonicalised, and decomposed; only the *commit* of
   its triples is held.
2. **Staging is recorded against the anchor**, so approving a staged
   submission commits exactly what was reviewed — the canonical JSON
   the reviewer saw.

## 7. What this record leaves open (and to whom)

- **Decomposition rules** (statement node shape, n-ary handling,
  qualifier triples) — #5907. This record assumes only that a
  statement node exists and anchors its triples.
- **Payload boundary** (what is attached rather than decomposed, and
  how payloads render in KDL export) — #5908. Default until decided:
  a payload renders as a reference node carrying its ID, never as
  inline content.
- **Annotation syntax** for the fork's multiple/structured annotations
  — #5753. Default until decided: single annotations only; a document
  using multiple annotations is rejected at stage 1 with an actionable
  error rather than silently narrowed.
- **Revision linkage of an edited export** — #5907 (§3.3).
- **Cross-statement re-composition for readability** — parent's open
  question (§3.3); default is no.
- **Where the trust policy is configured and who may approve staged
  submissions** — an implementation concern for the ingest ticket, not
  a design decision here.

## 8. Ordering: why this record proceeds without #5753

#5753 (the forked KDL Rust stack, and the annotation-payload question)
is open and blocked. This record does not depend on it because:

- The surfaces are defined in terms of **the pipeline contract**, not a
  particular parser crate. Stage 1 is "the validator", whichever crate
  satisfies #5753's fork decision.
- The only #5753-shaped decision that reaches a surface is annotation
  syntax, and §7 gives it a safe default (single annotations; reject
  the rest with an actionable error).

If #5753 lands a different annotation model, the change is confined to
stage 1's accepted grammar and the export renderer's annotation
emission — neither of which alters the three-front-door structure.

## 9. Where this record touches a parent decision

It does not change any parent decision. It refines two:

1. **§1.4's "same pipeline, no agent-only back door"** is made concrete
   as a stage-entry rule (§2.1): every write surface enters at stage 1,
   and the export read path starts from a retained anchor.
2. **§2.2's stage list** gains an explicit **stage 5 trust/policy
   gate**, which the parent implies (its open question about untrusted
   agents) but does not name. This is additive: stages 1–4 are
   unchanged.

## 10. Alternatives rejected, and why

| Alternative | Why rejected |
| --- | --- |
| Three separate pipelines, one per surface | The parent's explicit constraint; three pipelines drift, and the drift is invisible until an export fails to convert back |
| An agent endpoint that accepts pre-decomposed triples | Skips validation and canonicalisation — the agent-only back door the parent forbids, and it makes the canonical JSON anchor unverifiable |
| Trust gate at stage 0 (before parsing) | Decides trust about unparsed text; malformed documents reach reviewers as ambiguous errors |
| Export as a first-class data path (export → store) | Export must be derived and read-only, or the store has two write paths with different validation |
| Round-trip guarantee on KDL text | #5906 already ruled Tier B (formatting) out of the promise; requiring text equality would forbid the renderer from normalising |
| Separate staging store for untrusted submissions | Staging is a *state* of a submission against its anchor, not a second store; a second store duplicates the schema and the resolution path |
| Client-side validation as the real validator | Two validators disagree; the client's is a convenience over stage 1 |

## 11. Invariants implementation must hold

1. Every write path enters at stage 1; no path enters at stage 3 or
   later.
2. Export is read-only and never mints IDs or mutates the store.
3. Export never drops a fact: a request that cannot select complete
   source anchors reports the limitation instead of silently
   regrouping or dropping data.
4. `canonical JSON → KDL → canonical JSON` is Tier A equal
   (canonical-JSON oracle, per #5906).
5. Store-level reimport identity is not assumed; it is #5907's
   contract. Until then no surface presents export/reimport as an
   idempotent store operation.
6. Trust is evaluated on the author, after validation, and never
   changes which pipeline runs.
7. Agent provenance is recorded on the anchor, not authorable as KDL.
8. A staged submission commits exactly the canonical JSON that was
   reviewed.
9. Every surface-specific step is one of the three in §2.3; anything
   else is a pipeline change.

## 12. Change log

- 2026-11-08 — initial record, drafted against `origin/main` at
  `cc09dd4f5f`; consumes #5906's closed decision; defers decomposition
  (#5907), payload boundary (#5908), and annotation syntax (#5753).
- 2026-11-08 — narrowed the round-trip promise before opening the PR:
  store-level reimport identity and triple→KDL reconstruction are
  #5907's to settle (#5906 calls the content hash a *candidate*), so
  this record claims only the canonical-JSON conversion guarantee.
