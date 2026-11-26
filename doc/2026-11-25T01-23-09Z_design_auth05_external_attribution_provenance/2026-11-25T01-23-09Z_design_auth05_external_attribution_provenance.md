# AUTH-05 — Remote-actor provenance and uncertain origin

Issue: [#9432](https://github.com/developing-today/code/issues/9432)
(slice `AUTH-05` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal; not an authorization to implement.**

This record turns the AUTH-05 slice of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225, component #9208) into a typed provenance schema. The
schema says who and what caused a remote event, how far each claim
can be trusted, and who may see which field. HYD-07's classifier
reads these records. A separately scoped implementation issue can
be filed against this record. It authorizes no code, schema,
migration, deployment or live acceptance.

Governing decisions consumed, not re-opened:

- **Component #9208 AUTH-05.** `src/work/authority.rs` owns the
  provenance *schema and fixture only*. #9207 HYD-07 is the single
  owner of activity classification. AUTH-05 adds no second
  classifier and no writer. The cross-component acceptance
  `pmo_hyd_external_only_includes_uncertain_shared_actor` stays in
  HYD-07.
- **HYD-07 design record** (`doc/2026-11-23T12-19-51Z_design_hyd07_operation_session_attribution/`):
  - The attribution map is written at write time from the write's
    own receipt.
  - The decision order is own_session, then fleet_other, then
    external, then uncertain.
  - A username never assigns a session.
  - Coincidence is not evidence.
  - Its rule 3, *external*, needs "the event's provenance (AUTH-05
    record) positively names a principal that is not a shared Fleet
    identity". This record defines that test.
- **Charter C-R121/C-R122.** Session attribution needs Fleet
  operation and session ids, not only a GitHub username. An event
  that cannot be attributed is uncertain and is not silently left
  out.
- **`architecture.md`.** Identifiers use `snake_case`. Provider and
  session identifiers stay opaque strings. Fleet integers are
  canonical decimal strings.
- **Drew's standing note:** "y'all share my username". Agents act
  through the owner's own GitHub account as well as through App
  installations. A human-typed login is therefore not proof of a
  human.

## 1. What exists at main

- `src/work/authority.rs` holds AUTH-04's managed-body capability
  policy: `ConditionalWriteCapability`, `parse_managed_section`,
  `plan_update` and `reconcile_after_patch_timeout`. It is pure, does
  no I/O, and has its own tests. It contains nothing about
  provenance.
- `src/work/contracts.rs` has `AuthenticatedPrincipal
  { principal_ref, capabilities }`. That is the **caller** of a Fleet
  command, supplied by the auth layer. AUTH-05 is about a different
  party: the **remote actor** of an event Fleet observed on GitHub.
  The two must not be merged. A command caller is authenticated by
  Fleet. A remote actor is only *claimed* by the provider's payload.
- No code anywhere reads GitHub's `performed_via_github_app`. Actor
  identity in the aggregator and projector is the login string
  (`model.ts` `Comment.author`).
- These prerequisites are open: AUTH-01 #9429 (a shared-file
  predecessor on `authority.rs`), CLI-02 #9438, REL-03 #9513 and
  HYD-06 #9508. This record specifies against their stated outputs.
  It does not specify against running code.

## 2. Decisions

### D1. Two separate claims: who acted, and which operation caused it

A provenance record answers two questions and never lets one answer
the other:

1. **Actor:** which provider principal the source says performed the
   event. This is evidence *about the source's account*. It can only
   ever show that an event is "positively not Fleet's" (HYD-07
   rule 3).
2. **Cause:** which Fleet operation and session wrote it. This is
   present only when Fleet recorded it itself, at write time, as an
   `OperationRef`. It never comes from the actor, the body or the
   timing.

```rust
pub struct ProvenanceRecord {
    pub source_ref: SourceRef,            // canonical source: provider + stable repository id + object kind
    pub remote_id: String,                // source-native id of the object/event (opaque)
    pub remote_revision: Option<String>,  // revision/updated marker of *this* event, if the source gives one
    pub actor: ActorClaim,                // D2
    pub cause: Option<OperationRef>,      // D3; None unless Fleet recorded it at write time
    pub observed_at: String,              // when Fleet read the event (RFC 3339)
}
```

`ProvenanceRecord` has no body, body hash, title or timestamp-match
field. Coincident same-body text, the named negative case, therefore
has nowhere to enter attribution.

### D2. The actor claim is keyed by stable id, not login

```rust
pub struct ActorClaim {
    pub provider: Provider,                    // github (only value at V1)
    pub principal_id: Option<String>,          // provider's stable numeric/node id, canonical string
    pub principal_type: Option<PrincipalType>, // user | bot | organization | unknown, as the payload states
    pub via_app_id: Option<String>,            // performed_via_github_app.id when present
    pub login_display: Option<String>,         // display only; never compared for trust
}
```

- `login_display` is a display string only. No function in the
  module branches on it. A test asserts this by renaming the login
  while keeping the id, and expecting identical trust.
- `principal_id: None` means the payload gave no stable id. The
  record's trust is then `Unknown` (D4).

