# Design: where agent notes live — scratch, memory, repo, Zettelkasten, or ID graph

- **Issue:** [#6022](https://github.com/developing-today/code/issues/6022)
- **Parent:** [Atlas ingestion program #6018](https://github.com/developing-today/code/issues/6018)
- **Related programs:** [ID platform epic #2056](https://github.com/developing-today/code/issues/2056); [Zettelkasten layer #2048](https://github.com/developing-today/code/issues/2048)
- **Status:** placement decision for implementation; #6022 remains open for review and follow-on work.
- **Inputs:** #5545 (writer-set authority), #5686 (capability/relation family), #5955 (worlds and licensing), #5981/#5983 (docs migration), #6062 (ID-graph publication format), #6267 (notes submission pilot), #3125 (Atlas provenance registry), and #5669 (Atlas staging and sensitivity tiers).

## 0. The rule in one paragraph

Choose the canonical home by what must remain authoritative, not by whether the content is Markdown. **Use an ID-native object only when the note itself needs stable graph identity, writer authority, or durable typed relations; use a repo document when its truth must version with one product, codebase, or operational system; use the Zettelkasten when the knowledge is durable and reusable outside one repo; keep task-local or personal operational context in scratch or memory.** If placement or sensitivity is uncertain, keep the submission staged and ask for human promotion—never publish by guessing.

The quick repo-vs-Zettelkasten test is: **would changing a particular repo's code, interface, or operation make this note wrong or require it to change?** If yes, the repo owns it. If no, but it is worth finding and linking again across projects, the Zettelkasten owns it. This is about canonical ownership; a reviewed projection may exist in another corpus with provenance.

## 1. Decision table — apply in order

First ask whether the content needs to be an ID object. If not, select a canonical file or workspace home from the table. An Atlas staging submission is a workflow state, not a competing canonical home.

| Note class | Default canonical home | Lifetime / update rule | Atlas treatment | ID-native eligibility |
| --- | --- | --- | --- | --- |
| Ephemeral working note, scratch plan, intermediate reasoning | Task/session workspace scratch | Only while the task needs it; discard or summarize when the task closes. | None. Do not ingest raw scratch. | No. Promote only a separately reviewed durable result. |
| Durable finding | Zettelkasten when useful across projects; repo `doc/` when it is specifically evidence or guidance for one codebase | Keep while useful; amend or supersede with provenance rather than silently replacing history. | Submit as a draft; default to `internal` after review. Move to `public` only by explicit promotion; local-only/personal material stays local. | Sometimes: only if the finding is itself a stable, addressable object with meaningful writer/relation semantics. Otherwise publish a Zettel note. |
| Design | Repo `doc/` for a product/repo decision; Zettelkasten for a cross-project concept not owned by one implementation | Versioned decision record; later changes are linked amendments or new dated records. | Stage the source and any approved Zettel projection; promote at the content's sensitivity level. | Only for an intentionally collaborative, identity-bearing design object. A Markdown design record is not ID-native merely because it discusses ID. |
| Research | Repo `doc/` when it supports a specific implementation; Zettelkasten when the result is reusable independently | Retain source citations and spans; update by adding a dated result or superseding note. | Stage reviewed findings, not an unfiltered research workspace. Default `internal`; public promotion is explicit. | Rarely. The research result normally remains a document/note; an ID object is justified only by independent identity and relations. |
| Daily log | Personal workspace or memory | Append according to the log's own retention; extract durable decisions instead of converting the whole log. | None by default. A selected, redacted summary may be submitted separately. | No by default. A log entry does not become a graph object just because it has a date. |
| User-facing document | Repo docs for a specific product/service; Zettelkasten for a general guide not owned by one product | Maintain with the owning product or corpus; preserve released revisions and their provenance. | Public only after publication review; otherwise stage at the appropriate sensitivity tier. | Eligible when the product intentionally offers the document as a collaboratively edited, addressable ID object with explicit writers and relations. Otherwise keep the file canonical. |
| Private operational preference or personal context | Agent/user memory; sandbox-only instructions remain in their designated sandbox home | Retain under the memory owner's rules; revise or remove there, not through a public corpus. | None. Memory is not a bulk-ingest source. A shareable extracted fact requires a separate reviewed submission. | No by default. Do not turn personal context into a shared ID object without an explicit publication decision. |

### 1.1 Ordered placement procedure

1. **Does the note need its own stable identity and graph semantics?** If other ID objects must refer to it, its writers must be governed through ID authority, or its relations must be queryable as graph state, choose an ID-native object. Use the publication format from #6062 and the existing authority/relation families; this document does not define their wire format.
2. **Is its correctness coupled to a particular repository, product, or operation?** Make that repo's `doc/` (or maintained package documentation) canonical. This includes implementation-driving designs and product-specific user docs.
3. **Is it durable and useful independently of one repository?** Make it a Zettelkasten note, with links/backlinks and source provenance. Use the corpus registry and human promotion to choose the destination corpus; do not infer access or ownership from an agent's ability to write to a repository.
4. **Is it only temporary, personal, or machine-specific?** Keep it in workspace scratch or memory. Do not send it to Atlas or a public corpus.
5. **Is the answer unclear or the sensitivity uncertain?** Leave it in the submission staging flow until its owner reviews it. No fallback to public publication.

## 2. Canonical copies, projections, and publication

There is one canonical owner for each published note. A second file is a projection, not a second authority.

| Canonical owner | Allowed copies | Authority rule |
| --- | --- | --- |
| Code repo | A reviewed Zettelkasten import; Atlas index/extract | The repo path at a pinned commit is canonical for repo-specific decisions. A Zettel import carries source repo/path, commit, content hash, authored/imported times, and remains a draft until promoted. Edit the canonical doc through its repo workflow; do not edit the projection as if it were the source. |
| Zettelkasten | Atlas index/extract; a repo link or short contextual pointer | The selected Zettel corpus is canonical for cross-project knowledge. A repo may cite it, but does not silently fork its content. Personal and shareable corpora remain distinct according to the corpus registry and sensitivity decision. |
| ID graph | Repo or Zettelkasten rendering/projection with the canonical ID and source revision | The ID object is authoritative for identity, writer set, and graph relations. Projections are regenerable and do not write back to the object. Exact object and lineage encoding is owned by #6062. |
| Submission DB (before promotion) | Staging metadata and content-addressed source evidence | The Turso submission DB is the system of record for the submission and its attempts. It is not the permanent canonical home after a human promotes the content. No direct auto-commit or auto-promotion. |

Every surface uses one `note_submission` envelope. At minimum, retain the content hash, provenance spans, source system, machine, and date. The submission DB records the candidate and its state; an authorized human reviews classification, destination, and sensitivity before publication. The #6267 pilot's draft frontmatter and provenance footer are the established import pattern, not permission to bypass staging for future submissions.

Atlas has two related but distinct classifications. The ingestion workflow first uses a **staging** unit and promotion; the physical core sensitivity level is exactly `public`, `internal`, or `secret`, with `internal` as the default. Promotion to `public` or `secret` is an explicit recorded move. Caller/source classifications such as `shareable`, `personal`, `local-only`, and `secret-bearing` are sensitivity ceilings, not synonyms for those physical tiers. Local-only material never leaves the machine; secret-bearing material is not made public by redaction-free ingestion. See #5669 and the Atlas binary service design for the detailed storage and ceiling rules.

## 3. ID-native publication boundary

An ordinary finding, design, research result, or user-facing Markdown page does **not** qualify merely because it has links or may be indexed. Publish a note as ID-native only when all are true:

- it needs stable identity independent of a repo path or Zettelkasten filename;
- another ID object or application needs to refer to its identity or relations as live graph data; and
- its writer authority and visibility can be expressed through the existing ID authority/capability model.

Likely candidates are intentionally collaborative ID knowledge objects and durable findings whose graph identity is part of their meaning. Scratch, daily logs, raw research, static repo designs, and personal memory are not candidates by default. A curated file can be a projection of an ID-native note, but the projection must name the canonical ID and revision and must not claim independent authority.

This consumes, rather than extends in parallel:

- **Writer authority:** #5545 and the `idset.writer` design. Its rule that stored membership and view-specific counting are distinct means a copy or projection must not accidentally become a second writer authority.
- **Capabilities and relations:** #5686 and the related worlds/capability work. The provider, capability, consumer, and grant model is the source for authorization; note placement must not create a note-specific permission model.
- **Publication and lineage format:** #6062. Its PROV-Lite terms (`wasRevisionOf`, `specializationOf`, `wasDerivedFrom`, `wasGeneratedBy`, `wasAttributedTo`) are source concepts to map into ID relations, not a new ID relation vocabulary. #6062 remains responsible for the concrete publication format.
- **Note references:** the existing `note.link` event and `NoteLinkOp::Assert`/`project_links` path represent note links; backlinks are a projection. Do not confuse topic links with tag triples or invent a second backlink store.

The ID graph owns note identity, writer authority, and graph relations only where the eligibility test above passes. It does not automatically become the blob store or canonical owner of every Markdown body; #6062 settles the encoding and content/projection mechanics.

## 4. Migration of the current sprawl

Apply the #5983 MOVE / STAY / STAY-UNTIL-REPLACED classification first. Its test is whether material makes sense to someone running the fleet/code without this sandbox. Sandbox-environment material remains in its designated sandbox home; documentation about a topic moves to that topic's canonical home. **Copy, never delete**; leave a one-line canonical pointer at a moved source when that source is maintained. The Atlas documentation sweep #5981 is the inventory for workspace design sources, not authority to ingest all workspace content.

| Existing source | Decision | Migration treatment |
| --- | --- | --- |
| `~/workspace/research_notes/` | Sweep and classify note by note. | Repo-specific findings/designs become repo docs; durable cross-project findings become reviewed Zettelkasten submissions; transient or personal research stays local. Preserve the source and provenance, add canonical pointers where the source is maintained, and do not bulk-ingest the directory. |
| `~/workspace/goals/**` | Grandfather active working files; promote durable outcomes. | Goal/task logs remain with the active goal. A final decision or reusable finding is copied to its repo or Zettelkasten home with provenance; the goal file points to it. Do not migrate every scratch file as a document. |
| `~/memory/` topics | Reference-only; not a move source. | Keep memory under its existing owner and rules. Do not bulk-copy or ingest topics. When a distinct fact or decision is approved for wider use, submit that extract through `note_submission` and leave a pointer in memory if useful. |
| Existing repo design/documentation imported to a Zettelkasten corpus | Preserve repo as canonical for repo-specific records. | Treat the Zettelkasten copy as a provenance-bearing projection/draft. Promote it only through the submission workflow; future repo edits do not silently overwrite a reviewed Zettelkasten note. |

Migration is additive and auditable: inventory source path and hash, classify, copy the chosen content, record the canonical destination, then add a pointer. Historical sources are not deleted to make the new rule appear complete.

## 5. Alternatives considered

- **Put every agent note in the Zettelkasten:** rejected. It would mix ephemeral work, private memory, and repo-specific decisions into one corpus and make ownership ambiguous.
- **Make every note an ID object:** rejected. Stable identity and writer/relation semantics are costly and unnecessary for scratch, logs, and ordinary documents; the ID graph is used only when its semantics are needed.
- **Treat Atlas as the canonical home:** rejected. Atlas is a retrieval/indexing and staged-promotion system over source material, not a replacement for the repo, Zettelkasten, memory, or ID graph.
- **Continue ad hoc placement or ingest whole workspaces:** rejected. It loses canonical ownership, provenance, and sensitivity review and conflicts with the #5981/#5983 classification and #6267 submission gate.

## 6. Implementation sequencing and checks

The next implementation issues are linked from #6022. They must consume this placement policy and the existing submission pipeline; they must not reimplement #6062's ID publication format or #5981's docs inventory.

- [#7717 — Implement agent-note placement routing and gated promotion](https://github.com/developing-today/code/issues/7717): policy-aware `note_submission` routing and reviewed publication.
- [#7718 — Inventory legacy agent notes under the placement policy](https://github.com/developing-today/code/issues/7718): provenance-preserving classification of the non-document remainder, with memory reference-only.

Before closing #6022, verify that:

- each class in the decision table has a canonical default, lifetime, Atlas treatment, and ID-native qualification rule;
- a repo-specific design and a cross-project finding resolve differently under the one-step test;
- uncertain sensitivity stays staged, and no path auto-commits or auto-promotes;
- all projections preserve canonical identity, source revision/hash, and provenance;
- `research_notes`, goal workspaces, and memory have distinct migration decisions, with memory explicitly reference-only;
- the design is linked from #2048 and #2056, and the follow-on implementation issues are linked on #6022.

## References

- [#6022 — placement design issue](https://github.com/developing-today/code/issues/6022); [#6018 — Atlas ingestion program](https://github.com/developing-today/code/issues/6018)
- [#2048 — Zettelkasten layer](https://github.com/developing-today/code/issues/2048); [#2056 — ID platform epic](https://github.com/developing-today/code/issues/2056)
- [#5545 — writer-set authority](https://github.com/developing-today/code/issues/5545); [#5686 — client-side capability providers](https://github.com/developing-today/code/issues/5686); [#6062 — ID-graph publication format](https://github.com/developing-today/code/issues/6062)
- [#5981 — Atlas documentation sweep](https://github.com/developing-today/code/issues/5981); [#5983 — docs migration classification](https://github.com/developing-today/code/issues/5983)
- [#6267 — note submission pilot](https://github.com/developing-today/code/issues/6267); [#3125 — Atlas provenance registry](https://github.com/developing-today/code/issues/3125); [#5669 — Atlas staging tier](https://github.com/developing-today/code/issues/5669)
- [`planning/DOCUMENTATION_ARCHITECTURE.md`](../../planning/DOCUMENTATION_ARCHITECTURE.md); [`doc/DOCUMENTATION_PROTOCOL.md`](../DOCUMENTATION_PROTOCOL.md)
