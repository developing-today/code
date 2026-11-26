# AUTH-03 — Managed comment conflict and creation recovery; no exactly-once claim

Issue: [#9430](https://github.com/developing-today/code/issues/9430)
(slice `AUTH-03` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the AUTH-03 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "update known comment ID, stable marker, serialize
Fleet-owned logical role, recover lost ack and verify/conflict", with
the negatives "zero marker eventual visibility, multiple markers,
human-authored text, edit/delete/minimize; never delete human
content" and the migration rule "no bulk comment adoption; persist ID
only after verified match". It decides the marker, the local role
record, the create, update and reconcile steps, and what Fleet does
when GitHub, a human or a second Fleet writer gets in the way. It
claims no exactly-once delivery. It authorizes no code, schema,
remote write or deployment (§8).

Governing decisions consumed, not re-opened:

- **Component #9208 §4, managed content honesty:** the comment update
  endpoint documents PATCH by id with a body and no conditional
  compare-and-swap, so "a read/compare/PATCH sequence cannot prevent
  a human edit in the gap". Prefer "one Fleet-managed known comment
  ID, serialize Fleet writers locally by `(work, repo, issue, role)`,
  store exact remote ID and operation markers, re-read after update,
  and surface divergence. Initial POST lost acknowledgment is
  uncertain: search marker and reconcile before retry; local unique
  managed-role row prevents Fleet/Fleet duplicate intent, not a
  remote exactly-once guarantee. … cleanup requires explicit
  Fleet-owned-resource verification and authorization, never delete
  human-authored content."
- **Component #9208 §7:** POST applied with acknowledgment lost is
  `unknown`; "find exact operation marker, adopt one; if none
  uncertain/recheck before POST; multiple -> conflict."
- **AUTH-04** (`work/authority.rs`, #9554): the same honesty applied to
  body sections. A pre-read check is never called optimistic locking;
  a timeout after PATCH is unknown and is reconciled by re-reading.
- **AUTH-02** (`doc/2026-11-25T22-05-00Z_design_auth02_public_mirror_consent/`):
  what may be posted and under which consent; every post is one REL-03
  operation with a marker; a lost answer is handed here; the posting
  login rotates across three fleet GitHub Apps and a personal token
  and is never attribution.
- **REL-03 / REL-04:** operation identity, the stored receipt and the
  durable intent written before any remote call.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- `work/authority.rs` holds AUTH-04 only: `SECTION_START`/`SECTION_END`
  markers, `ConditionalWriteCapability` (default unsupported),
  `Refusal`, `PlannedUpdate`, `reconcile_after_patch_timeout`. No
  managed comment model exists.
- `fleet gh sync-comment` (`gh_sync.rs`) posts a comment and returns
  no structured comment id. There is no comment edit or list verb in
  the `sync-*` set; `fleet gh` passes other `gh` calls through its
  guard.
- `gh.rs` `may_retry` (#8887) repeats a write only after a refusal that
  proves it did not run (rate limit, permission).
- `gh.rs` rotates three App installations (`Fleet`, `DevToday`,
  `Backup`) and a personal token; the comment author login varies with
  the token used.

## 1. Intent

When Fleet keeps one progress comment on an issue, it must keep
exactly that comment up to date, never post a second copy because an
answer got lost, never overwrite what a person wrote, and say plainly
when it cannot tell what happened. GitHub gives no conditional write,
so the design is honest about the one race it cannot close and makes
every other case detectable.

## 2. Ownership

| Concern                                       | Owner                              | This slice                        |
| --------------------------------------------- | ---------------------------------- | --------------------------------- |
| Marker grammar, role record, reconcile policy | `work/authority.rs`                | **owns**                          |
| What may be posted, consent                   | AUTH-02                            | consumer                          |
| Operation identity, durable intent, receipts  | REL-03, REL-04                     | consumer                          |
| Outbox transport                              | #9209                              | consumer                          |
| Comment create, read, list, edit calls        | `fleet gh` (`gh.rs`, `gh_sync.rs`) | read-only consumer; §7 names gaps |
| Attribution of comments in activity           | HYD-07, AUTH-05                    | supplies the id-to-operation map  |

The policy is pure, like AUTH-04: it parses markers, compares
snapshots and returns a plan or a refusal. No I/O in `authority.rs`.

## 3. The marker

A Fleet-managed comment's first line is exactly one marker:

```
<!-- fleet:managed-comment v=1 role=<role> work=<authority_id>/<work_id> op=<operation_id> -->
```

- `role` is from a closed, versioned set. This record proposes one:
  `progress`. AUTH-02's one-off mirrors use their own
  `<!-- fleet:mirror v=1 op=<operation_id> -->` marker and no role.
- `op` is the operation that last wrote the comment, so each write is
  traceable to one receipt.
- Parsing is exact: one marker, on the first line, every field
  present, no extra fields. A comment with zero markers is not
  managed. A comment with two or more markers, or a malformed one, is
  refused with `managed_marker_malformed` and never written.
- **A marker is a claim, not proof.** Anyone can type it. A comment is
  treated as Fleet's only when its id is in a Fleet receipt, or, during
  reconcile (§5), when its author is one of the fleet's own posting
  identities and its body digest equals the intended body. A marker
  on any other comment is human text: it is reported
  (`managed_marker_untrusted`) and never edited or deleted.

## 4. The local role record

One record per logical role, unique on
`(authority_id, work_id, provider, owner/repo, number, role)`, held
in the existing broker store under REL-04's operation tables:

| State          | Meaning                                             | Who may move it                    |
| -------------- | --------------------------------------------------- | ---------------------------------- |
| `absent`       | no comment for this role                            | a create operation                 |
| `creating(op)` | intent written, POST may or may not have landed     | that operation's reconcile         |
| `bound(id)`    | comment id verified as Fleet's                      | update, reconcile, explicit unbind |
| `conflict`     | several candidates, human edit, or untrusted marker | owner-confirmed resolution only    |
| `gone(id)`     | the bound comment was deleted or hidden on GitHub   | an explicit recreate operation     |

Writers take the record in one `BEGIN IMMEDIATE` transaction. A second
Fleet writer that finds `creating(op)` for another live operation does
not POST; it returns `pending_sync` naming that operation. This stops
Fleet racing itself. It does not stop GitHub from holding two
comments, which §5 detects.

The comment id is persisted only after a verified match (§5). There is
no bulk adoption: an existing comment, even with a valid-looking
marker, is never bound unless a Fleet receipt or a reconcile for this
record's own operation names it.

## 5. Create, update and reconcile

**Create** (`absent` → `creating(op)` → `bound(id)`):

1. Write the REL-04 intent and set `creating(op)`.
2. POST the body with the marker.
3. On 2xx with an id: GET that id, check the marker and the body
   digest, then set `bound(id)` and record the receipt. Outcome
   `verified_complete`.
4. On any answer that does not prove the POST did not run (timeout,
   reset, 5xx, 2xx with no id): outcome `uncertain`; go to reconcile.
   No second POST.

**Reconcile after a lost create answer.** List the target's comments
created at or after the intent time (paginated, all pages), and keep
those whose first line is this operation's marker, whose author is a
fleet posting identity, and whose body digest equals the intended
body:

| Candidates found                        | Result                                                                   |
| --------------------------------------- | ------------------------------------------------------------------------ |
| exactly one                             | adopt: `bound(id)`, `verified_complete`                                  |
| none, inside the visibility window      | stay `uncertain`; read again                                             |
| none, after the visibility window       | stay `uncertain`, next action "confirm re-post under the same operation" |
| two or more                             | `conflict`; list every id; bind none                                     |
| marker present, author or digest differ | ignore that comment as untrusted; report `managed_marker_untrusted`      |

GitHub's list can lag a successful POST, so zero results right after a
timeout prove nothing. The visibility window is a policy value; this
record proposes three reads over two minutes as a default for owner
review. After it, Fleet still does not re-post by itself: a re-post
needs an explicit confirmation, reuses the same operation id and
marker, and so leaves any late duplicate detectable. This is why the
slice claims no exactly-once delivery.

**Update** (`bound(id)`):

1. GET the comment. If it is gone or hidden, set `gone(id)` and stop
   (`managed_comment_gone`). Fleet never recreates it on its own.
2. Check the marker and that the body digest equals the digest Fleet
   last wrote. If it differs, a person edited it: set `conflict`,
   keep both snapshots, stop (`managed_comment_edited_externally`).
3. PATCH by id with the new body (marker `op` updated).
4. GET again. Digest equals the new body: `verified_complete`. Equals
   the old body: the PATCH did not apply; the same operation may try
   again. Anything else: someone wrote in the gap; set `conflict`,
   keep the snapshots.

Steps 2 to 4 narrow the human-edit race to the moment between the GET
and the PATCH. They do not close it, and Fleet never calls this
optimistic locking. A human edit landing in that moment is overwritten
and then reported, with the human's text kept in the snapshot.

**PATCH timeout:** `uncertain`; GET and compare as in step 4.

## 6. Conflicts, duplicates and human content

- **Duplicates.** When reconcile finds two or more Fleet candidates,
  Fleet binds none and reports all ids. Removing the extras needs an
  owner-confirmed operation that names each id, and each must be
  proven Fleet's (receipt or reconcile match, fleet posting identity,
  intended digest). The default is to leave them.
- **Human content is never deleted or edited.** A comment that is not
  proven Fleet's is read only, whatever its marker says.
- **Edited, deleted, hidden.** Edited → `conflict`; deleted →
  `gone(id)`; hidden or minimized → `gone(id)` with reason `hidden`.
  Fleet never un-hides, restores or reposts without a new operation.
- **Resolution.** Leaving `conflict` or `gone` is an explicit
  owner-confirmed operation: rebind to a named proven id, unbind and
  stop, or create anew under a new operation.

## 7. Adapter gaps, tests and gates

The policy needs three adapter capabilities `fleet gh` does not offer
as typed verbs today: a comment create that returns the id, a comment
GET and list with author and created time, and a comment PATCH by id.
Adding them is a change to `gh_sync.rs` under its owner, gated in §8.
Until then the model runs against the CEN-04 adapter harness only.

Tests (first red is the slice's starting test):

| Test                                                                       | Proves                                                             |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `pmo_auth_managed_comment_timeout_reconciles_marker_before_repost` (first) | apply-then-timeout fake: one comment, adopted, no second POST      |
| `pmo_auth_managed_comment_zero_marker_waits_out_visibility`                | lagging list: stays `uncertain`, no re-post inside the window      |
| `pmo_auth_managed_comment_repost_needs_confirmation_same_op`               | after the window, re-post only on confirmation, same op and marker |
| `pmo_auth_managed_comment_multiple_markers_conflict`                       | two candidates: `conflict`, both ids listed, none bound            |
| `pmo_auth_managed_comment_human_marker_untrusted`                          | a marker typed by a non-fleet author is never adopted or edited    |
| `pmo_auth_managed_comment_human_edit_stops_update`                         | changed digest before PATCH: `conflict`, snapshots kept, no PATCH  |
| `pmo_auth_managed_comment_gap_edit_detected_after_patch`                   | edit between GET and PATCH: reported, human text in the snapshot   |
| `pmo_auth_managed_comment_deleted_or_hidden_is_gone`                       | 404 or hidden: `gone`, no recreate                                 |
| `pmo_auth_managed_comment_second_fleet_writer_does_not_post`               | `creating(op)` held: second writer gets `pending_sync`             |
| `pmo_auth_managed_comment_no_bulk_adoption`                                | pre-existing marked comments are never bound                       |
| `pmo_auth_managed_comment_never_deletes_human_content`                     | property: no plan deletes or edits a comment not proven Fleet's    |
| `pmo_auth_managed_comment_malformed_marker_refused`                        | zero, two, or malformed markers refuse with no write               |

Command: list, then run
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_auth_managed_comment_timeout_reconciles_marker_before_repost`.
A zero-test filter is not a pass.

## 8. Persisted fields and gates

Persisted: the role record (§4) and, on each receipt, the comment id,
the body digest written and the posting identity. All live in the
REL-03/REL-04 operation store; no column on a task or note row.

- **Hard blockers:** AUTH-02 (#9499, also the shared-file predecessor
  on `authority.rs`), REL-03, REL-04, CEN-04. REL-05 relay ordering is
  a recommended integration edge, not a blocker.
- **Owner decisions this record does not make:** the visibility window
  default, the role set beyond `progress`, and when duplicates may be
  cleaned up.
- **Stages each needing their own evidence and approval:** the three
  adapter verbs, any authenticated remote write, deployment.
- **Compatibility:** no bulk adoption; persist an id only after a
  verified match.
- **Rollback:** stop managed comment writes. Role records, receipts
  and comments on GitHub stay; nothing is deleted.

## 9. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                            | Where  |
| -------------------------------------- | ------------------------------------------------------------------------ | ------ |
| C-R005, C-R107, H-R017, O-R023, P-R009 | Cohesive Fleet CLI; names reconciled                                     | §2, §7 |
| C-R108                                 | Common flags (`--idempotency-key`, `--expected-version`, `--verify`)     | §5     |
| C-R009                                 | Report notes and comments, handle new activity, verify directly          | §5     |
| C-R123, P-R017                         | Field authority explicit; edit only managed content                      | §3, §4 |
| C-R125                                 | No continuous body mirroring                                             | §2     |
| C-R126, O-R031, H-R014                 | Private journal vs public comments; managed comments only, no duplicates | §3, §6 |
| C-R127                                 | Explicit mirroring to an owned managed comment; honest concurrency       | §5     |
| C-R132                                 | Native GitHub state projected, not replaced                              | §6     |
| H-R024, H-R031                         | Original commands, errors and failures stay in coverage                  | §7     |
| O-R033                                 | Provenance kept; nothing erased on resolution                            | §6     |
| P-R013                                 | Identity, links and metadata version represented                         | §4     |
| P-R027, P-R052, P-R067                 | Acceptance for safe managed content and recovery scenarios               | §7     |

## 10. Non-goals

- No exactly-once claim and no optimistic-locking claim.
- No body-section editing (AUTH-04) and no consent rules (AUTH-02).
- No automatic duplicate cleanup and no automatic recreate.
- No new GitHub client; adapter verbs are named as gaps only.
