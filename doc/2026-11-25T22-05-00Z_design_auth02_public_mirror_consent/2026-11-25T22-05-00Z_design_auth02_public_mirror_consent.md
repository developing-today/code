# AUTH-02 — Public mirror is explicit, allowlisted and operation-scoped

Issue: [#9499](https://github.com/developing-today/code/issues/9499)
(slice `AUTH-02` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the AUTH-02 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "public status category is allowlisted; private note
mirrors only with explicit consent, exact text/target and durable
operation; default private." It decides what may leave Fleet for
GitHub, what a consent record binds, how the target is named, and
what happens when the POST answer is lost, the bot identity is shared
or permission is revoked. It authorizes no code, schema, remote write
or deployment; those remain separately gated (§9).

Governing decisions consumed, not re-opened:

- **Component #9208 §3.1 and §4:** a private note "is never copied to
  issue/PR or Project without explicit, scoped, authorized mirror
  operation"; the authority table says "No automatic mirror; explicit
  note-mirror opt-in, exact text and target confirmation" and the
  public status reason is "Separate from note text".
- **Component #9208 §4, managed content honesty:** no documented
  compare-and-swap on comment updates; an initial POST whose answer
  is lost is uncertain and must be reconciled by marker before any
  retry. AUTH-03 (#9430) owns that reconciliation.
- **Component #9208 §7:** "Public mirror permission revoked — outbox
  terminal/auth failure; private note still retained, no repeated
  unauthorized retry." "Bot username matches another session —
  operation/session mapping required; otherwise attribution
  `uncertain`."
- **JRN-02** (`doc/2026-11-25T21-59-00Z_design_jrn02_private_note_status_reason/`):
  notes are always `private`; reads are scoped; the public status
  reason is the closed `StatusReasonCode`; a mirror without consent is
  refused with `note_mirror_consent_missing`.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  operation identity, the principal-scoped idempotency key and the
  stored receipt. REL-04 owns the durable intent; #9209 owns outbox
  transport.
- **DSP-08** (`doc/2026-11-25T21-52-00Z_design_dsp08_directives_owner_authority/`):
  grant is necessary, floors are not sufficient, and authority is
  checked again at every effect.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **Comment delivery exists.** `fleet gh sync-comment --repo OWNER/REPO
<issue|pr> <n> --body T | --body-file P` (`gh_sync.rs`, #10973)
  rewrites to `gh issue|pr comment`. It returns no structured comment
  id to its caller.
- **The author login is not stable.** `gh.rs` mints tokens from three
  fleet GitHub Apps (`Fleet`, `DevToday`, `Backup`) and a personal
  token, in rotation, with the matching installation first. The same
  Fleet write can appear under any of those logins.
- **Writes are not retried after they may have landed.** `gh.rs`
  `may_retry` (#8887): rate-limit and permission refusals may repeat;
  any other failure repeats only reads. A comment POST that times out
  is therefore not retried by the adapter.
- **Notes are private and local** (`turso_queue/notes.rs`); nothing on
  main copies a note to GitHub.
- `work/authority.rs` holds AUTH-04 (managed body section, default
  unsupported). No mirror policy exists.

## 1. Intent

Sometimes a worker or the owner wants a finding seen on the issue. That
must be a deliberate act about one exact text going to one exact place,
recorded so it can be proved or reconciled later, and nothing else
should ever leave Fleet by accident. Separately, a board or issue may
show a short status category, which is a fixed phrase and never a note.

## 2. Ownership

| Concern                                 | Owner                                    | This slice                    |
| --------------------------------------- | ---------------------------------------- | ----------------------------- |
| Mirror policy, consent record, refusals | `work/authority.rs`                      | **owns**                      |
| Public status category rendering        | `work/authority.rs` (`StatusReasonCode`) | **owns** the allowlist gate   |
| Note store and read scope               | JRN-02                                   | consumer                      |
| Operation identity and receipt          | REL-03                                   | consumer                      |
| Durable intent and outbox transport     | REL-04, #9209                            | consumer                      |
| Managed comment marker and reconcile    | AUTH-03                                  | hands off on uncertain        |
| Comment delivery                        | `fleet gh sync-comment` (`gh_sync.rs`)   | read-only consumer            |
| Attribution of the posted comment       | AUTH-05, HYD-07                          | records the mapping they need |

No new GitHub client, queue, service or outbox producer.

## 3. What may leave Fleet

Exactly two kinds of content, nothing else:

| Kind                   | Source                                 | Needs                                              |
| ---------------------- | -------------------------------------- | -------------------------------------------------- |
| Public status category | `StatusReasonCode` fixed rendering     | the repository's projection is switched on (§6)    |
| Mirrored note revision | one `task_notes` revision, exact bytes | a consent record (§4) and a durable operation (§5) |

Refused with `mirror_kind_not_allowed`, whatever the request says:
journal checkpoints and handoffs (JRN-03), history rows and
`previous_body`, legacy `park_reason` text, findings or decisions
stored as anything other than a note, hydration content, and any
free text a request supplies that is not a note revision. A checkpoint
stays private even when the owner asks; the owner may write a note
and mirror that.

Default is private. Nothing is mirrored because a note was written, a
work changed state, a lane finished, or a setting was turned on for
another kind.

## 4. The consent record

A mirror is allowed only for a `MirrorConsent` that binds all of:

| Field            | Meaning                                                                              |
| ---------------- | ------------------------------------------------------------------------------------ |
| `note_id`        | the note                                                                             |
| `note_revision`  | the revision (JRN-02 §3); an edit after consent voids it                             |
| `content_digest` | digest of the exact bytes that will be posted, marker excluded                       |
| `target`         | provider, owner, repo, number and kind (`issue` or `pull_request`), plus `placement` |
| `placement`      | `new_comment` or `managed_comment:<role>` (AUTH-03)                                  |
| `principal_ref`  | the authenticated principal that consented                                           |
| `confirmed_at`   | authority time                                                                       |
| `operation_id`   | the REL-03 operation the consent is spent on                                         |
| `policy_version` | the mirror policy version in force                                                   |

Rules:

1. **Who may consent:** the note's author, or the owner principal
   (DSP-08 §2). The principal must also pass the policy row in §7 for
   the target repository. A request field naming an actor never
   counts.
2. **Exact text.** The consent is shown the exact bytes and target
   before it is recorded. Any difference between the consented digest
   and the bytes the adapter would send refuses the send
   (`mirror_content_changed`); Fleet never trims, rewraps or
   summarizes after consent.
3. **Single use.** A consent is spent by one operation. Mirroring the
   same text again, or to a second target, needs a new consent.
4. **Expiry.** A consent not yet spent lapses after the policy's
   lifetime. This record proposes no default; until the owner sets
   one, a consent lapses at the end of the session that recorded it.
5. **Withdrawal.** The consenting principal may withdraw an unspent
   consent; a spent consent cannot be withdrawn, and removing the
   public comment is a separate GitHub action outside this slice.

## 5. Target and operation

**Target naming.** A target names owner, repo, number and kind. A bare
number, a URL that does not parse to one resource, or a kind that does
not match what the provider returns for that number is refused with
`mirror_target_ambiguous`. When a work links more than one GitHub
resource and the request names none, the mirror is refused the same
way; Fleet never picks one.

**Operation.** Each mirror is one REL-03 operation with a durable
REL-04 intent written before any remote call. The posted body carries
a hidden marker line naming the operation (`<!-- fleet:mirror
op=<operation_id> -->`), whose exact grammar AUTH-03 owns. The receipt
records the returned comment id and URL, the token source that made
the call (App slug or personal), and the operation id.

**Outcomes:**

| What happened                                | Outcome             | Next                                                    |
| -------------------------------------------- | ------------------- | ------------------------------------------------------- |
| 2xx with a comment id, read back matches     | `verified_complete` | consent spent                                           |
| 2xx with no id, or read back missing         | `uncertain`         | AUTH-03 reconcile by marker; no new POST                |
| timeout, reset or 5xx after the request left | `uncertain`         | AUTH-03 reconcile by marker; no new POST                |
| refused before sending (rate limit)          | `pending_sync`      | the adapter's existing retry; same operation id         |
| 401/403, or grant or role revoked            | `rejected`          | `authorization_revoked`; terminal, no retry, note stays |
| 404 or 410 on the target                     | `rejected`          | `mirror_target_gone`; terminal                          |
| 422 (body refused)                           | `rejected`          | terminal; consent stays unspent                         |

A lost answer is never treated as failure (that invites a duplicate)
or as success (that hides a missing comment). It stays `uncertain`
until AUTH-03's reconcile finds exactly one marked comment, finds
none after its visibility window, or finds several.

**Shared bot.** Because the posting login rotates across three Apps
and a personal token, a comment's author login proves nothing about
which session or operation wrote it. Attribution comes only from the
receipt's comment id mapped to the operation (HYD-07, AUTH-05). A
marker found in a comment whose id is not in a Fleet receipt is
treated as untrusted text, not as Fleet's.

## 6. Public status category

A repository's projection may show `StatusReasonCode` renderings only
when switched on for that repository by an owner policy change. The
projection carries the code's fixed wording and, for `parked_until`,
the time. It never carries a note, a caller string or legacy park
text. Switching it off stops new projections; it does not edit or
delete anything already shown.

## 7. Authorization

The DSP-08 policy table gains one proposed row over existing names:

| Verb               | Severity | PM role floor                                          | GitHub permission floor     | Confirmation            |
| ------------------ | -------- | ------------------------------------------------------ | --------------------------- | ----------------------- |
| `work.note.mirror` | high     | `owner` or `orchestrator`, or `agent` for its own note | `triage` on the target repo | the consent record (§4) |

A grant is still necessary (DSP-08 §3). Authority is checked at
consent, at intent write, and again immediately before the remote call
(DSP-08 §7); a change between them ends the operation with
`authorization_revoked` and no remote effect.

## 8. Metadata commands allocated here

The slice also carries the `metadata.*` command rows (merged from
CLI-07). They are field-authority writes under component §4, not
mirrors: title, summary, labels and fields are written from the
request's explicit values only, never from a note or checkpoint. A
priority write is refused here and routed to the priority owner
(#9199/#9200). `metadata.get` and `fleet-gh.metadata-query` are reads
and follow the source's own visibility.

## 9. Persisted fields, tests and gates

Persisted additions, all inside the operation record REL-03/REL-04
own: the `MirrorConsent` fields (§4), and on the receipt the comment
id, URL and token source. No new table and no column on `task_notes`.

Tests (first red is the slice's starting test):

| Test                                                                     | Proves                                                                  |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `pmo_auth_mirror_requires_scope_and_excludes_private_checkpoint` (first) | no consent, or a checkpoint source, refuses with no outbound effect     |
| `pmo_auth_mirror_missing_opt_in_refused`                                 | `note_mirror_consent_missing`; note unchanged                           |
| `pmo_auth_mirror_consent_binds_exact_bytes`                              | any byte difference refuses with `mirror_content_changed`               |
| `pmo_auth_mirror_note_edit_voids_consent`                                | a new revision after consent refuses                                    |
| `pmo_auth_mirror_consent_is_single_use`                                  | second send under the same consent refused                              |
| `pmo_auth_mirror_ambiguous_target_refused`                               | bare number, wrong kind, multi-link work with no target                 |
| `pmo_auth_mirror_lost_post_response_is_uncertain`                        | apply-then-timeout fake gives `uncertain`, hands to AUTH-03, no re-POST |
| `pmo_auth_mirror_shared_bot_login_is_not_attribution`                    | a marker in a comment with no receipt id is untrusted                   |
| `pmo_auth_mirror_revoked_permission_is_terminal`                         | revocation before the call: no remote effect, no retry, note retained   |
| `pmo_auth_status_category_renders_code_only`                             | random note text never appears in the status projection                 |
| `pmo_auth_status_category_off_by_default`                                | a repository with no switch shows nothing                               |
| `pmo_auth_metadata_write_never_reads_note`                               | metadata title/summary come only from explicit request values           |

Command: confirm listed with
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`,
then run
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_auth_mirror_requires_scope_and_excludes_private_checkpoint`.
A zero-test filter is not a pass.

Gates:

- **Hard blockers:** JRN-02 (record merged in #15283; also the
  shared-file predecessor on `authority.rs`), REL-03 (record on main)
  and CEN-04 (adapter harness).
- **Runtime gates:** `sync-comment` returning a structured comment id
  is a change to `gh_sync.rs` under its owner; until then every mirror
  answer is `uncertain` and goes to AUTH-03's reconcile.
- **Owner decisions this record does not make:** consent lifetime,
  which repositories switch on the status category, and whether
  `agent` may mirror its own notes (the §7 row is a proposal).
- **Stages each needing their own evidence and approval:** the policy
  row, adapter changes, any authenticated remote write, deployment.
- **Compatibility:** never adopt an existing comment as a mirror;
  additive only.
- **Rollback:** switch off mirrors and the status category. Spent
  consents, receipts and posted comments stay; nothing is deleted.

## 10. Requirement trace (design-level)

| ID                                                    | Outcome (crosswalk, abridged)                                        | Where  |
| ----------------------------------------------------- | -------------------------------------------------------------------- | ------ |
| C-R005, C-R107, H-R017, O-R023, P-R009, P-R002        | Cohesive Fleet CLI over existing tools; names reconciled             | §2, §8 |
| C-R108, P-R012                                        | Common flags, `--dry-run`, `--idempotency-key`, `--expected-version` | §4, §5 |
| C-R009, O-R021                                        | Coordinate work through progress and external reconciliation         | §3, §5 |
| C-R123, P-R017                                        | Notes and field authority explicit                                   | §3, §8 |
| C-R125                                                | No continuous title/body mirroring                                   | §3, §8 |
| C-R126, O-R031, H-R014                                | Private notes separate from public comments; selective mirror only   | §3, §4 |
| C-R127                                                | Explicit mirroring to an owned comment or section only               | §4, §5 |
| C-R132                                                | Native relationships projected, not replaced with prose              | §8     |
| C-R150–C-R153, O-R049, O-R050, O-R052, P-R035, P-R036 | Worker, resume, live comment and near-timeout walkthroughs           | §3, §5 |
| H-R016                                                | Crash after remote mutation, conflict, duplicate cases               | §5     |
| H-R024, H-R031                                        | Original commands, errors and failures stay in coverage              | §8, §9 |
| O-R022                                                | Adapter, journal and outbox responsibilities kept separate           | §2     |
| O-R026, P-R013                                        | Stable identity with qualified external links                        | §5     |
| O-R033                                                | Lifecycle distinct from ownership; provenance kept                   | §3     |
| P-R027                                                | Accept against authority and outbox scenarios                        | §9     |

## 11. Non-goals

- No managed comment update, marker grammar or duplicate cleanup
  (AUTH-03).
- No body-section mirror (AUTH-04 keeps it unsupported by default).
- No outbox transport or retry engine (#9209, REL-04).
- No new role or permission name; the §7 row uses existing ones.
- No consent lifetime default and no repository switched on.
