# Design: AI over notes — grounded answers, exact citations, and whose AI sees whose notes

- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Date: 2026-11-09
- Roadmap item: none; this is a cluster follow-on under [#5678](https://github.com/developing-today/code/issues/5678).
- Issues: [#5842](https://github.com/developing-today/code/issues/5842) (this cluster), parent [#5678](https://github.com/developing-today/code/issues/5678), object model [#2048](https://github.com/developing-today/code/issues/2048), read-authority operationalisation [#5840](https://github.com/developing-today/code/issues/5840).
- Baseline: `main` at `69aa168d5da3aa0265f95ffdedeeed2fb4da1ce3`.

## Intent

#5678 asks for one design per feature cluster, including an "AI-over-notes cluster (à la NotebookLM)": source-grounded Q&A over a user's corpus, summarisation, citation back to source, audio/overview generation if in scope, and the privacy model — whose AI sees whose notes. #5842 asks this record to settle the chosen shape, the rejected alternatives, the invariants implementation must hold, and the parent's two open questions: **local vs hosted models**, and **whether AI processing changes the read-authority analysis**.

Today there is no such feature and no partial implementation: nothing in `pkgs/id` performs corpus-grounded answering, and no `doc/` record covers it. A user can read their own (and, eventually, shared) notes, but cannot ask a question and receive an answer that is traceable to the exact revision it came from.

This record inherits, and does not re-decide, two rulings:

1. The parent's read authority: **default public read**; the owner may grant read to any number of groups and/or users; resolution order **object → user default → server config → public**. The parent's constraint is explicit: *"AI indexing/search must never expose content to a reader the resolution order would deny."*
2. #2048's object model: a stable note ID, signed immutable revisions, and typed references whose optional pinned revision is an exact citation (`doc/2026-11-01_design_id2048.md:36-46`). The operational read-policy order and the current enforcement gaps are recorded in `doc/2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions/2026-11-08T08-19-40Z_design_id_knowledge_sharing_permissions.md:35-44,64-68` (#5840).

## Decision in brief

Build a **grounded-answer module**: given an asker, an explicit corpus selector, a question, and a processing choice, it returns answer segments that each cite a stable note ID and an exact revision ID. Retrieval and context assembly always run **as the asker**, under the existing read policy, and the model — local or hosted — is a **processor, never a principal**. It receives only the asker-readable plaintext placed in its prompt and can neither read the store nor widen access.

**Model locality is the user's choice, with local as the default and hosted inference only under recorded, per-corpus, provider-named consent.** A note owner may mark a note **local-only**; that constraint can only tighten, never widen, read authority. Audio/overview generation is out of v1 scope except as a rendering of an already-grounded answer.

**AI processing does not change the read-authority analysis.** It adds obligations — a viewer-scoped index, per-viewer display filtering, consent, and an audit trail — but no new authorization path, no service identity, and no read the order would not already allow.

## Scope and constraints

In scope: corpus-grounded Q&A; summarisation and overviews over an explicit corpus; citation-back-to-source; the privacy model for local and hosted inference; the seam, invariants, and implementation order for these.

Out of scope: implementing retrieval, an index, a model runtime, or an endpoint; re-deciding the parent's read order or #2048 note identity; the graph, sync, content-model, sharing, capture, extensibility and attachment clusters; the importer designs; automatic tagging, semantic search as a user-facing standalone feature, and agentic tool use over the store.

The constraints that bound every decision below:

- **No full-text search exists.** `pkgs/id/src/search.rs:1-21` states full-text search is out of scope for Phase 5; search covers tag names/values over live claims, rebuilt per call and not persisted. #2048 permits a *later* full-text index only if it is scoped to content the viewer is authorized to read (`doc/2026-11-01_design_id2048.md:46`).
- **The read boundary is not enforced yet.** `event_sync` serves the whole store under `PullPolicy::Open` (`pkgs/id/src/event_sync.rs:1-31`); `blob_gate` has no reference-index producer and is not wired into `id serve` (`pkgs/id/src/blob_gate.rs:1-25`). #5840 records both as explicit prerequisites, not presumed capabilities.
- **No embedding store, model runtime, consent record, per-segment provenance, or audit log exists.** These are prerequisites, not assumptions.

## Terms

- **Asker:** the ID account on whose behalf a question is retrieved and answered. Every store read carries this identity.
- **Corpus:** the explicit set of notes a question is asked over — a note, a revision range, a collection's readable members, or a user-selected set. It is never an implicit whole-store default.
- **Grounded answer:** a derived artifact made of segments, each citing at least one `(note ID, revision ID)` pair or marked ungrounded.
- **Processor:** a model runtime (local process or hosted provider) that receives a prompt and returns text. It has no identity, credentials, or standing access to notes.
- **Derived artifact:** an answer, summary, embedding, or audio rendering produced from note content. It is a copy, not a source of authority.
- **Local-only note:** a note whose owner has forbidden hosted inference over its content. This is a processing constraint, not a read restriction.

## Design

### 1. The seam: one grounded-answer module

A single deep module owns corpus selection, retrieval, context assembly, inference, provenance assignment and output filtering. Its interface is small:

```
answer(asker, corpus_selector, question, processing_choice)
  -> { segments: [{ text, citations: [{note_id, revision_id, span?}] }],
       ungrounded_count, withheld_count, model: local | hosted(provider), provenance }
```

Callers (Q&A UI, summary UI, a future audio renderer) do not retrieve, prompt, or filter themselves. Tests cross the same seam: an extractive adapter with no model can exercise citation and filtering; a fake local model can exercise provenance; a fake hosted provider can exercise consent. Depth here is deliberate: the alternative — each surface doing its own retrieval and filtering — is how a second, weaker access path appears.

### 2. Corpus selection and retrieval run as the asker

The selector names notes, a collection, or "everything I can read". The module resolves it to the **intersection of the selector and the asker's currently readable set**, using the same policy decision function #5840 requires for search, sync and blobs. Resolution happens *before* retrieval; a broader selector never widens authority. A note the asker cannot read contributes nothing — not its title, ID, body, or existence. The answer may report an aggregate withheld count, never a per-note explanation.

Retrieval needs an index, because the current tag-only search (`pkgs/id/src/search.rs:72-96`) cannot ground prose. The index is **viewer-scoped**: built from content the viewer may read and queried under the same decision function, or stored per-world with access checks applied to every hit. #2048's rule stands: no global index, and any full-text index is scoped to authorized content. Embeddings, if used, are derived data under the same scoping (§7).

### 3. Citations are exact and revision-pinned

A citation is `(stable note ID, revision ID)` — optionally a span — never a title or a URL. Revision IDs are what make a citation exact (`doc/2026-11-01_design_id2048.md:36-38`): a later edit to the note cannot silently change what the answer cited. A segment with no citation is rendered as **ungrounded**, visibly distinct from a sourced one. The module never fabricates a citation to satisfy the format: if synthesis cannot be attributed, it is marked, not back-filled.

### 4. Display is per-viewer; derived text is tainted by its sources

An answer quoted from a restricted note is a copy of that note's content. Serving it to a reader who cannot read the source would leak exactly what the read order protects. Therefore:

- The answer artifact is **private to the asker by default**.
- Rendering evaluates read authority **for the viewer**, at display time, per segment. Segments whose sources the viewer may not read are withheld and replaced by an aggregate withheld count. No quote, title, note ID, or existence signal survives a denial.
- **The asker is not special at render time.** The same renderer runs for the owner and for a share recipient, so a policy change after generation takes effect without rewriting the answer.
- Saving an answer as a note is an explicit act. It creates a new #2048 note with its own policy resolved by the ordinary order — but the UI must warn when the content embeds restricted sources, because widening that note's audience is a deliberate **copy**, not a grant. This is the honest boundary #5840 already states: revocation cannot recall what an authorized reader copied.

### 5. Summaries and overviews have explicit scope

A summary targets an explicit set: one note, a revision range, a collection's readable members, or a caller-supplied set. "Everything I can read" is allowed only because it is resolved per asker at query time; it is never a shared artifact and never a global index. Collection overviews are summaries of that collection's readable members, with the same citation and taint rules.

Audio/overview generation is **deferred**. If added later it is a presentation layer over an already-grounded answer (text-to-speech of cited segments), not a second generation path that could escape provenance. The NotebookLM-style "audio overview" is therefore a rendering decision, not an AI-authority decision.

### 6. Model locality and the privacy model

**Local is the default; hosted is opt-in per corpus.** The retrieval boundary is what enforces authority, and it runs where the data lives. The inference step may be local (a bundled or user-configured local endpoint) or hosted (a configured provider), but the provider only ever sees the prompt the harness assembled for the asker.

- **Hosted requires consent.** Before the first hosted request over a corpus, the asker records consent naming the provider and the corpus (or collection), and the UI states plainly that note plaintext leaves the device to that provider. Consent is revocable and is never implied by an account or server default.
- **The model is a recipient, not a principal.** There is no AI service account, no standing read grant, and no store access for the model. "On someone else's behalf" means the processor acts for the asker and inherits only the asker's readable set; it never inherits the note owner's rights and never gains rights of its own.
- **Processing policy can tighten, never widen.** A note owner may mark a note local-only; a server may forbid hosted processing entirely. Neither can grant read. The asker's consent selects *where* processing happens among permitted options; it cannot override a local-only note.
- **No automatic widening.** The service never turns a private answer into a shared one, never indexes a corpus for someone who cannot read it, and never uses an answer to expose a source that the read order would deny.

This answers the parent's privacy question directly: *whose AI sees whose notes?* The asker's AI sees exactly the asker's readable set, in the asker's context; a hosted provider sees only what the asker consented to send; no AI sees a note on behalf of someone who cannot read it.

### 7. Derived data is not a second access path

Indexes, embeddings, answers and summaries are copies. They are stored under the policy of the content they derive from, queried under the asker's authority, and never served for content that has been revoked or is no longer readable. An embedding index is not shared across principals unless the underlying content could be. Deleting or revoking a source must not leave a queryable derived artifact that answers questions about it. This is the same non-erasure caveat as #5840, applied to derived data: the service must not resurrect what policy now denies.

## Decisions

1. **Grounded answers, not a chat over an open corpus.** The unit is a question over a named corpus, with per-segment citations. A free-floating chat that can roam the store has no defensible corpus boundary and no stable citation target. *Reason: the parent's constraint is about what a reader is shown; an explicit corpus makes that checkable.*
2. **One deep grounded-answer module, not per-surface pipelines.** Q&A, summary and any future audio renderer cross one seam. *Reason: a second retrieval path is a second place to get authorization wrong; depth concentrates the check and the tests.*
3. **Retrieval always runs as the asker; no service principal.** *Reason: a service account would be a second authorisation path, would exceed any individual's authority, and would make audits ambiguous. The parent's order already names the principals.*
4. **Citations pin the stable note ID and the exact revision ID.** *Reason: #2048 already made revision IDs the exact-citation unit; titles and URLs are mutable locators. Pinning is also what lets display filtering re-check the right content later.*
5. **Answers are private by default; display filtering is per viewer at render time.** *Reason: derived text is a copy, and a copy can leak a restricted source. Checking at render time keeps policy changes effective without rewriting artifacts, and avoids a stale copied ACL.*
6. **Local inference is the default; hosted is opt-in per corpus with recorded provider consent.** *Reason: local-first matches ID's storage model and makes the privacy default real; forcing hosted would make a private default impossible; forcing local would foreclose capable models for users who accept the disclosure. The choice belongs to the person disclosing, within the owner's tightening constraint.*
7. **An owner may mark a note local-only; processing policy tightens only.** *Reason: "whose AI sees whose notes" is not fully answered by read grants alone if a grantee can send content to any provider. A tightening-only processing flag respects owner control without re-deciding who may read.*
8. **Audio/overview is a rendering of a grounded answer, deferred past v1.** *Reason: a separate audio generation path would need its own provenance and taint rules; rendering an existing cited answer inherits both.*
9. **AI processing does not change the read-authority analysis.** *Reason: the processor has no identity, retrieval is asker-scoped, derived data is scoped like its sources, and output is filtered per viewer. It adds obligations and a disclosure recipient, not an authorisation path.*
10. **No global index and no implicit whole-store corpus.** *Reason: #2048 forbids a global index; an implicit corpus would be an unbounded disclosure and would make "what did the AI see" unanswerable.*

## Alternatives considered

| Alternative | Why it is not v1 |
|---|---|
| A central hosted service holds a copy of the corpus and indexes it provider-side | Creates a second store with its own ACL that drifts from the read order, forbids a local-first default, and turns a provider breach into a corpus breach. It also requires the global index #2048 forbids. |
| A dedicated "AI reader" service account with broad read | A second principal that exceeds any asker's authority, makes revocation and audit ambiguous, and contradicts the parent's resolution order. |
| Let the model retrieve via tool calls with store credentials | The model could query beyond the asker's scope, and its access would not be reviewable as the asker's. Retrieval must be harness-controlled and asker-scoped; the model sees only the assembled prompt. |
| Filter only in the client | Client filtering does not protect sync or blob serving — the exact lesson of #5840. A modified or alternate client would bypass it. |
| Copy the sources' ACL onto the answer at generation time | Policy changes and group membership drift, leaving a stale grant. Per-viewer evaluation at render time is the only point that stays correct. |
| Persist every answer as a signed note automatically | Floods the permanent event log with derived text, confuses authored work with machine output, and creates a durable copy that must then be re-governed. Saving is explicit. |
| A default "ask my whole vault" corpus | Unbounded disclosure, no stable scope to consent to, and no defensible answer to "what did the model see". Corpora are named. |
| Hosted-only inference | Forecloses a local privacy default and contradicts ID's local-first storage model. |
| Local-only inference | Forecloses capable models for users who would accept a disclosed provider; unnecessary as a global rule when consent plus a per-note local-only flag covers both. |
| Treat audio overview as its own generation path | It would need separate provenance and taint handling and could escape citation requirements. Render the grounded answer instead. |

## Invariants for implementation

1. No asker, no answer: every store read carries the asker's identity and existing read authority; there is no AI service principal or standing grant.
2. Corpus resolution is `selector ∩ asker-readable`, computed before retrieval; a broader selector never widens authority, and an unreadable note contributes no title, ID, body or existence signal.
3. Model input is a subset of asker-readable plaintext. Hosted inference requires recorded, per-corpus, provider-named consent, and consent cannot widen read.
4. A note marked local-only is never sent to a hosted provider, whatever the asker consents to.
5. Every answer segment cites at least one `(note ID, revision ID)`, or is visibly marked ungrounded; a citation names an exact revision.
6. Rendering filters per viewer at display time: unreadable segments are withheld behind an aggregate count with no quote, title, ID or existence leak; the same function runs for the asker and for share recipients.
7. Answers are private to the asker by default; saving one as a note is explicit, and widening its audience is an explicit copy action that warns when restricted sources are embedded.
8. Derived data (indexes, embeddings, answers, summaries) is scoped to and access-checked against the content it derives from; it cannot serve revoked content or become a second access path.
9. Processing policy (owner local-only, server no-hosted) can tighten but never widen read authority.
10. No global index, no implicit whole-store corpus, and no note-identity change: citations use the #2048 stable note and revision IDs unchanged.

## Implementation order and acceptance

1. **Land the prerequisites first.** #5840's enforcement slices (policy resolver, signed policy updates, event sync and blob serving under the policy, collaboration writes) and #2048's phase issues for stable note identity, typed references and durable revisions (#3814, #3816, #3819). Grounding cannot be safer than the store it reads; no AI surface is enabled before the event/blob boundary is enforced.
2. **Add viewer-scoped retrieval.** A scoped full-text index (and, if justified, an embedding index) that is built per viewer or access-checked per hit. Revisit `pkgs/id/src/search.rs`'s full-text exclusion deliberately; do not make the existing tag search global. Property tests: `selector ∩ authority` holds; no unreadable hit; denials leak no existence.
3. **Land the grounded-answer seam with an extractive-only adapter.** No generative model: return cited spans. This proves citation, per-viewer filtering and withholding before a model is in the loop.
4. **Add a local generative adapter behind the same seam.** Property tests: prompt inputs are a subset of the authority set; every segment carries provenance or is marked ungrounded.
5. **Add a hosted adapter behind consent records.** Tests: no request without recorded consent; consent is per corpus and per provider; a local-only note vetoes the request; revocation stops future requests.
6. **Add the share path and audit.** Per-viewer display filtering for recipients; an audit record of asker, corpus selector, provider and withheld count, with no note plaintext.
7. **Add summaries and collection overviews** over explicit sets, reusing steps 2–6. Defer audio to a follow-up design.

Acceptance for this record is the record itself being accepted by the issue owner; closure of #5842 waits for that acceptance, not for implementation. Implementation issues are filed from the reviewed record rather than bundled into it. The feature is not enabled until steps 1–5 pass their boundary tests.

## Consequences

- **Easier:** one seam serves Q&A, summaries and future audio renderers; the citation and filtering rules are stated once and tested through a single interface.
- **Harder:** per-segment provenance, per-viewer rendering, consent UX and an access-checked index all cost work the current tag-only search does not do. Model quality and citation fidelity need an evaluation harness this record does not specify.
- **Costs:** a retrieval index must be scoped and access-checked, which may be the most expensive prerequisite; hosted inference adds provider configuration, a consent record and an audit trail.
- **Compatibility:** v1 needs no new signed event kind for private answers; saving an answer reuses `note.create`/`note.revise`. If durable answer provenance becomes a signed fact, that is a new event kind — recorded as an open question, not assumed.
- **Migration:** none. The feature is additive and disabled until the prerequisites land.

## Open questions

1. **Where do private answer artifacts live** — client-local store, a server-side private store, or a note with an owner-only policy? *Settled by the client/storage slice owner when step 3 is designed.*
2. **Does a saved answer need a durable provenance event kind**, or is provenance metadata on the note sufficient? *Settled with the #2048 maintainers.*
3. **Is a note's local-only mark a signed claim or a local preference?** A signed claim survives sync and is visible to collaborators; a local preference does not. *Settled with the #5840 policy work.*
4. **Audio/overview generation:** in scope later, and who owns that design? *The issue owner decides after this record is accepted; the recommendation is a separate rendering design.*
5. **Which local models meet the citation and grounding bar**, and what is the evaluation oracle? *Settled by an evaluation spike, not by this record.*
6. **Audit retention and who may read the audit trail.** *Settled by security/privacy review before step 6.*

## Change log

- 2026-11-09: Initial decision record for #5842, inheriting the #5678 read-authority ruling and the #2048 object model; records the #5840 enforcement gap and the absent full-text index as prerequisites.