### D3. `OperationRef`: opaque, recorded and validated

```rust
pub struct OperationRef {
    operation_id: OpaqueId,   // REL-03 operation identity
    session_id: OpaqueId,     // #9206 session identity
    outbox_id: Option<OpaqueId>, // #9209, when delivered through the outbox
}
```

- `OpaqueId` is a validated newtype:
  - non-empty;
  - at most 256 bytes;
  - no control characters;
  - not only whitespace;
  - compared byte-for-byte;
  - never parsed, split or case-folded.
- `OperationRef` has **one** constructor:
  `OperationRef::from_write_receipt(&WriteReceipt)`. `WriteReceipt`
  is the REL-03/#9209 effect receipt type, consumed as a trait until
  REL-03 lands. No constructor takes a login, an `ActorClaim` or a
  body. This is how the first oracle is met: "rejects actor-username
  -only attribution and requires opaque provider session/ref
  strings". Attribution without an operation and session reference
  cannot be expressed in the type.
- Deserialization uses `#[serde(deny_unknown_fields)]` and runs the
  same `OpaqueId` validation. A stored record cannot smuggle in an
  empty or whitespace session id. Such a record fails to load. It is
  not coerced into a value.
- HYD-07's `AttributionEntry` embeds `OperationRef` rather than
  redefining these fields. That is the "consumer integration edge" to
  #9207.

### D4. Trust classification is a pure lookup against a shared-identity registry

```rust
pub enum ActorTrust {
    SharedFleet,       // a principal or App that Fleet sessions write through
    DistinctHuman,     // a user principal positively not shared with Fleet
    DistinctSystem,    // a bot/App principal positively not Fleet's
    Unknown,           // anything not positively one of the above
}

pub fn actor_trust(claim: &ActorClaim, registry: &SharedIdentityRegistry) -> ActorTrust
```

`SharedIdentityRegistry { version: String, principal_ids: BTreeSet<String>, app_ids: BTreeSet<String> }`
is a value passed in by the caller. The #9206 identity contract
supplies it, from the identities Fleet sessions actually authenticate
as. The registry is **not** a new store. Until #9206 lands, the
caller passes it from configuration.

Rules, applied in order, first match wins:

1. If `via_app_id` is in `registry.app_ids`, the result is
   `SharedFleet`. This covers the "shared App token" negative case:
   any actor acting through a Fleet App installation is shared, even
   if it is a user-to-server token showing a human login.
2. If `principal_id` is in `registry.principal_ids`, the result is
   `SharedFleet`. This includes the owner's own account, which
   agents share. The owner's real human edits are therefore *never*
   `DistinctHuman`. With no operation map they classify `uncertain`
   and are still shown, which is the safe direction (HYD-07 §3).
3. If the registry is unloaded (`version` empty), or `principal_id`
   or `principal_type` is missing, the result is `Unknown`. The
   classification fails closed. An empty registry must never make
   every bot look distinct.
4. If `principal_type = user`, the result is `DistinctHuman`.
5. If `principal_type ∈ {bot, organization}` and `via_app_id` is
   absent or not Fleet's, the result is `DistinctSystem`.
6. Otherwise the result is `Unknown`.

`actor_trust` returns no "Fleet session" outcome. Only
`OperationRef` can produce one, and HYD-07 does that join. Even
`SharedFleet` means only "could have been any Fleet session or the
owner by hand".

### D5. Visibility

The provenance fields have three visibility classes. This record
fixes the classes and leaves rendering to its consumers:

