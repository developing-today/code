# JRN-02 — Private note read/search scope and status-reason separation

Issue: [#9464](https://github.com/developing-today/code/issues/9464)
(slice `JRN-02` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the JRN-02 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row asks for a "private note entity/current
projection; correction appends/supersedes; public status reason is an
allowlisted code, never an implicit copy of private body". Main has
since gained a real notes store, so this record reconciles the row
with that store instead of designing a second one, then decides who
may read and search a note, how a correction stays append-only, and
what a public status reason may carry. It authorizes no code, schema,
deployment or remote write; those remain separately gated (§9).

Governing decisions consumed, not re-opened:

- **Component #9208 §3.1:** three separate concepts. A private work
  note is "private by default. Search/list/read only under
  actor/task access control; it is never copied to issue/PR or
  Project without explicit, scoped, authorized mirror operation." A
  status reason is "structured, allowlisted … It is not permission to
  publish private note content." A GitHub comment is GitHub-owned.
- **Component #9208 §3.2 and §3.3.1 (JRN-01, merged in #12398):** the
  journal is "one event history on the canonical existing task
  identity/storage authority, not a second event-sourcing database";
  a note `edit` "creates a new event with correction/supersedes ref
  and updates a current-note projection … it does not rewrite/delete
  the original"; deletion is a tombstone; retrying the same operation
  id allocates no second correction.
- **Component #9208 §5:** the public status reason is "a fixed
  allowlist (`waiting_for_review`, `external_dependency`,
  `parked_until`, etc.) but not arbitrary private note fields. Exact
  code set and public wording require owner review."
- **CLI-02** (`doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`):
  the shared envelope and strict parser. Command registration stays
  with the single CLI-13 slice; this record names verbs, it does not
  register them.
- **JRN-03** (`work/journal.rs`, #14346): `JournalVisibility` and the
  rule that private checkpoints never project anywhere else.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **The notes store exists.** `turso_queue/notes.rs` (#14170 Q4)
  keeps an append-only `task_notes` table: `id, task_id, author,
author_kind, session_id, model_provider, model_id, model_variant,
body, created_at, edited_at, deleted_at`, indexed on
  `(task_id, id)`. The author is the authenticated worker from the
  broker's `X-Agent-Token` channel, never a request field. Bodies are
  limited to 4096 characters.
- **Verbs:** `/note-add`, `/note-list`, `/note-edit` (author-only),
  `/note-remove` (author-only soft delete). Each write commits the
  note change, a `note_*` event and a state-neutral `task_history`
  row in one `BEGIN IMMEDIATE` batch. `history_transition` does not
  accept note kinds, so a generic `/events-append` cannot fabricate
  a note history row.
- **Edit overwrites the row.** `/note-edit` replaces `task_notes.body`
  and puts the prior body in the `note_edited` history row's
  `previous_body`. The immutable record of the old text is therefore
  the history row, and the `task_notes` row already behaves as the
  current-note projection.
- **No reader scope.** `/note-list` takes only `task_id`; any caller
  that reaches the broker can read any task's notes. There is no
  note search verb and no visibility column.
- **Status reason is free text.** `/park` merges
  `{"park_reason": reason}` into `task.result`. No status-reason code
  type exists. `contracts.rs` `ReasonCode` is the priority-decision
  registry (`stale_snapshot`, `unauthorized_input`, …), not a status
  reason set.
- **The client** reads `/note-list` in `client/commands_query.rs` and
  tolerates a broker that does not serve it.
- `work/authority.rs` holds AUTH-04 (managed body section policy);
  `work/journal.rs` holds JRN-03 (checkpoint and handoff).

## 1. Intent

Agents and humans write private findings while they work. Those
notes must be readable by the people and agents doing the work and by
nobody else, searchable without leaking across works or authorities,
correctable without losing what was first written, and never turned
into a public comment or a public status line by accident. A public
status reason is a separate, closed vocabulary that a board or issue
may show; it never carries note text.

## 2. Ownership

| Concern                                  | Owner                                | This slice                                     |
| ---------------------------------------- | ------------------------------------ | ---------------------------------------------- |
| Note storage, write verbs, history rows  | `turso_queue/notes.rs` (#14170 Q4)   | read-only consumer; §8 names the wiring change |
| Note read/search scope, projection model | `work/journal.rs`                    | **owns**                                       |
| Status-reason code set and public render | `work/authority.rs`                  | **owns**                                       |
| Mirror of a note to GitHub               | AUTH-02 (#9499)                      | refuses without its consent record (§6)        |
| Grants and principals                    | DSP-01 `work/operations.rs`, DSP-08  | consumer                                       |
| Command names and registration           | CLI-02 contract, CLI-13 registration | names verbs only                               |
| Park, resume, transition semantics       | #9179, #9206 lifecycle               | consumer                                       |

No new table, queue, service or event store. The existing `task_notes`
table, `events` and `task_history` stay the store.

## 3. Note model and the current projection

A note is the `task_notes` row plus its history. The model in
`journal.rs` names three views over the same rows:

- **Note** (`NoteRef`): `note_id`, `work: WorkRef`, `author`,
  `author_kind`, `session_id`, model marks, `created_at`, and
  `visibility`. For JRN-02 the only value a note may hold is
  `JournalVisibility::Private`. A payload naming any other value is
  rejected (`note_visibility_not_private`), the same rule JRN-03 uses
  for checkpoints.
- **Current note** (`CurrentNote`): the `task_notes` row as it stands,
  with `revision` = 1 + the number of `note_edited` rows for that
  note. A removed note's current view is a tombstone that keeps
  `note_id`, author and `deleted_at` and shows no body.
- **Audit view** (`NoteHistory`): the creation event and every
  `note_edited` / `note_removed` row in order, each with its
  `previous_body`, actor and time.

Correction rules:

1. An edit appends one `note_edited` row whose `previous_body` is the
   exact prior bytes and whose `supersedes_revision` names the
   revision it replaced. The prior row is never updated or deleted.
2. The edit and its history row commit in one batch, as today. If the
   batch fails, neither the row nor the revision advances.
3. A retried edit under the same `operation_id` returns the first
   result and appends nothing. Today `/note-edit` has no operation id;
   §8 adds one as an optional request field.
4. Remove is a tombstone event; it never deletes the row or its
   history. Redaction for a legal or policy reason is a separate,
   owner-authorized event that replaces stored bytes with a redaction
   marker while keeping event identity. It is out of scope here.
5. Edit and remove stay author-only. No role, owner included, may
   edit someone else's note; an owner may add a correcting note that
   references it.

## 4. Who may read a note

Today any broker caller can list any task's notes. JRN-02 closes that.
A read is allowed when the authenticated principal meets **one** of:

| Basis         | Rule                                                                                 |
| ------------- | ------------------------------------------------------------------------------------ |
| Author        | the principal wrote the note                                                         |
| Active holder | the principal holds the work's current claim or bound session                        |
| Work grant    | a DSP-01 `Grant` gives the principal a read verb (`status`) on the work's repository |
| Owner         | the configured owner principal (DSP-08 §2), for any work in its authority            |

Everything else is denied with `note_read_unauthorized`, and the
response says nothing about whether notes exist. The decision is made
by the broker from the authenticated principal; a `worker`, `author`
or `actor` field in the request never widens it. Each denied read is
journaled as a policy decision (DSP-08 §9 receipt shape) without the
note body.

Removed notes appear only with `include_removed` and only to the
author or owner; others see the tombstone count, not the tombstones.

## 5. Search

`/note-search` is a new broker verb on the existing broker. Rules:

- **Scope is required.** A request names an authority and either one
  work or a bounded set of works (at most 200). There is no
  authority-wide unscoped search.
- **Filter before match.** The broker first reduces the requested
  works to those §4 lets the caller read, then matches. Results,
  counts and "more results" flags are computed only over readable
  works, so the response cannot reveal a match in a work the caller
  may not read.
- **Authority isolation.** A work id is always resolved inside the
  named authority. The same number in another authority is a
  different work and never matches.
- **Match:** case-insensitive substring over the current body, with an
  optional author and time range. Bounded: at most 100 results per
  page, a cursor for the next page, and a query of at most 256
  characters. Full-text indexing is a later change behind its own
  evidence; the contract does not depend on it.
- **History is not searched** unless the caller asks for
  `include_history` and is the author or owner, because a prior body
  may hold text its author chose to correct.

## 6. Status reason is a separate field

A status reason is a typed code, never text copied from a note.

`StatusReasonCode` in `authority.rs` is a closed enum, separate from
the priority `ReasonCode`. Proposed initial set, from component §5
plus the park conditions already on main:

| Code                  | Public default rendering          |
| --------------------- | --------------------------------- |
| `waiting_for_review`  | Waiting for review                |
| `external_dependency` | Waiting on an external dependency |
| `parked_until`        | Parked until a set time           |
| `capacity_wait`       | Waiting for capacity              |
| `blocked_by_work`     | Blocked by other work             |
| `needs_owner`         | Needs an owner decision           |
| `handed_off`          | Handed off                        |
| `unclassified`        | (not rendered publicly)           |

The set and its wording are a proposal; the owner may change them by
a versioned policy change (component §5 asks for owner review).

Rules:

1. A transition carries `status_reason_code` as a typed field, plus an
   optional `status_reason_note_id` that points at a private note for
   the longer explanation. The note stays private.
2. The public projection renders only the code's fixed wording and,
   for `parked_until`, the time. It never renders a note body, a
   caller-supplied string or the legacy `park_reason` text.
3. **No spoofing.** Free text that happens to equal a code
   (`"waiting_for_review"`) is text, not a code. Only the typed field
   sets a code, and an unknown code is rejected
   (`status_reason_unknown`), not mapped.
4. **Legacy mapping.** An existing `result.park_reason` string is
   read as structured status evidence with code `unclassified` and
   its bytes kept, unchanged, in the private projection. Nothing
   reclassifies it by matching words. A new park written after
   adoption carries a code; the free-text field stays accepted for
   compatibility and is treated as a private note body.
5. **No mirror without consent.** Nothing in this slice writes to
   GitHub or the Project. A request to show a note publicly is
   refused with `note_mirror_consent_missing` unless AUTH-02's
   consent record exists for that note revision and target; AUTH-02
   then owns the operation.

## 7. Serialization boundaries

Every outward serializer (hydration bundle, Project projection,
public status view, `fleet.result/v1` effects, board API) goes through
one allowlist: it may carry note ids, counts and status-reason codes,
never a note body or `previous_body`. The test in §8 checks this with
random strings planted in note bodies.

Inside the broker, `/note-list` and `/note-search` return bodies only
to callers §4 allows.

## 8. Persisted fields, tests and gates

Persisted additions (additive, nullable, no backfill):

- `task_notes.visibility TEXT NOT NULL DEFAULT 'private'`, with a
  check that it is `private`.
- `note_edited` history detail gains `supersedes_revision` and an
  optional `operation_id`.
- transition detail gains `status_reason_code` and optional
  `status_reason_note_id`; `result.park_reason` stays as is.

The wiring in `turso_queue/notes.rs` and the park path is a change to
files this slice does not own; it lands as a follow-on under the
#14170 notes owner once this model is merged.

Tests (first red is the slice's starting test):

| Test                                                         | Proves                                                                |
| ------------------------------------------------------------ | --------------------------------------------------------------------- |
| `pmo_journal_private_note_never_serializes_publicly` (first) | property: random note bodies are absent from every outward serializer |
| `pmo_journal_correction_preserves_original_event`            | edit appends; the prior history row and its bytes are unchanged       |
| `pmo_journal_correction_retry_is_idempotent`                 | same operation id twice gives one revision                            |
| `pmo_journal_remove_is_tombstone`                            | removed note keeps id, author and history; body hidden                |
| `pmo_journal_note_read_requires_scope`                       | each §4 basis allows; a stranger is denied with no existence signal   |
| `pmo_journal_request_actor_field_never_widens_read`          | a forged `author`/`worker` field changes nothing                      |
| `pmo_journal_search_filters_before_match`                    | matches in unreadable works do not affect results, counts or paging   |
| `pmo_journal_search_isolates_authorities`                    | the same work number in two authorities never cross-matches           |
| `pmo_journal_status_reason_text_cannot_spoof_code`           | free text equal to a code stays text; unknown code rejected           |
| `pmo_journal_legacy_park_reason_maps_unclassified`           | legacy bytes kept privately; public view shows no text                |
| `pmo_journal_mirror_refused_without_consent`                 | `note_mirror_consent_missing`; no outbound effect                     |
| `pmo_journal_note_visibility_is_private_only`                | any other visibility rejected                                         |

Command: confirm both `pmo_journal_private_note_never_serializes_publicly`
and `pmo_journal_correction_preserves_original_event` appear in
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`,
then run the package filter `pmo_journal_`. A zero-test filter is not
a pass.

Gates:

- **Hard blockers:** JRN-01 (closed; record merged in #12398) and
  CLI-02 (closed; record on main). Shared-file predecessors: JRN-01 on
  `journal.rs` (done) and AUTH-05 (#9432) on `authority.rs`.
- **Runtime gates:** #3424 reason reconciliation before the code set
  is adopted, and #9178 ownership semantics before the "active
  holder" read basis is wired.
- **Owner decisions this record does not make:** the status-reason
  code set and its public wording (§6).
- **Stages each needing their own evidence and approval:** the
  `task_notes` column, broker wiring, deployment.
- **Rollback:** disable the new read check and search verb; notes,
  history and codes already written stay. No row or history is
  deleted.

## 9. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                                  | Where      |
| -------------------------------------- | ------------------------------------------------------------------------------ | ---------- |
| C-R005, C-R107, P-R009, H-R017, O-R023 | Cohesive Fleet CLI; names reconciled; no parallel mechanism                    | §2, §5     |
| C-R108                                 | Common flags (`--work`, `--json`, `--dry-run`, `--idempotency-key`)            | §3, §5     |
| C-R009, C-R150, H-R001                 | Agents report notes and resume work in the coordinated workflow                | §3, §4     |
| C-R126, O-R031                         | Private notes and journal separate from public comments; selective mirror only | §3, §6, §7 |
| H-R014                                 | Private notes vs public comments preserved                                     | §4, §7     |
| H-R016                                 | Blocked or parked resume, conflict cases                                       | §6         |
| H-R024, H-R031                         | Original commands, errors and failures stay in coverage                        | §2, §8     |
| O-R022                                 | Journal responsibility kept separate                                           | §2         |
| O-R033                                 | Lifecycle distinct from ownership; parked state keeps provenance               | §6         |
| P-R013                                 | Work identity, state/reason and metadata version represented                   | §3, §6     |
| P-R017                                 | Field authority explicit; synchronize selectively                              | §6, §7     |

## 10. Non-goals

- No second notes store, event store or search service.
- No mirror operation (AUTH-02) and no managed comment (AUTH-03).
- No change to who may write or edit a note (author-only stays).
- No reclassification of legacy park text and no ratified code set.
- No command registration (CLI-13).
