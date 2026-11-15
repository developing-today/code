# Atlas Ingestion Brief — Statement Inventory (2026-11-04)

Source: Drew's dictation, main chat, 2026-11-04 23:15:44 CST ("We'll need a long-term plan for semi-automating updates to Atlas ingestion…").
Every statement below is numbered; the filing map at the end records the disposition of each (new issue filed / covered by an existing issue / comment on an existing issue).

Interpretations flagged up front:

- **"that tile cast"** (¶2) is read as **the Zettelkasten** (dezren39/zettel, the Emanote slip-box) — the sentence contrasts it with "repo", matching the standing zettel-vs-repo placement question. Flagged, proceeding.
- **"the can repo"** (¶3) is read as **dezren39/cairn** (the 473-file personal corpus repo; "cairn" → "can" is a standard dictation garble). Flagged, proceeding.
- **"what chart it was in"** (¶5) is read as **what shard** it was in (shard → "chart"). Flagged, proceeding.
- **"tile cast"** aside, no other garbles required interpretation; "settlecasting" in the earlier docs brief was likewise Zettelkasten.

Grounding state (verified via gh, 2026-11-04 ~23:20 CST):

- Shard restoration complete: 31 new TursoDB shards; internal = 28 shards / 107,001 chunks; secret = exactly 1 shard / 638 chunks, secret material only in the 2 secret DBs; production monolith still libSQL; fan-out search [#5755](https://github.com/developing-today/code/issues/5755) (BM25 leg pending there); query front-end [#5773](https://github.com/developing-today/code/issues/5773); parameter surface with full vector logged per run [#5774](https://github.com/developing-today/code/issues/5774); Jaccard rescore + ingest dedup ≥0.9 recorded edge [#5733](https://github.com/developing-today/code/issues/5733); staging tier [#5669](https://github.com/developing-today/code/issues/5669). Storage rulings 2026-11-04: Turso + D1 both, embedded replicas first-class, cold tiers → rustfs; TursoDB for ALL new databases; production stays libSQL for now. Current spend anchor: Turso scaler $24.92/mo yearly.
- **Provenance registry already exists as an issue:** [#3125](https://github.com/developing-today/code/issues/3125) — chunk_occurrences (every chunk records every source locator with spans + observed_at), content-addressed extracts (CAS), `repo@commit:path` pinning, multi-DB resolution. Drew's 2026-10-31 direction there: keep ALL data, only byte-identical dupes cleaned. Several statements below EXTEND #3125 rather than start greenfield; disposition = comment on #3125 + cross-links.
- Provenance spans as mandatory metadata already designed tonight in the zettel/notes brain-dump design (content hash + provenance spans; Turso submission DB as system of record) — same pattern, tie in.
- Docs-migration Atlas sweep [#5981](https://github.com/developing-today/code/issues/5981) enumerates untracked Atlas material; ingestion source enumeration overlaps it heavily.
- Licensing family filed tonight: [#5977](https://github.com/developing-today/code/issues/5977) classifier, [#5979](https://github.com/developing-today/code/issues/5979) SPDX tooling, [#5984](https://github.com/developing-today/code/issues/5984) policy/ToS, [#5986](https://github.com/developing-today/code/issues/5986) AGPL/GPL hosting research.
- Script migration: [#5670](https://github.com/developing-today/code/issues/5670) moves all Atlas/search/embedder scripts into code + fleet binary.

---

## Topic 1 — Semi-automated ingestion operations

- **S1.** There must be a **long-term plan for semi-automating updates to Atlas ingestion** (not ad-hoc manual runs): scheduled and/or triggered updates that keep Atlas current with its sources.
- **S2.** The automation **monitors disk and I/O** — ingestion must watch storage consumption and I/O load and behave accordingly (throttle/pause/report rather than fill disks or swamp the host).
- **S3.** The automation **must not spend more than the plan** — a hard cost ceiling tied to the current paid plan (today: Turso scaler $24.92/mo yearly); ingestion design treats the plan as the budget envelope.
- **S4.** **Source set for automatic ingestion** explicitly includes: reboots / boot records, session logs, chat logs, statistics, and reports — operational byproducts get added to Atlas automatically.
- **S5.** **Usefulness filter:** automation must NOT upload data that is useless to have in Atlas — semi-automated ≠ indiscriminate; sources/classes need an inclusion test.

## Topic 2 — Classification revalidation (task)

- **S6.** A concrete **task to revalidate that there is nothing sensitive in the wrong classification level** — scan the corpus (today: internal 28 shards / 107,001 chunks vs secret exactly 1 shard / 638 chunks, secret material only in the 2 secret DBs) and verify tier placement; findings fixed or reported. This is a task to execute, not only a design.

## Topic 3 — Agent notes placement + ID graph

- **S7.** Figure out **where agent notes go** — a placement rule for agent-authored notes.
- **S8.** Clarify **the difference between the Zettelkasten and the repo** (dictation: "that tile cast"; interpretation flagged above) as note homes — what belongs in each, and why.
- **S9.** **Publishing directly into the ID graph** is an option to design for — notes (and other objects) published as ID-graph objects, not only as files.
- **S10.** **Tie-in:** common methods to connect and show ID relations are already covered in other tickets (writer-set [#5545](https://github.com/developing-today/code/issues/5545), the [#5686](https://github.com/developing-today/code/issues/5686) family, zettel/ID-graph designs); the notes-placement design ties into those rather than inventing a parallel relation model.

## Topic 4 — GitHub corpus extraction

- **S11.** Extract **ALL comments from ALL pull requests and ALL issues in ALL our repos** into Atlas (developing-today/code, the forks, darwin-nix, hatch-personal, hatch, zettel, cairn, … — the full estate).
- **S12.** Also extract from **external repos we have interacted with in the past or rely on** (upstream issues/PRs/discussions we filed, commented on, or depend on) — a defined external set, not the whole of GitHub.

## Topic 5 — External-content licensing gate

- **S13.** **Determine the licensing position for external-repo content BEFORE external ingestion is built**: can we copy a comment from a GitHub discussion? Is it Creative Commons? (Facts to establish: GitHub ToS grants GitHub a license; authors keep copyright; what may a third party store, embed, and redistribute, for our own comments vs others' comments, issues/PRs vs Discussions.) Design may proceed; the external build is gated on the answer.
- **S14.** Sequencing per Drew: figure the licensing out **before all of this is built, but after it is somewhat designed** — i.e., the gate sits between design and build for the external corpus specifically (our own repos are not gated).

## Topic 6 — First-pass corpus

- **S15.** After design (+ the S13 gate for external parts), do a **first pass adding all these data points into Atlas**: **all the code repos, all the docs we can find, and the cairn repo** ("the can repo"; interpretation flagged above). A concrete first ingestion run, enumerated.

## Topic 7 — Content versioning + retention

- **S16.** Versioning must be understood well enough to **add new versions of files** as first-class objects (a changed file = a new version, not an overwrite).
- **S17.** Versioning must **understand renames** — a renamed file's new version links to its old self, not a fresh unrelated object.
- **S18.** **Keep the old and the new**, but resolution defaults to **"the max" — the newest version of every file** (search/retrieval can ask for latest and get it; history remains addressable).
- **S19.** Old versions **roll out to cold storage** over time (per the 2026-11-04 storage ruling: cold tiers → rustfs) — a lifecycle, not permanent hot residence.
- **S20.** Old versions are **eventually removed** under an explicit policy with three named triggers: **by date** (age), **by number of increments** (N newer versions exist), or **by lack of access** (never / no longer ever retrieved).

## Topic 8 — Retrieval statistics

- **S21.** Keep **stats on files/embeds that come in from a shard on a query and make it into the top K** — per-file/per-embed shard-stage survival counts.
- **S22.** When shard results are **merged and rescored**, keep stats on **which results make the top 1, 2, 3, and top 1–5** — final-rank distribution per file/embed, feeding the retention and search-evolution uses (S27).

## Topic 9 — Result feedback + query trace

- **S23.** Build **simple feedback** on search results: "this was exactly what I needed" / "this was terrible" / everything in between.
- **S24.** **Bare minimum: thumbs up / thumbs down on all searches**, usable by any user with no instruction ("any user knows exactly how to add a bare minimum").
- **S25.** Feedback may carry an **optional comment**; longer feedback — **up to an essay** on why a result was terrible for a query — is explicitly welcomed ("we should thank them for their time"); **we normalize comments ourselves** afterwards based on context and detail.
- **S26.** **Result↔query traceability:** results carry IDs (assumed to exist — verify); any result must be associable to its query with the full trace: **the scores it got at each stage**, **which shard it was in** ("chart"; interpretation flagged above), **the exact state of what the embed contained**, and **its metadata**.
- **S27.** Feedback + stats have named uses beyond performance/benchmarks: **guide evolutions of Atlas** — **what files are kept / not kept** (feeds S20), **changes to search patterns**, and system evolution generally.

## Topic 10 — Provenance standard

- **S28.** It is **very important to have a standard way to understand a file's provenance: when and where it came from** — one provenance standard across all sources.
- **S29.** Provenance must support **associating a file with its past and future selves** (the version lineage of S16–S18 expressed in provenance terms).
- **S30.** For git files this is simple **if commits are done in the right way — renames show up as renames instead of inserts and deletes**; ingestion may need commit-hygiene expectations on source repos.
- **S31.** Where git lineage is absent, **searches for segments of files within Atlas** may be needed to **make associations** (link a loose file/embed to its source/version relatives).
- **S32.** Sources must carry **the metadata to begin with**, so that files that don't associate (and duplicates) **can be logged** — unassociated/duplicate is a recorded state, not silent loss.
- **S33.** **Marking format:** a solid system for marking **`repo@hash` with path** (repo@hash:path), **or source-system marks like "OpenCode DB @ AMD laptop on this day"** — every embed's origin mark names system + location + time.

## Topic 11 — Existing-embed provenance audit (task)

- **S34.** **Scan all existing embeds** and determine whether they use the **appropriate path or details** — a concrete audit task over the current corpus (107,639 chunks), producing a defect list against the S33 standard.

## Topic 12 — Delta-only ingestion

- **S35.** A way to say **"we only take the deltas"**: re-ingesting a largely-unchanged corpus adds only what is new. Drew's numbers: a 21 GB corpus of which **20.5 GB is already in Atlas** must NOT trigger updating every file, retagging, or editing all metadata to a new year/date.
- **S36.** What lands in a delta run is exactly: **all the new files and the edits to the old files**. Edits carry a **new date** and **repeat some of the data that overlaps** (overlap repetition is accepted, not engineered away).

## Topic 13 — Offset / overlap embeds

- **S37.** A system may **accept embeds with different overlaps that are not along boundary lines**: an embed is accepted with a recorded span into the current file version — Drew's example: the embed's content, joined to the most recent file, runs **from index 17 to 241 because indices 0–16 matched a prior embed**; a different embed covers a different end; everything after 241 matched another embed — **only this one diff is new**.
- **S38.** **Search-time join semantics:** a search hit shows the **full paragraph**; when results are joined together, lines that were merged **do not double up** — the join reconstructs the source text once, deduplicating the overlapped spans.

## Topic 14 — Horizons / program shape

- **S39.** **Same directions as the other recent briefs:** break this down, **don't lose anything**, make **a bunch of smaller designs**, plus **a larger design that expands depth and scope wherever the research takes us**, thinking ahead to **v2, v3, v4**.
- **S40.** The larger design should push beyond the brief where research justifies it (candidate expansions recorded in the expansive-design issue: feedback-driven ranker tuning, provenance-complete resolution APIs, cross-source lineage graphs, classification-aware retrieval, cost-aware tiering).

---

## Filing map (filed 2026-11-04, all in developing-today/code)

Program tracker: [#6018](https://github.com/developing-today/code/issues/6018) — Atlas ingestion program (S39's tracking home; child list at [comment 6650433323](https://github.com/developing-today/code/issues/6018#issuecomment-6650433323)).

| Statements                                                                     | Disposition                                                                                                                                                                                               |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S1–S5 (ingestion ops)                                                          | NEW design [#6019](https://github.com/developing-today/code/issues/6019)                                                                                                                                  |
| S6 (classification revalidation)                                               | NEW task [#6021](https://github.com/developing-today/code/issues/6021)                                                                                                                                    |
| S7–S10 (agent notes / zettel / ID graph)                                       | NEW design [#6022](https://github.com/developing-today/code/issues/6022); tie-ins #5545/#5686/#5955 + brain-dump envelope design named in body                                                            |
| S11, S12, S14 (GitHub corpus + sequencing)                                     | NEW task [#6023](https://github.com/developing-today/code/issues/6023) (external half build-gated on #6024)                                                                                               |
| S13 (external licensing question)                                              | NEW research [#6024](https://github.com/developing-today/code/issues/6024), licensing family; cross-link comment on [#5986](https://github.com/developing-today/code/issues/5986#issuecomment-6650432159) |
| S15 (first pass)                                                               | NEW task [#6025](https://github.com/developing-today/code/issues/6025); doc sources from [#5981](https://github.com/developing-today/code/issues/5981) (cross-link comment 6650432839)                    |
| S16–S20 (versioning + retention)                                               | NEW design [#6026](https://github.com/developing-today/code/issues/6026) (incl. the #3125 keep-all tension, stated in body)                                                                               |
| S21–S22 (retrieval stats)                                                      | NEW design [#6027](https://github.com/developing-today/code/issues/6027) (with S23–S27)                                                                                                                   |
| S23–S25 (feedback)                                                             | NEW design [#6027](https://github.com/developing-today/code/issues/6027)                                                                                                                                  |
| S26 (result↔query trace)                                                       | NEW design [#6027](https://github.com/developing-today/code/issues/6027)                                                                                                                                  |
| S27 (feedback uses → retention/search evolution)                               | NEW design [#6027](https://github.com/developing-today/code/issues/6027), consumed by [#6026](https://github.com/developing-today/code/issues/6026)                                                       |
| S28, S31, S32, S33 (provenance standard + marks + association + logged states) | COVERED-BY [#3125](https://github.com/developing-today/code/issues/3125) + extension comment [6650431155](https://github.com/developing-today/code/issues/3125#issuecomment-6650431155)                   |
| S29 (past/future selves)                                                       | [#3125](https://github.com/developing-today/code/issues/3125) comment (identity shared) + design [#6026](https://github.com/developing-today/code/issues/6026)                                            |
| S30 (git rename hygiene)                                                       | Design [#6026](https://github.com/developing-today/code/issues/6026)                                                                                                                                      |
| S34 (existing-embed audit)                                                     | NEW task [#6029](https://github.com/developing-today/code/issues/6029)                                                                                                                                    |
| S35–S36 (delta-only)                                                           | NEW design [#6028](https://github.com/developing-today/code/issues/6028)                                                                                                                                  |
| S37–S38 (offset embeds + joins)                                                | NEW design [#6028](https://github.com/developing-today/code/issues/6028)                                                                                                                                  |
| S39–S40 (program shape + expansive design)                                     | Tracker [#6018](https://github.com/developing-today/code/issues/6018) + NEW expansive design [#6020](https://github.com/developing-today/code/issues/6020)                                                |

**Unmapped statements: ZERO.** New issues: 12 (1 tracker, 6 designs, 4 tasks, 1 licensing-gate research). Comments on existing issues: 4 (#3125, #5986, #5981, #6018 child list).
