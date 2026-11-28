# HYD-07 — Operation/session attribution and external-only semantics

Issue: [#9509](https://github.com/developing-today/code/issues/9509)
(slice `HYD-07` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.** This
record elaborates the attribution slice of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`, PR
#9225) into the classifier contract an implementation issue can be
scoped against. It does not re-decide anything that package settled,
and it does not authorize code, schema, migration, deployment, or live
acceptance; those remain separately gated stages (see §8).

Governing decisions consumed, not re-opened:

- **FDR-PMO-004** (hydration, cursors, attribution): "Attribute
  Fleet-caused activity through operation/session mapping and returned
  remote IDs; shared GitHub actor alone is insufficient. Unknown
  attribution stays uncertain and is included by default in
  external-only results." Its attribution oracle — a shared bot
  username across sessions without operation mapping remains uncertain
  and included in external-only — is this slice's acceptance test.
- **Component #9207 §4**: external attribution maps Fleet
  `operation_id`/`session_id`/`outbox_id` to remote returned IDs and
  content markers; for edits without remote ID mapping, classify
  origin `uncertain`; `--external-only` includes uncertain by default
  and reports confidence.
- **AUTH-05** (#9208): `src/work/authority.rs` owns the trusted
  principal/provenance-record schema. This slice consumes those
  records; it does not define a second provenance schema and there is
  no second activity classifier.

Verified state at writing (2026-11-23, origin/main `7fecf882f`):
`pkgs/fleet/crates/fleet-queue/src/work/activity.rs` does not exist,
the acceptance test `pmo_hyd_external_only_includes_uncertain_shared_actor`
is not implemented anywhere on main, and the granular prerequisites
REL-03 ([#9513](https://github.com/developing-today/code/issues/9513)),
CLI-05 ([#9502](https://github.com/developing-today/code/issues/9502)),
and HYD-06 ([#9508](https://github.com/developing-today/code/issues/9508))
are all still open. The design below therefore specifies contracts
against those slices' stated outputs, not against running code.

## 1. Intent

`fleet work changes <work-id> --since-start --external-only` (C-R118)
answers the question an agent actually asks mid-task: *what happened
on this work item that did not originate from my session?* The naive
implementation — filter out everything whose GitHub actor is the
Fleet bot login — is wrong in both directions:

- **False exclusion.** Every Fleet session, the PM Stack writers, and
  the hosted doors write through a small set of shared App/bot
  logins. Actor filtering hides other sessions' writes, other Fleet
  components' writes, and a human driving the same App installation —
  exactly the concurrent activity `--external-only` exists to surface.
- **False attribution.** A shared login never establishes *which*
  session caused an event (C-R121: session attribution requires Fleet
  operation/session IDs, not only a GitHub username).

So attribution must rest on correlation Fleet itself records when it
performs a remote write, never on who the source says acted.

## 2. Identity model

### 2.1 Fleet-side keys

| Key | Owner | Meaning |
| --- | --- | --- |
| `operation_id` | REL-03 (#9513), carried in the shared command/result envelope (`src/work/contracts.rs`) | One logical Fleet operation; idempotency identity and stable effect receipt hang off it. |
| `session_id` | #9206 session identity, surfaced in lifecycle receipts (CLI-05, #9502) | The agent session that issued the operation. Opaque; never derived from an actor name. |
| `outbox_id` | #9209 outbox (FDR-PMO-006) | The durable outbox record for a remote write that is delivered asynchronously rather than inside the operation's own call. |

### 2.2 Remote-side keys

- **Remote object ID**: the source-native ID the remote returns for
  the created object (comment ID, review ID, issue/PR number scoped
  by canonical repository identity, check-run ID). This is the ID
  HYD-06's normalized activity records already key on, kept distinct
  from revision/`updated_at`, delivery ID, and occurrence/observation
  times (FDR-PMO-004).
- **Content marker**: a nonce/marker embedded in the written body by
  the writing path (the pattern the content-creation ledger already
  uses). A marker is secondary evidence: it corroborates a mapping,
  it never substitutes for one, because bodies are copyable and
  quotable by anyone.

### 2.3 The attribution map

The map is the slice's only new durable concept. One entry is written
**at write time, from the write's own receipt** — never reconstructed
later by inference:

```text
AttributionEntry {
  operation_id, session_id, outbox_id?,   // Fleet side; outbox_id when applicable
  source_ref,                              // canonical source (repo identity, endpoint class)
  remote_id,                               // source-native ID returned by the write
  remote_revision_at_write,                // revision/updated marker observed at write, if any
  recorded_at,                             // when Fleet recorded the entry
}
```

Rules:

- **Write-before-observe.** An entry exists only because a Fleet
  write path recorded the remote ID its call returned (directly, or
  via the outbox delivery receipt). Hydration/ingest never creates
  entries; it only reads them.
- **Append-only.** Entries are never edited or deleted by the
  classifier. A wrong entry is superseded by recording the correction
  as a new fact in the operation's receipt trail (REL-03), not by
  mutating history.
- **Scoped match.** A lookup matches on `(source_ref, remote_id)`
  together. The same numeric ID in a different repository is not a
  hit. An entry whose `remote_revision_at_write` is known and whose
  event revision differs is evidence about the *creation*, not about
  the later revision (see §4 on edits).
- **Co-located with the activity store.** The map lives with
  HYD-06's normalized records in `src/work/activity.rs`'s domain (the
  shared-file sequencing edge HYD-06 → HYD-07 on that file stands).
  HYD-06 owns records/revisions/change projection; HYD-07 owns the
  mapping and the classifier over them. Retention follows the
  activity records the map classifies: an entry must not expire
  while events it could classify are still served.

## 3. Classification

Every normalized activity event is classified relative to a
**requesting session** (the session asking, identified by
`--session-id` / the caller's session context). Classification is a
pure function of `(event, map view, requesting session)` producing:

```text
origin     ∈ { own_session, fleet_other, external, uncertain }
confidence ∈ { mapped, marked, distinct_actor, none }
evidence   = the map entry / marker / provenance fact relied on, or its absence
```

Decision order — first matching rule wins:

1. **Own session (`mapped`).** A map entry matches the event's
   `(source_ref, remote_id)` and its `session_id` equals the
   requesting session. This is the *only* rule that may exclude an
   event from `--external-only` output.
2. **Other Fleet (`mapped`/`marked`).** A map entry matches but names
   a different session (or an operation with no session, e.g. a PM
   Stack writer), or the event carries a verifiable Fleet content
   marker with no map entry. Reported as Fleet-caused but *not* the
   requesting session's — still included in `--external-only` output,
   labeled, because C-R118 asks what did not originate from *that
   session*.
3. **External (`distinct_actor`).** No Fleet evidence, and the
   event's provenance (AUTH-05 record) positively names a principal
   that is not a shared Fleet identity — a human user, another
   system. Actor identity is used only in this direction: to say
   "positively not ours", never to say "ours".
4. **Uncertain (`none`).** Everything else: shared Fleet actor with
   no map entry and no marker; an edit whose creation was mapped but
   whose revision was not (§4); an event whose map lookup could not
   be performed (map unavailable, source not yet reconciled); any
   legacy event predating the map. Uncertain is a first-class,
   reported outcome — never silently folded into either side
   (C-R122).

Two prohibitions, restating the settled decisions as classifier
invariants:

- **Username never assigns session** (slice migration rule). No code
  path may move an event out of `uncertain` because of its actor
  login, in either direction.
- **Coincidence is not evidence.** Identical body text, identical
  timestamps, or same-second posting by a human and a session do not
  attribute anything (negative case: coincidental same-body human
  comment stays `external`/`uncertain`, never `own_session`).

## 4. Edits, revisions, and stale mappings

Attribution attaches to **events**, not objects, and HYD-06's model
keeps an edit as a new revision of the same logical item:

- A mapped creation attributes the creation event only. A later edit
  event is classified on its own evidence; with no mapping for the
  edit it is `uncertain` even when the creation was `own_session`
  (component #9207 §4: "For edits without remote ID mapping, classify
  origin `uncertain`"). This is deliberate: an edit is precisely
  where a second actor — human or session — most often enters.
- A write path that performs an edit on Fleet's behalf records a map
  entry for that edit the same way as for a creation, keyed by the
  same `(source_ref, remote_id)` with the edit's revision; the
  classifier can then attribute that revision.
- **Stale mapping** (named negative case): if the remote ID was
  re-used, moved, or the object was deleted and re-created, a map hit
  whose recorded revision/context no longer matches the event's
  object identity degrades to `uncertain`, not `own_session`. When
  in doubt the classifier must fall toward `uncertain`: a false
  exclusion hides real activity, a false inclusion only adds a
  labeled row.
- **Changed remote ID** (AUTH-05 negative case): if the source
  re-keys an object, prior entries simply stop matching; the events
  classify `uncertain` until a write path records the new ID.

## 5. `--external-only` semantics

For `fleet work changes` and the other commands allocated to this
slice (`activity.list`, `activity.since`, `queue.timeline`,
`work.changes`, `github.pr.reviews`; flags `flag.external-only`,
`flag.session-id`, `flag.since-start`, `flag.start`,
`flag.require-clean-reconciliation` — full inventory in
`census/command-slice-map.jsonl`):

- Without `--external-only`: all events in the window, each carrying
  `origin`, `confidence`, and evidence. Classification is annotation,
  not filtering.
- With `--external-only`: the result is `fleet_other ∪ external ∪
  uncertain` — everything not positively attributed to the requesting
  session. Uncertain rows are included **by default** (C-R122) and
  are labeled `uncertain`, never relabeled `external`; consumers can
  distinguish "definitely not ours" from "cannot prove whose".
- Windows are session-relative as the flags state: `--since-start`
  means the requesting session's start; `--since-checkpoint` and
  `--since ISO` compose with the filter unchanged — the window picks
  events, the classifier labels them, the filter drops only rule-1
  rows.
- `--require-clean-reconciliation`: when set, a result whose
  underlying source coverage is not cleanly reconciled (per HYD-06's
  reconciliation state — unprocessed pages, known deletion gaps,
  map unavailable) is returned as the shared outcome `partial` with
  the gap named, rather than as a plain list that merely *looks*
  complete. Without the flag, the same rows are returned with their
  per-row confidence and a coverage note in the summary. The flag
  changes how gaps are surfaced, never which rows classify which way.
- A session's own prior write may still be *visible* in hydration
  (it is part of the record); visibility is not attribution. It is
  excluded from `--external-only` only via rule 1.

## 6. Acceptance oracle, specified

The slice's done command stands as written in #9509: list, then run
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue
--lib --features client,queue-broker
pmo_hyd_external_only_includes_uncertain_shared_actor`. The test is
specified here so the implementation issue can be scoped to it:

**Fixture.** One work item; one shared bot login; two sessions, A
(requesting) and B. Only session A has an operation/remote-ID map.
Events in the window:

| # | Event | Map evidence | Expected in `--external-only` for A |
| --- | --- | --- | --- |
| 1 | Comment created by A's operation | entry (A, remote ID) | **excluded** — rule 1 |
| 2 | Comment created by B's operation, same bot login | no entry | **included**, `uncertain` |
| 3 | Human comment with a body identical to event 1's | no entry | **included**, `external` — coincidence is not evidence |
| 4 | Edit to event 1's comment, no edit mapping | creation entry only | **included**, `uncertain` — edits don't inherit |
| 5 | Comment whose map entry is stale (remote ID re-keyed) | non-matching entry | **included**, `uncertain` |
| 6 | Comment by a distinct human actor | provenance names the human | **included**, `external` |

The assertion the test name carries: event 2 — same actor, no map —
is present in the output. Zero-test filter passes are not acceptance
(the list step in the done command is part of the oracle).

## 7. Compatibility and migration

- **Legacy activity is `uncertain`.** Events ingested before the map
  exists have no entries and classify `uncertain`; no backfill by
  actor, body, or timestamp is permitted. This is the slice's stated
  migration: "legacy uncertain; username never assigns session."
- No existing command spelling, flag, or outcome changes; the ratified
  `expedite=15` / `next-up=25` values are untouched and unrelated.
  Classification fields are additive annotations on existing result
  shapes.
- Any persisted-schema change for the map waits for the QTI crosswalk
  and the named current owner (#9241; slice compatibility gate) —
  this design fixes the entry's logical fields, not its storage.
- Rollback is disable-only: turning the classifier off returns
  unfiltered `changes` output; map entries and activity records are
  preserved. No history is deleted.

## 8. Gates and sequencing

Implementation of this design starts only when **all** of the
following hold (slice runtime gates, restated):

1. This record is merged.
2. REL-03 #9513 (operation identity/receipts — the map's Fleet-side
   keys), CLI-05 #9502 (session identity in lifecycle receipts), and
   HYD-06 #9508 (normalized records/revisions in `activity.rs`, the
   shared-file predecessor) are resolved.
3. A separately scoped implementation issue is filed against this
   record and the §6 oracle; #9509 itself authorizes no code.

Production adoption, any schema/migration step, and live acceptance
remain separate stages with their own evidence, per the slice body.
This design creates, changes, or retires **no schedule on any
surface**; `doc/fleet-schedule-registry.md` is intentionally
untouched.

## 9. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records only which part of this design speaks to each
group — it claims design coverage, not implementation.

| IDs | Where addressed |
| --- | --- |
| C-R118, C-R121, C-R122 | §1, §3, §5, §6 — the session-relative filter, ID-based attribution, uncertain-included rule |
| C-R119, C-R120 | §2.2–§2.3, §4 — event identity/revision separation the classifier consumes; cursor rules stay HYD-06's |
| C-R111, C-R112 | §5 — the activity/changes surface this classification annotates |
| C-R005, C-R009, C-R107, C-R108 | §5 — command/flag allocation consumed as-is (`--session-id`, `--since-start`, `--external-only`) |
| C-R150, C-R151 | §2.3, §7 — durable correlation metadata and its legacy treatment |
| H-R014, H-R016, H-R017, H-R024, H-R031 | §6, §8 — oracle and compatibility coverage carried into the implementation issue's acceptance |
| O-R022, O-R023, O-R028, O-R030, O-R049, O-R050 | §3, §4 — candidate/example occurrences behind the classifier rules and negative cases |
| P-R009, P-R012, P-R015, P-R016, P-R018, P-R027, P-R039, P-R050, P-R051, P-R065, P-R066 | §2, §3 — proposal occurrences behind the identity model and provenance consumption |

## 10. Non-goals

- No new queue, database, scheduler, service, resolver, executable,
  GitHub client, or general creation-outbox producer (slice boundary).
- No change to #6533 or its live acceptance; dispatch stays #9203.
- No actor-based heuristics, body-similarity matching, or timing
  correlation as attribution evidence — considered and rejected in
  §3, permanently for this classifier, not as a tuning question.
- No second classifier for AUTH-05: its cross-component acceptance
  (`pmo_hyd_external_only_includes_uncertain_shared_actor`) is owned
  here and referenced there, exactly as AUTH-05 states.