| Field | Class | Who may see it |
| ----- | ----- | -------------- |
| `source_ref`, `remote_id`, `remote_revision`, `observed_at` | `authority_reader` | any authenticated reader of the work's authority |
| `actor.login_display`, `actor.principal_type`, derived `ActorTrust` | `authority_reader` | same; the source already shows these publicly |
| `cause.operation_id`, `cause.outbox_id` | `authority_reader` | same; operation ids are already in result envelopes |
| `cause.session_id` | `session_scoped` | the session itself, and principals with an explicit capability to read other sessions (#9206 names it); everyone else sees `other_session` |

- No provenance field is ever written to a public GitHub surface by
  this slice.
- Mirroring is AUTH-02's consent-gated path. A content marker
  (HYD-07 §2.2) carries a nonce, not a session id.
- `redact_for(reader: &AuthenticatedPrincipal, requesting_session)`
  is the one projection function. It is pure, and HYD-07's output
  calls it.

### D6. Negative cases map to type and rule, not to heuristics

| Negative case (issue) | Where it is stopped |
| --------------------- | ------------------- |
| Username-only attribution | `OperationRef` has no login constructor (D3); `login_display` is never read (D2) |
| Shared App token | D4 rule 1, giving `SharedFleet`. With no op map, HYD-07 classifies it `uncertain`, not external and not own_session |
| Coincident same body | `ProvenanceRecord` has no body field (D1) |
| Changed remote ID | The key is `(source_ref, remote_id)`. A re-keyed object has `cause: None` until a write path records the new id, so it classifies uncertain (HYD-07 §4) |
| Missing op map | `cause: None`. A legacy record is the same case. HYD-07 classifies it `uncertain` unless rule 3 shows it positively distinct |

### D7. Migration: old attribution is uncertain

Records built from events observed before the map exists get
`cause: None`. There is no backfill from logins, bodies or timing. A
historical event becomes attributed only if a write receipt for it
already exists (REL-03). Rollback disables the classifier's use of
`ActorTrust` (HYD-07 then labels these rows `uncertain`). It deletes
no records.

## 3. Placement in `authority.rs`

`authority.rs` already holds AUTH-04's policy, and the shared-file
edge after AUTH-01 stands. AUTH-05 adds one clearly delimited section:

- the doc header `//! Remote-actor provenance — AUTH-05`;
- the types in D1–D4, `actor_trust`, `redact_for` and `OpaqueId`;
- the tests module `provenance_tests`.

AUTH-05 does not touch the existing AUTH-04 items. It adds no new
file, crate, store or I/O. The `WriteReceipt` trait is defined here
only if REL-03 has not yet landed its receipt type. If it has, AUTH-05
uses that type. Either way the trait is the seam, and nothing is
duplicated.

## 4. Fixture

The fixture is
`pkgs/fleet/crates/fleet-queue/tests/fixtures/provenance/github-actors.json`.
It holds a table of `ActorClaim` inputs, a registry, and the expected
`ActorTrust`. It covers:

1. An App bot through a Fleet App.
2. A user-to-server token through a Fleet App with a human login.
3. The owner's account (in the registry) acting directly.
4. The same login renamed, with the same `principal_id`.
5. A distinct human.
6. A distinct third-party bot (Dependabot-shaped).
7. A missing `principal_id`.
8. An unloaded registry.
9. An organization actor.
10. A payload with an extra unknown field, which loads with the extra
    field ignored at the payload layer. The stored-record layer
    denies unknown fields (D3).

## 5. Tests (in `authority.rs`, package `fleet-queue`, features `client,queue-broker`)

1. `pmo_auth_provenance_requires_operation_session_refs` (the named
   red test). `OperationRef` can be built only from a write receipt
   with valid opaque operation and session ids. A serialized record
   with an empty, whitespace or over-long `session_id` fails to load.
   A record whose only identifying data is a login has
   `cause == None`.
2. `pmo_auth_provenance_login_never_changes_trust`: in fixture row 4,
   renaming `login_display` leaves `actor_trust` unchanged. A login
   equal to a registry member's login, but with a different id, is
   not `SharedFleet` by login. Its trust comes from the id only.
3. `pmo_auth_provenance_shared_app_token_is_shared_fleet`: rows 1–2.
4. `pmo_auth_provenance_shared_owner_account_is_not_distinct`: row 3.
5. `pmo_auth_provenance_unloaded_registry_is_unknown`: row 8.
6. `pmo_auth_provenance_missing_principal_id_is_unknown`: row 7.
7. `pmo_auth_provenance_has_no_body_channel`: a compile-time
   assertion via an exhaustive destructure of `ProvenanceRecord`. If
   someone adds a field, the test fails to compile until it is
   reviewed.
8. `pmo_auth_provenance_changed_remote_id_is_unattributed`
9. `pmo_auth_provenance_legacy_record_is_unattributed`
10. `pmo_auth_provenance_session_id_redacted_for_other_readers`
11. `pmo_auth_provenance_fixture_table`: every row of §4.

The run is remote only, per Drew's Nov 23 rule. Dispatch it with
`fleet queue dispatch --repo developing-today/code --ref <branch>
--command 'cargo test --manifest-path pkgs/fleet/Cargo.toml -p
fleet-queue --lib --features client,queue-broker pmo_auth_provenance_'`
on a GitHub-hosted runner. First assert that `-- --list` shows all 11
names. Zero tests is not a pass. Cross-component acceptance stays
`pmo_hyd_external_only_includes_uncertain_shared_actor` in HYD-07.

## 6. Boundaries

- **Owned:** the AUTH-05 section of `src/work/authority.rs` and the
  provenance fixture.
- **Consumes:** REL-03 receipts (operation_id), #9206 session
  identity and registry, and #9209 outbox ids.
- **Consumed by:** HYD-07 (`AttributionEntry.operation_ref`, rule 3 via
  `actor_trust`, output via `redact_for`).
- **Not here:** the classifier, the attribution map, `--external-only`,
  CLI registration (CLI-13), any GitHub client call or any write.

## 7. Open questions (not blocking)

- **Registry source.** Which identities populate the registry (App
  installation ids and the shared owner account) is #9206's to state.
  Until it does, configuration supplies the registry and the
  classification fails closed (D4 rule 3).
- **Non-GitHub providers.** For providers such as Kaneo and Slack,
  `Provider` is an enum, so adding a provider is a reviewed change and
  never a free string.

## 8. Gates

Implementation starts only on a separately scoped issue, after this
record merges and after AUTH-01, CLI-02, REL-03 and HYD-06 are
resolved. Any persisted provenance field waits for the #9241 QTI
crosswalk.
