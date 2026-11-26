# AUTH-01 — Field ownership and scoped authorization contract

Issue: [#9429](https://github.com/developing-today/code/issues/9429)
(slice `AUTH-01` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record turns the field authority table of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
component `9208-…` §4) into one executable decision: for any write to
any field Fleet knows about, who owns the field, whether this caller
may write it here, against which version, and what the caller is told
when the answer is no. It also gives HYD-04 the read decision it is
waiting on. It authorizes no code, schema, remote write or deployment;
those stay separately gated (§11).

Governing decisions consumed, not re-opened:

- **Component #9208 §4:** "Field mutation carries authenticated-principal
  capability, scope (work/ref/field), expected version or current
  baseline, reason code, operation/idempotency ID, visibility intent.
  Reject unauthorized or stale writes before local transaction." "No new
  auth system." "Never silently choose last-writer-wins across
  Fleet/GitHub/Project." `dry_run` "must create no task/edge/note/event/
  outbox writes or remote effects."
- **Component #9208 §4, priority:** AUTH-01 "must reject every
  routine/automatic protected-set score, peer-rank, priority-label or
  membership edit, including a forged owner actor field and a manual
  hysteresis bypass." PRI-01 (`work/priority.rs`) already enforces this;
  AUTH-01 routes to it and never decides priority itself.
- **DST-01** (`work/contracts.rs`): authorization reads the
  `AuthenticatedPrincipal` only, never `actor_ref`; reason codes explain
  an operation and none grants authority.
- **DSP-01 / DSP-08** (`work/operations.rs`,
  `doc/2026-11-25T21-52-00Z_design_dsp08_directives_owner_authority/`):
  the Fleet grant is necessary for every verb from every source; a
  source floor is an extra condition, never a grant; authority is
  checked again at every effect.
- **AUTH-02, AUTH-03, AUTH-04:** note mirror consent, managed comment
  recovery and managed body section are separate operations with their
  own policy in `work/authority.rs`. AUTH-01 classifies those fields and
  hands off; it does not restate their rules.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- `work/authority.rs` holds only AUTH-04 (managed body section, default
  unsupported unless a verified conditional-write capability or a
  best-effort opt-in exists). There is no field matrix and no read
  policy.
- `work/contracts.rs` has `WorkRef`, `AuthenticatedPrincipal`,
  `evaluate_authorization` (capability check, `actor_ref` ignored),
  `ReasonCode` with `check_field_effect` (base / derived / hint), and
  `ContractError` with `ClientSuppliedAuthorization`,
  `UnauthorizedInput`, `OwnerPolicyProtected` and
  `OwnerPolicyVersionConflict`.
- `work/priority.rs` (PRI-01) has `PriorityWrite`, `WriteSource` and
  `check_priority_write`, which refuses every non-owner write to the
  protected set and forged owner sources.
- `work/operations.rs` (DSP-01) has `Grant` (principal × verbs × repos)
  and `AuthorizationPolicy::authorize`. Its `Verb` set has no metadata or
  link verb.
- `work/journal.rs` has `JournalVisibility` {`Private`, `Internal`,
  `PublicProjection`}.
- **Project fields:** the only Project field mutation in Fleet code is
  `updateProjectV2ItemFieldValue` in `AppRotationGitHubWriter`
  (`hosted-worker/src/github.ts`). The local `fleet gh` adapter
  (`crates/fleet/src/gh.rs`) classifies `project item-list` and
  `project field-list` as reads. Which process is the sole Project writer
  is fixed by the #9198 census; this record encodes that answer as data
  and does not re-decide it.
- **Titles:** `fleet gh` metadata create (`gh_metadata.rs`, #9073) stamps
  provenance on issue and PR creation. Nothing on main copies a queue
  title onto a PR title after creation, and nothing forbids it either.

## 1. Intent

Fleet, GitHub and the Project each own some fields about the same piece
of work. Today the boundary lives in prose. A worker that changes a
queue title could, through a helpful adapter, rename the PR; a Project
column could gain a second writer; a status reason could carry a line
of a private note onto a public board. AUTH-01 makes the boundary one
function that every writer calls before its local transaction, and
makes the refusal specific enough that the caller can fix the request
rather than retry it.

## 2. Ownership

| Concern                                  | Owner                         | This slice                   |
| ---------------------------------------- | ----------------------------- | ---------------------------- |
| Field matrix, write and read decision    | `work/authority.rs`           | **owns**                     |
| Typed refusals for field writes          | `work/authority.rs`           | **owns**                     |
| Fleet grants and verb floors             | `work/operations.rs` (DSP-01) | consumer; proposes one verb  |
| Priority and protected set               | `work/priority.rs` (PRI-01)   | routes to it                 |
| Lifecycle transitions and leases         | #9178 / #9179 / REL-02        | routes to it                 |
| Dependency graph mutation                | DEP-03                        | routes to it                 |
| Note mirror, managed comment and section | AUTH-02 / AUTH-03 / AUTH-04   | classifies and hands off     |
| Operation identity and receipts          | REL-03                        | consumer (`operation_id`)    |
| Resource identity and resolution         | #9206, HYD-02                 | consumer (resolved ref only) |
| GitHub delivery                          | `fleet gh` (`gh.rs`)          | read-only consumer           |
| Project field delivery                   | the #9198 sole writer         | read-only consumer           |

Pure policy and tests only. No new client, store, service, token or
role. Shared CLI files are excluded (CLI-12 and AUTH-02 §8 own the
command rows that call this).

## 3. Field classes

Every writable field belongs to exactly one class. The class set is
closed: a field name that maps to no class is refused with
`FIELD_UNCLASSIFIED`, never treated as a free-form Fleet field.

| Class                  | Fields (examples)                                         | Authority                 | Write rule here                                                     | Read rule (§7)                   |
| ---------------------- | --------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------- | -------------------------------- |
| `fleet_descriptive`    | queue title, summary                                      | Fleet                     | allowed with grant and expected version; never propagates elsewhere | internal                         |
| `fleet_lifecycle`      | state, status code, assignment, lease, session binding    | lifecycle owner           | **routed**; AUTH-01 never writes it                                 | internal                         |
| `fleet_priority`       | urgency, importance, protected membership, peer rank      | PRI-01                    | **routed** to `check_priority_write`                                | internal                         |
| `fleet_relation`       | hard and advisory edges                                   | DEP-03                    | **routed**                                                          | internal                         |
| `fleet_note`           | private notes, checkpoints, findings                      | Fleet journal             | allowed with grant; visibility `Private` only                       | private: authorized readers only |
| `public_status_reason` | the rendered status category                              | Fleet status code         | registry code only; free text refused                               | public projection                |
| `creation_provenance`  | origin, verified external actor, provenance stamp         | Fleet                     | write once at creation; later writes refused                        | internal                         |
| `github_resource`      | issue or PR title, body, state, labels, comments, reviews | GitHub                    | only through `fleet gh` with resolved ref and external revision     | follows the source               |
| `github_managed`       | managed body section, managed comment                     | GitHub, Fleet-marked part | **handed off** to AUTH-04 / AUTH-03                                 | follows the source               |
| `project_field`        | any GitHub Project field                                  | the #9198 sole writer     | refused for every other writer                                      | follows the source               |

Three consequences are stated so that no implementation drifts:

1. **Classes do not propagate.** A write to `fleet_descriptive` changes
   the queue row and nothing else. Seeding a queue title from an issue
   title happens once, at creation, as a `creation_provenance` act; after
   that the two are independent (`pmo_auth_queue_title_does_not_mutate_pr_title`).
   An adapter that wants to rename the PR makes a separate
   `github_resource` write, with its own decision.
2. **Routed is not allowed.** For `fleet_lifecycle`, `fleet_priority`
   and `fleet_relation` the decision is `Routed { owner }`. A caller that
   applies a routed write itself, without the owner's own check, is a
   bug the tests look for.
3. **The status reason is not a note.** `public_status_reason` takes a
   value from the closed status registry. It has no free-text field, so
   there is nothing a note could leak through.

## 4. The write request

```text
FieldWrite {
  principal:        AuthenticatedPrincipal   // from the auth layer only
  source:           cli | service | pm_stack | github_comment | adapter
  work:             WorkRef
  resource:         ResolvedRef | None       // required for github_* and project_field
  field:            FieldName                // mapped to exactly one class
  expected_version: Version | Baseline       // Fleet row version, or external revision
  reason:           ReasonCode
  operation_id:     OperationId              // REL-03; required for every non-dry-run write
  visibility:       Private | Internal | PublicProjection
  owner_policy_ref: Option<OwnerPolicyRef>   // only where PRI-01 asks for it
  dry_run:          bool
}
```

Nothing in the request is taken from a client-supplied authority field.
A request that carries `actor_ref`, a role name or a capability list in
its payload has those ignored for the decision and recorded as
`ClientSuppliedAuthorization` evidence, as DST-01 already does.

`ResolvedRef` is the HYD-02 identity tuple (provider, host,
repository id, kind, remote id, qualifier) after resolution. A number
alone, a URL that resolves to two kinds, or a ref that has not been
resolved is not a `ResolvedRef`.

## 5. Decision order

The decision is a pure function of the request, the field matrix
version, the grant set and the current versions the caller read. It is
evaluated in this order and stops at the first failure, so the refusal
is always the most basic thing wrong:

1. **Authenticated.** No principal → `UNAUTHENTICATED`.
2. **Operation id.** A non-dry-run write without one →
   `OPERATION_ID_REQUIRED`.
3. **Classified.** Unknown field → `FIELD_UNCLASSIFIED`.
4. **Writable here.** Routed classes return `Routed { owner }`;
   `github_managed` returns `Routed { AUTH-03 | AUTH-04 }`;
   `creation_provenance` after creation and `project_field` from any
   writer other than the configured sole writer are refused
   (`PROVENANCE_WRITE_ONCE`, `PROJECT_SOLE_WRITER`).
5. **Grant.** DSP-01 `authorize(principal, verb, repo)` with the verb
   from §8 → `GRANT_MISSING` (carrying the DSP-01 sub-reason).
6. **Source floor.** DSP-08 floor for the source → `FLOOR_NOT_MET`.
7. **Resolved ref.** `github_resource` without a single `ResolvedRef` →
   `AMBIGUOUS_RESOURCE_REF`.
8. **Version.** Fleet fields compare `expected_version` with the row
   (`VERSION_CONFLICT`); GitHub fields compare the external revision the
   caller read with the one the adapter reads just before the call
   (`EXTERNAL_REVISION_STALE`). A missing version is a conflict, never
   "latest wins".
9. **Reason.** `ReasonCode::check_field_effect` for the attempted effect
   → `EFFECT_NOT_PERMITTED`.
10. **Visibility.** `fleet_note` with anything but `Private`, or
    `public_status_reason` with a value outside the registry →
    `VISIBILITY_VIOLATION`.
11. **Confirmation.** High-impact writes named in §8 return
    `ConfirmationRequired { preview }` until a confirmation bound to the
    same `operation_id` and preview digest arrives.

Results: `Allowed`, `Routed { owner }`, `ConfirmationRequired`, or
`Refused { code, current }`. `current` names the owner, the current
value's version or revision and the remediation, as component §4 asks
("typed conflict with current owner/value, source revision and
remediation"); it never includes the value of a field the caller cannot
read (§7).

`dry_run` runs steps 1–11 and returns the same result with no store,
journal, outbox or remote effect. Authority is evaluated again at intent
write and immediately before any remote call (DSP-08 §7); a change in
between ends the operation with `authorization_revoked`.

## 6. Rules the negatives pin down

- **Direct Project write.** Any `project_field` write whose principal is
  not the configured sole writer is `PROJECT_SOLE_WRITER`, whatever its
  grant, role or GitHub permission. Mobile, queue, agents and the local
  CLI consume the projection; they ask the sole writer through its own
  interface. The sole writer's identity is a value in the matrix (§9),
  set from the #9198 census, not a code constant.
- **Ambiguous GitHub ref.** `#123` without a repository, a ref that
  could be an issue or a PR, a renamed repository whose id disagrees
  with its name, or a cross-repository ref outside the grant all fail
  at step 7 before any version is read. HYD-02's `SOURCE_UNRESOLVED`
  is the upstream error; AUTH-01 refuses rather than resolves.
- **Stale external revision.** GitHub has no general compare-and-swap
  (component §4, managed content honesty). AUTH-01 therefore compares
  before the call and the adapter re-reads after it; a mismatch either
  side is reported, never retried blindly. The record does not call the
  pre-check optimistic locking.
- **Status reason leaks note.** The status reason has no text input.
  A request that puts note text in `reason` or in a status field is
  `VISIBILITY_VIOLATION`; the test feeds a private note body and checks
  that no rendered status, journal `PublicProjection` entry or outbox
  payload contains any of its text.
- **Forged owner.** An `actor_ref` or payload role that claims the owner
  changes nothing; PRI-01 sees the authenticated principal only.

## 7. Read policy

HYD-04 (§6.2) asks for one read decision per field class. AUTH-01
gives it:

| Class                                       | Who may read                                                 | Hydration                    |
| ------------------------------------------- | ------------------------------------------------------------ | ---------------------------- |
| `fleet_note` (`JournalVisibility::Private`) | the note's author and principals with an explicit note grant | **never**; export is AUTH-02 |
| journal `Internal`                          | principals with a grant on the work's repository             | allowed                      |
| journal `PublicProjection`                  | anyone who can read the work                                 | allowed                      |
| queue row non-note fields                   | principals with a grant on the work's repository             | allowed                      |
| `github_resource`, `project_field`          | whoever the source lets read it                              | as the source allows         |

A field withheld by this policy is absent from the result, not
redacted in place. A source withheld whole is `forbidden / policy`. The
decision names the matrix version it applied and nothing about what it
withheld. This matches the interim HYD-04 policy exactly, so HYD-04 can
switch to calling AUTH-01 without widening anything.

## 8. Verbs and confirmations

DSP-01 has no verb for a field write. This record proposes one, as a
DSP-08 policy change with its own receipt:

| Verb       | Severity | PM role floor                               | GitHub floor                                    | Confirmation                |
| ---------- | -------- | ------------------------------------------- | ----------------------------------------------- | --------------------------- |
| `metadata` | work     | `orchestrator`, or `agent` holding the task | `triage` for labels; `write` for title and body | none, except the rows below |

Field classes reuse existing verbs where one fits: `fleet_lifecycle`
writes use the lifecycle verbs of DSP-08 (`start`, `park`, …),
`fleet_priority` uses `priority`, and `fleet_relation` uses the
`work.link` row CLI-12 §5 proposes (`link_unauthorized`,
`VERSION_CONFLICT`), which this record accepts unchanged.

Confirmation is required, bound to `operation_id` and a preview digest,
for: closing or reopening a GitHub issue or PR, removing a relation
that unblocks work, and any write the owner policy marks high impact.
Merge and note mirror keep their own confirmation (DSP-08 §8, AUTH-02
§4).

## 9. Policy as data

The matrix is versioned data, `fleet.field-authority/v1`: field name →
class, class → authority and verb, the sole Project writer reference,
and the confirmation list. It lives beside the DSP-08 verb table and
changes the same way, by a policy change with a receipt, never by an
edit inside a decision function. Every decision and every refusal
records the matrix version, so an audit can say which rules applied.
No new digest domain is needed: the version string is the reference.

## 10. Errors

| Code                      | Meaning                                                     | Mutates |
| ------------------------- | ----------------------------------------------------------- | ------- |
| `UNAUTHENTICATED`         | no authenticated principal                                  | no      |
| `OPERATION_ID_REQUIRED`   | non-dry-run write without an operation id                   | no      |
| `FIELD_UNCLASSIFIED`      | field name maps to no class                                 | no      |
| `PROVENANCE_WRITE_ONCE`   | creation provenance written after creation                  | no      |
| `PROJECT_SOLE_WRITER`     | Project field written by someone other than the sole writer | no      |
| `GRANT_MISSING`           | DSP-01 refused (no grant, verb or repository)               | no      |
| `FLOOR_NOT_MET`           | source floor below the DSP-08 row                           | no      |
| `AMBIGUOUS_RESOURCE_REF`  | GitHub write without one resolved ref                       | no      |
| `VERSION_CONFLICT`        | Fleet row version differs or is missing                     | no      |
| `EXTERNAL_REVISION_STALE` | GitHub revision changed since the caller read it            | no      |
| `EFFECT_NOT_PERMITTED`    | reason code does not permit this effect                     | no      |
| `VISIBILITY_VIOLATION`    | note made non-private, or status reason outside registry    | no      |

`Routed` and `ConfirmationRequired` are results, not errors. Every
refusal is decided before the local transaction, so no refusal leaves a
partial write.

## 11. Migration, tests and gates

**Migration.** Two stages, as the issue requires:

1. **Audit only.** Writers call the decision and record what it would
   have returned next to what they did. Nothing is refused. The audit
   shows which current writers would be refused and why.
2. **Enforce.** Once the audit shows no unexplained refusal for one
   writer, that writer enforces. Writers switch one at a time.

Rollback turns enforcement off and returns the writer to audit only. It
stops new adapter writes that depend on the new rules; it never creates
a second authority, never re-enables a second Project writer and never
lets a queue title flow to GitHub.

**First failing oracles** (all in `work/authority.rs`, prefix
`pmo_auth_`):

1. `pmo_auth_rejects_unscoped_project_write` — an agent with a full
   grant writes a Project field: `PROJECT_SOLE_WRITER`, no effect.
2. `pmo_auth_protected_priority_requires_owner_policy` — a priority
   write to a protected item returns `Routed { PRI-01 }`, and PRI-01
   refuses it without the owner policy; a forged owner `actor_ref`
   changes nothing.
3. `pmo_auth_queue_title_does_not_mutate_pr_title` — an allowed queue
   title write produces exactly one effect, on the queue row.
4. `pmo_auth_ambiguous_github_ref_refused` — a bare number and an
   issue-or-PR ref both fail at step 7.
5. `pmo_auth_stale_external_revision_refused`.
6. `pmo_auth_status_reason_never_carries_note_text`.
7. `pmo_auth_missing_operation_id_refused`.
8. `pmo_auth_unclassified_field_refused`.
9. `pmo_auth_creation_provenance_write_once`.
10. `pmo_auth_refusal_order_is_most_basic_first` — a request wrong in
    three ways reports the earliest step.
11. `pmo_auth_dry_run_has_no_effect` and
    `pmo_auth_read_policy_withholds_private_notes`.

**Done command:** confirm the test names above exist, then
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib
--features client,queue-broker pmo_auth_`.

**Gates before code:** the #9198 census names the sole Project writer
reference; CEN-04 (#9434) lists `fleet.field-authority/v1` in its schema
owner matrix; the `metadata` verb row is accepted as a DSP-08 policy
change. Gates before enforcement: the audit stage has run for the
writer being switched.

## 12. Requirement trace (design-level)

| Requirement (issue #9429)                                       | Where              |
| --------------------------------------------------------------- | ------------------ |
| Every write checks principal capability and scope               | §5 steps 1, 5, 6   |
| Work, ref and field are explicit                                | §3, §4, §5 step 7  |
| Expected version                                                | §5 step 8          |
| Reason                                                          | §5 step 9          |
| Visibility                                                      | §5 step 10, §6, §7 |
| Operation id                                                    | §5 step 2          |
| Negative: direct Project write                                  | §6, oracle 1       |
| Negative: ambiguous GitHub ref                                  | §6, oracle 4       |
| Negative: stale external revision                               | §6, oracle 5       |
| Negative: status reason leaks note                              | §3, §6, oracle 6   |
| Migration: audit before enforcement; rollback adds no authority | §11                |
| Docs: field matrix and operator policy                          | §3, §8, §9         |

## 13. Non-goals

- No new authentication, role, token or GitHub App.
- No decision on who the sole Project writer is (#9198).
- No priority, lifecycle or dependency rule; those owners keep theirs.
- No note mirror, managed comment or body-section rule beyond the
  hand-off (AUTH-02, AUTH-03, AUTH-04).
- No compare-and-swap claim for GitHub writes.
- No code, schema or remote write in this change.
