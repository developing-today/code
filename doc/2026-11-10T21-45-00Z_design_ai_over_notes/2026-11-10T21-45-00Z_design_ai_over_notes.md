# AI-over-notes: source-grounded Q&A, citations, and read-authority

- Status: draft
- Date: 2026-11-10T21-45-00Z
- Roadmap item: none
- Issues: [#5842](https://github.com/developing-today/code/issues/5842)
  (this design), parent
  [#5678](https://github.com/developing-today/code/issues/5678),
  programme epic
  [#2056](https://github.com/developing-today/code/issues/2056),
  note identity
  [#2048](https://github.com/developing-today/code/issues/2048)

## Intent

Issue #5842 asks for the design of the AI-over-notes cluster from
parent #5678's decomposition: source-grounded question answering
over a user's note corpus (the NotebookLM shape), summarisation,
citation back to source, possibly generated audio overviews — and
the privacy model: whose AI sees whose notes. Implementation
timing is a separate decision; this record settles the shape so
implementation tickets can be filed against it. It does not
re-decide the parent's rulings; where an answer would change one,
that is recorded as an open question instead.

## The inherited ruling (folded in, not re-litigated)

Parent #5678 records Drew's read-authority default for the
knowledge layer, and this design inherits it verbatim:

- **Default: public read.**
- The owner may **grant read to any number of groups and/or
  users**.
- **Resolution order: object → user default → server config →
  public.** An object's own read setting wins; else the owner's
  user-level default; else server configuration; else public.

And the parent's binding corollary: *AI indexing/search must never
expose content to a reader the resolution order would deny.* Every
decision below is an application of that sentence.

## Design

**The model is a reader, not a principal.** An AI feature never
holds read authority of its own. For any question, there is an
**asker** — the end user whose request it is — and the system
computes the asker's **read set**: the notes the resolution order
(object → user default → server config → public) grants that
asker. Retrieval, ranking, generation context, and citations all
operate strictly inside that set.

- **Filter, then rank.** Retrieval filters the corpus to the
  asker's read set *before* any ranking, embedding lookup, or
  counting. Rank-then-filter leaks: result counts, "no results in
  your visible set" phrasing, timing, and near-miss snippets can
  all reveal that a denied note exists and roughly what it says.
  A shared index is permitted only if the filter is applied
  inside the query, per asker, before scores are computed.
- **Citations are a correctness requirement, not a display
  nicety.** Every factual claim in an answer cites the note and
  span it came from, and a citation may only point at a note in
  the asker's read set (which is automatic if generation only
  ever saw that set). Claims the retrieved sources do not support
  are either dropped or marked as uncited generation; an answer
  with no citable support says so. Citation links resolve through
  the same read check as any other note open, so a re-shared
  answer cannot become a side channel.
- **Derived artifacts inherit authority.** Embeddings, summaries,
  answer texts, caches, and audio overviews are new objects made
  from notes. Each derived artifact carries the read setting of
  its sources — for a multi-source artifact, the **most
  restrictive** source setting — except the answer itself, which
  belongs to the asker alone (the asker could already read every
  source). An index entry is a derived artifact: it inherits its
  note's setting and is filtered by it.
- **Acting on someone else's behalf.** When a model or service
  runs for an asker (a hosted assistant, a shared agent, a
  scheduled digest), the authority in force is the **end
  asker's**, never the service's. A service that itself can read
  more must not let that broader access bleed into retrieval: the
  read set is computed from the asker's identity and grants,
  full stop. If the asker cannot be established, the read set is
  the public set.
- **Summarisation and audio overviews** are the same shape as
  Q&A with a standing question ("summarise X"), so they inherit
  everything above: sources from the requester's read set only,
  the output a derived artifact with inherited authority. Audio
  adds a rendering step, not an authority question.

## Decisions

1. **Model locality: the asker's choice, local-first by
   default.** Retrieval (embedding and search) runs where the
   corpus lives — the asker's node — consistent with the fleet's
   local-first posture and with the secret-tier embedder record
   (#6059's design): corpus content is not shipped to a hosted
   service just to be indexed. Generation is the asker's choice
   of local or hosted model. Choosing a hosted model sends it the
   question plus the retrieved excerpts — all inside the asker's
   read set, so no authority is violated — and the UI states that
   those excerpts leave the node before the first hosted call.
   *Reason:* locality is a privacy and availability preference,
   not an authority boundary; the authority boundary is the read
   set, and it holds identically for a local and a hosted model.
   Making it a choice (rather than local-only) keeps the feature
   usable on hardware that cannot run a capable model; making
   local the default keeps the no-egress posture the rest of the
   platform assumes.
2. **AI processing does not change the read-authority analysis —
   it enlarges the surface it applies to.** The resolution order
   and its outcome for any (asker, note) pair are unchanged by
   the existence of AI features; no new principal, grant, or
   default is introduced. What changes is the *inventory of
   objects the order must be applied to*: embeddings, indexes,
   summaries, caches, and generated audio did not exist before,
   and each is a place a denied note's content could leak if the
   derived-artifact inheritance above is not implemented.
   *Reason:* if AI changed the analysis itself — say, by treating
   the index as a new kind of reader with its own grants — every
   cluster inheriting the parent's ruling would have to be
   re-audited per feature. Treating the model as the asker's
   agent keeps one authority model for the whole knowledge layer.
3. **Filter-then-rank is an invariant, not an implementation
   preference** (see Design). Any retrieval design that ranks a
   superset and filters afterwards is non-compliant by
   construction, whatever its other merits.

## Alternatives considered

- **A separate AI authority model** (the assistant as a user with
  its own grants, owners sharing notes "with the AI"). Rejected:
  it forks the parent's ruled model, makes the effective reader
  set unauditable ("who can learn what's in this note?" gains a
  second answer), and breaks the on-behalf case — the assistant's
  grants would substitute for the asker's.
- **Per-asker fully separate indexes.** Compliant, and the safest
  form, but rejected as the *required* shape: it multiplies
  storage per grant-combination and makes grant changes expensive
  re-index events. A shared index with in-query per-asker
  filtering (decision 3) reaches the same authority outcome;
  separate indexes remain a permitted implementation choice for
  small corpora.
- **Hosted-only retrieval and generation** (the NotebookLM shape
  taken literally). Rejected: it makes a hosted service a
  technical requirement for a core knowledge-layer feature, which
  the platform's local-first principles do not allow, and it
  cannot serve the secret tier at all (that tier's embedder
  record is local-only, fail-closed).
- **Citations as best-effort.** Rejected: without the citation
  requirement the feature is a plausibility engine over private
  notes; source-grounding is the cluster's stated purpose in the
  parent, and citations are also the audit trail that a answer
  stayed inside the read set.

## Prerequisites — what current code lacks (assumptions refused)

This design names two gaps as **prerequisites**, not assumptions;
implementation tickets must schedule them before or as the first
AI work, per the parent programme's findings:

1. **No full-text search exists in current code.** There is no
   retrieval substrate — neither lexical (FTS) nor embedding
   search over notes. Retrieval must be built (lexical first is
   acceptable; embedding retrieval follows the #6059 record's
   evaluation discipline) before any Q&A surface.
2. **The event/blob read boundary is unenforced in current
   code.** Read-authority resolution exists as a ruling, not yet
   as an enforced boundary at the layer where notes, events, and
   blobs are read. Building an AI layer over an unenforced
   boundary would launder access: the AI would faithfully enforce
   a read set the underlying reads do not. Boundary enforcement
   is prerequisite zero — it is also the sharing & permissions
   cluster's core deliverable, and this cluster depends on that
   one landing first.

## Implementation order (for the tickets filed from this record)

1. Read-boundary enforcement at the corpus/event/blob layer
   (shared with the sharing & permissions cluster).
2. Retrieval substrate: full-text search over notes with in-query
   per-asker filtering (decision 3); embedding retrieval as the
   follow-on, per the #6059 evaluation record.
3. Citation-grounded Q&A with a local model: read-set computation,
   filter-then-rank retrieval, citation-required answers.
4. Hosted generation as the asker's explicit choice, with the
   egress disclosure.
5. Summaries and audio overviews as derived artifacts under the
   inheritance rule.

## Consequences

- Grant and read-setting changes must propagate to indexes and
  derived artifacts (a note made less public must leave shared
  indexes and stop appearing in others' derived artifacts) — an
  invalidation obligation implementation must price in.
- Answers are only as checkable as their citations; evaluation of
  this feature measures citation precision (does the cited span
  support the claim?) and read-set containment (did anything
  outside the set influence the answer?), not just fluency.
- The on-behalf rule constrains future agent features: any agent
  acting for a user inherits this read-set discipline.

## Open questions

- Does the asker-facing egress disclosure for hosted generation
  need a per-corpus (not just per-answer) consent setting?
  Settled by: the sharing & permissions design, when it defines
  grant UX.
- Should derived artifacts be recomputed or deleted when a
  source's read setting tightens — and who pays for recompute on
  large corpora? Settled by: the retrieval-substrate ticket
  (step 2), with measurements.
- Is audio overview generation in the first implementation scope
  or a later rung? Settled by: Drew, when implementation timing
  is decided; this record's authority treatment covers it either
  way.
