# Design: Atlas deletion propagation — source deleted / repo privated → next-sync corpus purge

- **Issue:** #6054 (parent #6024, licensing gate for the external corpus;
  Cluster 6 follow-up 33). Consumers: the GitHub extraction task #6023
  and the trace store (#6027).
- **Status:** design record. No sync, purge, or trace-store code is
  written or enabled by this document; implementation is a follow-on
  split (§8). The policy defaults in §5 are interim: they stand until
  parent #6024 emits its D.6 conclusion, and they are expressed as a
  policy-layer rule version precisely so that answer can change them
  without a re-ingest.
- **Inputs consumed, not re-defined:**
  - The retention record
    (`doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`)
    §4.4 — retirement is a report before it is a deletion, and every
    retired version leaves a tombstone record (file id, version seq,
    content hash, trigger, rule, run id); §5.2 — a tombstone head
    excludes a file from the newest-resolution view and the previous
    version does not resurface. This record does not re-decide either
    ruling; §4 below is about a different act (purge) that happens to
    reuse the same tombstone discipline.
  - The license-capture record
    (`doc/2026-11-08T09-15-51Z_design_atlas_license_capture_ingest/`)
    §2 — provenance records inputs, not verdicts: `repo_id`,
    `license_spdx`, `d6_applicable`, and the `zone` enum
    (`own | participated_external | general_external | github_docs`)
    on every ingest generation; §5 — policy is a pure function over
    recorded rows, and its reverse query already names the purge
    candidate shape: repos whose latest generation is a tombstone
    generation cited by a `detection_source = manual` deletion sync.
    This record generalises that candidate shape from manual to
    sync-detected (§3) and supplies the policy function it feeds (§5).
  - The #6054 finding itself: GitHub ToS §D.4–D.8 licenses end when
    the user removes their content, and the fork exception does not
    cover an external corpus — a defensible corpus honours source
    deletions. Triage (task 10420, 2026-11-08) verified the issue is
    real and current, and narrowed this record to the four open
    questions decided in §3–§6.

## 1. Problem

Atlas ingests external GitHub content (issues, PRs, comments, files)
into a searchable corpus of text chunks and vectors. Ingest is
generation-based and additive: today nothing removes a corpus item
when its source disappears. That leaves three distinct gaps:

1. **No detection.** A sync cannot say which previously-ingested
   items have vanished at the source, per source class.
2. **No purge semantics.** Even a known deletion has no defined
   effect on chunk text, vectors, and derived indexes — and the one
   removal mechanism that does exist on paper, retention retirement,
   is the wrong tool: it is space/policy-driven, report-first, and
   keeps versions until a rule fires.
3. **No interpretability after removal.** A served-result trace in
   the trace store (#6027) that references purged content must still
   resolve to an explanation, without retaining the content.

And one policy question that cuts across all three: our own repos
are not external. Drew may keep his own deleted comments; third-party
comments in our repos follow the external rule. The switch for that
is §5 (question d).

## 2. Vocabulary and invariants

- **Source item.** One ingested unit with a stable source identity:
  `(repo_id, source_class, source_item_id)`. Source classes are the
  classes the extraction task (#6023) actually ingests: `repo`,
  `issue`, `pull_request`, `issue_comment`, `review_comment`, `file`.
- **Generation.** One sync's ingest generation, as in the
  license-capture record: every generation carries a full manifest
  of the source items it observed for its scope.
- **Vanished.** A source item that was present in a previous
  generation's manifest and is confirmed gone at the source under
  the rules of §3. _Suspected_ is a different, weaker state (§3.3);
  the two are never conflated in records or in serving.
- **Purge.** The irreversible removal of §4. Purge is not retirement:
  retirement (retention record §4) is reversible-in-effect — versions
  remain stored until a conjunction of trigger and class rule retires
  them, and a tombstone preserves their hashes and provenance.
  Purge removes the content itself, on a rights signal, and the only
  thing that remains is the §6 tombstone.

Invariants (property-shaped, per the fleet testing doctrine; the
implementation follow-on names its oracles against these):

- **I1 — No purge without confirmation.** No item is purged on a
  single listing absence. Every purge traces to a §3.3 confirmation.
- **I2 — Serving safety precedes purge.** A suspected-vanished item
  leaves the default serving view at first absence, before purge is
  confirmed; a confirmed purge therefore never widens exposure, it
  only shrinks storage.
- **I3 — Purge is total per item.** After a purge completes, no chunk
  text, vector, or index entry for any version of the item remains
  in the serving corpus. Partial purge is a failed purge run, not a
  state.
- **I4 — Tombstones contain no content.** A §6 tombstone carries ids,
  a content hash, and provenance — never text, snippets, or vectors.
- **I5 — Old content never resurfaces.** If a source reappears (a
  repo made public again), its content is ingested fresh under the
  then-current provenance as a new lineage. A purge is never undone
  by a restore, a re-sync, or a backup replay (§4.4).

## 3. Detection — per source class, on sync (question a)

### 3.1 The diff

Each sync compares its freshly-built manifest against the previous
generation's manifest for the same scope. The diff yields three sets:
`added`, `changed` (same source identity, different content hash),
and `absent` (in the previous manifest, not in this one). Only
`absent` feeds deletion detection. `changed` is ordinary versioning
under the retention record and is never a purge signal on its own.

### 3.2 What "vanished" means, per class

| Source class                                  | Delete signal                                                                                                                                                                                          | Explicitly NOT a delete signal                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repo`                                        | Repo GET returns 404 or 410 for an id in the previous manifest. Private and deleted are indistinguishable from outside; both are the same corpus event (§3.4).                                         | A rename or transfer: the API redirects to the new full name and the repo id persists — that is a `changed` manifest row (new name recorded), not an absence.                                                                                                                                                                                                                    |
| `issue`, `pull_request`                       | Item GET returns 404/410 on confirmation probe. Issues and PRs are rarely hard-deleted, but a deleted fork or a removed spam PR produces exactly this.                                                 | State changes (`closed`, `merged`), label edits, locking. Those are `changed` rows.                                                                                                                                                                                                                                                                                              |
| `issue_comment`, `review_comment`             | Comment id absent from the parent's comment listing AND a direct GET of the comment returns 404.                                                                                                       | **An edit.** The id persists, `edited_at` advances, the body hash changes: that is a new version under the retention record, and the previous version's fate is a retention question, not a purge question. A comment the author edited to remove its substance is handled by versioning + newest-resolution (the edited text is what serves), not by pretending it was deleted. |
| `issue_comment`, `review_comment` (minimised) | —                                                                                                                                                                                                      | A minimised/hidden comment still exists at the source. It is recorded as `suppressed_at_source` on the manifest row and excluded from the default serving view (the source has withdrawn it from public view), but it is not purged: minimisation is reversible by the source, deletion is not.                                                                                  |
| `file`                                        | Path absent from the repo tree at the synced head AND the repo itself still resolves. File deletion is an ordinary commit away, so the tree diff is the signal; no per-file probe exists or is needed. | A rename: tree diff shows delete+add; the extraction layer's existing rename handling decides lineage, and the old path's versions follow retention, not purge — the content still exists at the source under the new path.                                                                                                                                                      |

### 3.3 Confirmation — the two-step rule (I1, I2)

Listing absence is not proof: pagination edges, permission flaps,
and API incidents all manufacture absences. Detection therefore has
two steps:

1. **First absence → suspected.** The item is recorded as a
   detection row (shape below) with `state = suspected`, and it is
   excluded from the default serving view immediately (I2). Its
   storage is untouched.
2. **Confirmation → purge-eligible.** Either (a) a direct probe of
   the item returns 404/410 (`detection_source = sync_probe`), or
   (b) the item is absent again in the next generation for the same
   scope (`detection_source = sync_diff`, the two-generation rule).
   Only then does §4 run. A manual deletion sync remains a third
   source (`detection_source = manual`), which is the candidate
   shape the license-capture record §5 already queries in reverse;
   manual rows still require step (a)'s probe or an operator-signed
   manifest — "manual" names who initiated the sync, not a weaker
   evidence standard.

If a suspected item reappears before confirmation, the detection
row closes as `state = refuted` and the item returns to the serving
view. Refuted rows are kept: a source class that flaps produces
evidence about the source, and the flap rate is the metric that
would justify tightening or loosening the two-generation rule.

**Detection row** (append-only, one per item per episode):

| field                                             | notes                                                                                              |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `repo_id`, `source_class`, `source_item_id`       | the §2 identity                                                                                    |
| `zone`                                            | copied from the item's latest provenance generation — policy (§5) reads the row, not a live lookup |
| `author_login`, `author_is_own_account`           | provenance inputs recorded at ingest (needed by §5's authorship branch); inputs, not verdicts      |
| `detection_source`                                | `sync_diff \| sync_probe \| manual`                                                                |
| `first_absent_generation`, `confirmed_generation` | generation ids; equal only on a probe confirmation                                                 |
| `detected_at`, `confirmed_at`                     | timestamps                                                                                         |
| `state`                                           | `suspected \| confirmed \| refuted \| purged`                                                      |

### 3.4 Repo-unavailable is a bulk event, not N item events

When a repo itself vanishes (§3.2, `repo` row), the sync writes one
tombstone generation for the repo — the exact shape the
license-capture record §5 reverse query already looks for
(`license_spdx` NULL, `zone` unchanged) — with
`detection_source` set by how it was confirmed. Every item of that
repo inherits `confirmed` from that single event; per-item probes
of a repo that no longer resolves would be noise. If the repo later
resolves again, I5 applies: fresh ingest, new lineage, and the
repo-level detection rows close as `refuted` for the record.

## 4. Purge semantics — external corpus (question b)

### 4.1 What purge removes

For a confirmed item, purge removes, for **every version** of that
item: chunk text rows, embedding vectors, and every derived index
entry (full-text and vector indexes alike). Nothing is archived,
exported, or moved to cold storage first. There is no restore path:
the §6 tombstone is the only remainder, and it carries no content
(I4).

### 4.2 How purge differs from retention retirement

|               | Retention retirement (sibling record)                                                  | Deletion-propagation purge (this record)                                                                                                                                                                                                                                                                                                           |
| ------------- | -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Driver        | Storage policy: age, increments, access (§4.2/§4.3 of that record)                     | A rights signal: the source withdrew the content                                                                                                                                                                                                                                                                                                   |
| Gate          | Conjunction — a trigger AND a class rule must both fire                                | §3.3 confirmation AND the §5 policy switch for the item's zone                                                                                                                                                                                                                                                                                     |
| Report-first  | Dry-run reporter; deletion runs are separate, logged acts                              | A purge manifest is written per sync (items, detection row ids, rule version) and is part of the sync's log, but purge executes in the same run after confirmation — the two-generation rule and the serving exclusion (I2) are the delay a report-first gate would exist to provide, and a human approval gate cannot scale to per-comment volume |
| Remainder     | Tombstone with content hash; versions retained until retired                           | Tombstone with content hash (§6); **no versions retained**                                                                                                                                                                                                                                                                                         |
| Reversibility | Retired versions are gone, but un-retired history is addressable (§5.3 of that record) | None. Reappearance means fresh ingest (I5), never un-purge                                                                                                                                                                                                                                                                                         |

Reconciliation with the keep-all rule (#3125, as evolved by the
retention record §4.1): keep-all governs content we hold at the
source's pleasure. A confirmed source deletion withdraws that
pleasure for the external corpus; retaining the text and vectors
afterwards would be keeping exactly the thing the rule's evolution
never covered. Purge is therefore not an exception carved out of
keep-all — it is outside its scope, and §5 decides zone by zone
where that scope boundary falls for our own repos.

### 4.3 Execution properties

- **Idempotent and resumable.** A purge run is keyed by
  `(purge_run_id, source_item_id)`; re-running a run completes
  unfinished items and skips finished ones. A run that stops
  mid-item leaves the item in `state = confirmed`, never in a
  half-purged serving state: the item was already out of the
  serving view since suspicion (I2), and I3 is judged at run
  completion, with the run's manifest as the checklist.
- **Order.** Vectors and index entries are removed before chunk
  text within an item, so no intermediate state can serve content
  whose text is gone or index content whose text remains.
- **Scope check at execution.** The purge executor re-reads the
  item's detection row and provenance generation at execution time
  and applies the current §5 rule version — a policy change between
  detection and execution is honoured, because policy is a function
  over recorded rows, not a verdict stamped at detection.

### 4.4 Backups and restores

Purged content may still exist inside backup snapshots until they
rotate out under the existing PITR/retention decision
(`doc/2026-11-05T15-50-41Z_decision_atlas_pitr_vs_retention/`).
That is accepted, with one hard rule: **a restore replays the
tombstone set before the corpus serves.** The §6 tombstones are
append-only and stored with the trace store, outside the corpus
backup rotation, so a restore can always re-apply every purge that
happened after the snapshot was taken. A restored corpus that
serves purged content because tombstone replay was skipped is a
defect of the restore, graded like a fresh leak.

## 5. The per-zone policy switch (question d)

### 5.1 Shape

Deletion policy is a policy-layer rule, versioned like the
license-capture record's `policy_eval` — a pure function over
recorded provenance inputs:

```
deletion_policy(zone, author_is_own_account, rule_version)
    -> purge | keep
```

The function's output is evaluated twice in the item's life: at
detection (to predict) and at execution (to act, §4.3). Changing
the policy is a policy-layer act that writes a new rule version;
re-evaluation over existing detection rows is the license-capture
record §5 query shape run in reverse — rows whose
`deletion_policy` output differs between the current and the new
rule version are the work list. No re-ingest is ever required to
change a deletion decision for content not yet purged; for content
already purged, no rule change can undo it (I5), which is why the
defaults below err toward `keep` exactly where Drew's own authorship
is involved.

### 5.2 Defaults (interim, pending #6024's D.6 conclusion)

| zone                    | own-authored item                                                                                                                                                                                                | third-party-authored item                                                                          |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `own`                   | **keep** — Drew may keep his own deleted comments in his own repos; the item leaves the default serving view (it was deleted at the source) but text and vectors are retained under the retention record's rules | **purge** — third-party comments in our repos follow the external rule (the #6054 scope statement) |
| `participated_external` | **purge** — a deletion at the source is the author's or the repo owner's act; honouring it costs nothing and the corpus has no claim                                                                             | **purge**                                                                                          |
| `general_external`      | **purge**                                                                                                                                                                                                        | **purge**                                                                                          |
| `github_docs`           | **purge**                                                                                                                                                                                                        | **purge**                                                                                          |

Two clarifications:

- `keep` in the `own` row means _exempt from purge_, not _immune
  from everything_: the item is still excluded from the default
  serving view from first suspicion onward (I2 applies to every
  zone), and its retained versions age under the retention record
  like any other version. The switch decides storage fate, never
  serving fate — a deleted-at-source item is never served from the
  default view again in any zone.
- Authorship is judged by `author_is_own_account`, recorded at
  ingest. Content ingested before authorship recording exists has
  `author_is_own_account = unknown`, which evaluates as
  third-party (purge) in external zones and as **keep** in the
  `own` zone — in the one zone where a wrong guess destroys Drew's
  own content, unknown errs toward keeping; everywhere else it
  errs toward honouring the deletion.

### 5.3 The #6024 gate

Parent #6024 has not emitted its D.6 conclusion. That conclusion
may rule that some external content was never licensed for corpus
retention at all, which would widen purge from
confirmed-deletion-only to whole classes or zones. That widening,
if it comes, lands exactly here: a new rule version of
`deletion_policy` (plus, if needed, a widened detection source),
evaluated over recorded rows per §5.1. Nothing in §3–§4 or §6 needs
to change for it, and nothing in this record pre-empts it: the
defaults above govern only deletions the source itself performed.

## 6. The trace-log tombstone (question c)

The trace store (#6027) logs served results: which items, which
versions, in response to what. After a purge, a trace referencing
the item must resolve to an explanation, not a dangling reference
and not retained content. The mechanism is the tombstone — the same
discipline as the retention record §4.4, with fields narrowed to
what a rights-driven removal may keep:

| field                                                     | notes                                                                                                                                                                           |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `corpus_item_id`, `version_ids`                           | every purged version id of the item                                                                                                                                             |
| `content_hash`                                            | hash of the head version at purge time — proves what was there without retaining it (I4); per-version hashes are already carried by retention tombstones for un-purged versions |
| `repo_id`, `zone`                                         | provenance context                                                                                                                                                              |
| `reason`                                                  | `source_deleted \| repo_unavailable \| policy_6024` (the third value is reserved for the §5.3 widening; unused until that rule version exists)                                  |
| `detection_row_id`, `purge_run_id`, `policy_rule_version` | the audit chain: how it was detected, which run removed it, under which policy                                                                                                  |
| `purged_at`                                               | timestamp                                                                                                                                                                       |

Resolution rule: a trace lookup that reaches a tombstone renders
**"purged under deletion propagation on \<date\> (\<reason\>)"** —
the same shape the retention record gives a retired version
("retired under policy"), with the reason vocabulary of this
record. The tombstone carries no snippet, no title, and no author
display name: ids and the hash are the audit floor, and anything
more would re-retain a shadow of the purged content.

Tombstones are append-only, are never themselves purged or retired,
and live with the trace store — outside the corpus backup rotation —
because §4.4's restore rule consumes them.

## 7. What this design does not decide

- The D.6 licensing conclusion itself — that is #6024's, and §5.3
  is the socket it plugs into.
- Retention of versions that were edited, superseded, or retired
  for space/policy — the retention record's ground, reused here
  only by reference.
- License capture and provenance recording — the license-capture
  record's ground; this record consumes its rows.
- The extraction mechanics of #6023 (how a sync fetches and diffs)
  beyond the manifest contract §3.1 requires of it.
- Serving-view mechanics for historical addressing (`as_of`,
  explicit version pins) beyond I2/I5: an explicit historical pin
  at a purged item resolves to the §6 tombstone, never to content.

## 8. Implementation split (follow-ons)

1. **Extraction (#6023):** emit the §3.1 manifest diff per sync and
   write §3.3 detection rows; implement the confirmation probe.
2. **Policy layer:** implement `deletion_policy` (§5.1) as a
   versioned rule over provenance + detection rows, with the
   reverse re-evaluation query for rule changes.
3. **Purge executor:** §4.3 semantics against chunk, vector, and
   index stores, keyed and resumable, writing the per-sync purge
   manifest.
4. **Trace store (#6027):** the §6 tombstone table, the resolution
   rendering, and tombstone replay as a restore precondition (§4.4).
5. **Serving:** suspected-item exclusion (I2) in the
   newest-resolution view, alongside the retention record §5.2
   tombstone-head rule.

Each follow-on is design-complete against this record; none of them
requires a further decision from #6024 except a possible §5.2
defaults change under §5.3.
