# DST-04 — Priority label events through the Projector intake to Fleet authority

Issue: [#9454](https://github.com/developing-today/code/issues/9454)
(slice `DST-04` of [#9200](https://github.com/developing-today/code/issues/9200),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
The issue asks to "map already-authenticated, delivery-deduplicated
priority label/removal events into the shared Fleet command contract
and reconcile echoes without granting the disposable projector
authority", reusing the existing HMAC intake and hot ledger and adding
"no policy or remote mutation to webhook.ts". This record decides which
label events become commands, what each command says, how one human act
gets one command identity, how Fleet's own label writes are told apart
from human intent, and where each refusal comes from. It authorizes no
code, schema, deployment or remote write.

Governing decisions consumed, not re-opened:

- **Component #9200** (`9200-priority-distribution.md`): the Projector
  "may verify webhook HMAC and deduplicate transport delivery, but actor
  authorization and canonical priority acceptance belong to Fleet
  authority"; "a mutable mirror label is not membership authority";
  "projection events do not feed back as new human intent; source
  operation/correlation IDs distinguish echo"; reason codes
  `label_band_change`, `label_removed_needs_triage`, `label_conflict`.
- **#7030 and PRI-01** (`work/priority.rs`): protected `expedite = 15`
  and `next-up = 25`; label normalization and label removal are
  `WriteSource`s that never amend the protected set. Only the
  authenticated configured owner action does.
- **DSP-01** (#9445, closed): the Projector's `operation-forwarder.ts`
  builds an envelope with a semantic idempotency key, never the delivery
  GUID, and hands it to an injected `FleetOperationSink`, which owns
  authorization and durable acceptance.
- **DST-01** (#9451, closed): the shared `Command` / `Result` contract
  in `work/contracts.rs`; authorization reads the authenticated
  principal only.
- **AUTH-01** (`doc/2026-11-25T22-13-03Z_design_auth01_field_ownership_scoped_authorization/`):
  a priority write is `Routed { PRI-01 }`; GitHub label writes go
  through `fleet gh` only.
- **REL-09** (`doc/2026-11-25T22-14-37Z_design_rel09_last_activity_gates/`):
  for completion freshness, a label change is a non-invalidating
  `LabelChange`.

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **Intake.** `pm-stack/projector/src/server.ts` serves
  `POST /github/webhook`: bounded raw body, HMAC-SHA256 through
  `src/webhook.ts`, a required `x-github-delivery` GUID and delivery
  dedupe that counts only `applied` or `ignored` outcomes.
- **Labels are projected, not forwarded.** `projector.ts` stores an
  issue's label names on the projected row. Nothing turns a label event
  into a command.
- **Forwarder.** `operation-forwarder.ts` (DSP-01) handles `/fleet`
  comment commands only. `server.ts` calls `operationEnvelopeFor` only
  when a sink is configured, and the default wiring configures none, so
  forwarding is inert.
- **Fleet side.** `contracts.rs` has `ReasonCode::LabelBandChange`,
  `LabelRemovedNeedsTriage`, `LabelConflict`, `ProjectionEcho` and
  `StaleSnapshot`, each with its permitted effects. `priority.rs` has
  `check_priority_write` and the ratified profile. `work/distribution.rs`
  (DST-03) does not exist yet.

## 1. Intent

A person who adds `priority/critical` to an issue means "move this
up". Fleet
should hear that as a priority request from that person and decide it
under the same rules as any other priority request. Fleet's own label
writes, which mirror decisions already made, must never come back as
new requests. A label on a protected item must never change the item,
whoever added it.

## 2. Ownership

| Concern                                    | Owner                                     | This slice                 |
| ------------------------------------------ | ----------------------------------------- | -------------------------- |
| HMAC, bounded body, delivery GUID dedupe   | `projector/src/webhook.ts`, `server.ts`   | read-only consumer         |
| Label event → envelope mapping             | `projector/src/operation-forwarder.ts`    | **owns** the label path    |
| Envelope → shared `Command` mapping        | `work/distribution.rs` (DST-03 file)      | **owns** the label mapping |
| Authorization, acceptance, durable receipt | Fleet sink (DSP-01), AUTH-01, PRI-01      | consumer                   |
| Accepted-intent write                      | DST-03                                    | consumer                   |
| Outbound label writes (the echo source)    | `reprioritize_sync_outbox`, `fleet gh`    | read-only consumer         |
| Projector tests                            | `projector/test/priority-webhook.test.ts` | **owns**                   |

No new service, route, queue, table, client or HMAC helper. The
Projector gains no policy and no remote write.

## 3. Which events become commands

Only `issues` and `pull_request` events with action `labeled` or
`unlabeled`, where the label named in the event is in the **label
mapping** below, produce a label envelope. Every other label event is
projected as today and forwards nothing.

Two label families exist on main, and neither names a band exactly:

- **Category labels** `priority/critical`, `priority/high`,
  `priority/normal`, `priority/low`. Fleet writes them itself
  (`validate::label_for_priority`, through `reprioritize_sync_outbox`),
  and each covers a range of values (`category_for_priority`: up to 10,
  up to 30, up to 50, above 50). They are lossy: 15 and 25 both render
  as `priority/high`.
- **Rank labels** `expedite`, `now`, `next`, read by the console
  (`console/src/ops.ts` `RANKS`). `expedite` names #7030's protected
  set.

The **label mapping** is versioned data in the priority profile. It
lists, for each label, the range it covers and the value to set when the
current value falls outside that range. This record proposes the
category ranges above, and no representative values. Until the owner of
the profile (#9199 / #9200) sets those values, a category label that
would move a value returns `LABEL_MAPPING_UNCONFIGURED` and a triage
hint, and changes nothing. Nothing here invents a band.

The envelope carries **the full mapped-label set after the change**,
taken from the payload's `issue.labels` (or `pull_request.labels`), not
only the delta. Fleet decides from the resulting set and the item's
current value:

| Resulting mapped-label set                 | Result                                          | Reason code                  |
| ------------------------------------------ | ----------------------------------------------- | ---------------------------- |
| one label, current value inside its range  | `no_change`; the value is kept, so 15 stays 15  | none                         |
| one label, current value outside its range | `priority.set_band` to the mapped value         | `label_band_change`          |
| `expedite` added or removed                | protected-membership write; PRI-01 decides (§7) | `label_band_change`          |
| none, after an `unlabeled`                 | `priority.request_triage`; base value kept      | `label_removed_needs_triage` |
| two or more labels                         | no mutation; triage hint                        | `label_conflict`             |

The first row is what keeps the ratified values: a category label that
merely agrees with the stored value never rewrites it. Nothing in this
path snaps 15 to 10 or 25 to 20. A label that is not in the mapping
forwards nothing.

## 4. One act, one identity

Delivery dedupe and command identity stay separate (#9200, DSP-01):

- **Delivery GUID** remains the Projector's transport key. It is copied
  into the envelope as provenance only.
- **Idempotency key:**
  `ghlabel:v1:<repo_id>#<number>:<label_set_digest>:<issue_updated_at>`.
  A label webhook carries no stable event id the way a comment carries a
  comment id, so the key is built from what the payload proves:
  - the repository id, not its name, which survives a rename;
  - the issue or PR number;
  - the SHA-256 digest of the sorted priority-label set;
  - the payload's `updated_at` value.

  A redelivery of the same delivery produces the same key, and so the
  sink returns its stored receipt. The timestamp is part of identity
  only. It never orders commands.
- **Polls do not create commands.** Reconcile reads (`/reconcile`)
  update the projection only. If they could also forward, one act would
  arrive under two identities.
- **Value idempotence backs it up.** Setting a band an item already
  holds changes nothing, and the receipt says `no_change`.

## 5. Stale and out-of-order events

GitHub may deliver two label events out of order. Arrival time decides
nothing. Fleet authority checks that the event's set is still true
before it accepts:

1. The sink reads the item's current labels through the `fleet gh`
   adapter.
2. If the current priority-label set differs from the envelope's set,
   the command is `stale_snapshot`. It makes no change and adds no hint,
   because a newer event, or the next reconcile, carries the current
   state.
3. Only an envelope whose set matches the fresh read goes on to
   authorization.

This is a freshness check, not a lock. A change between that read and
acceptance is caught by the work version that DST-03 records
(`expected_work_version`), and is reported as `VERSION_CONFLICT`.

## 6. Echoes

Fleet writes priority labels itself, to mirror decisions already
accepted: today `verb_reprioritize` queues a `priority/<category>` label
in `reprioritize_sync_outbox`, and the broker drains it through
`fleet gh`. REL-04's intent rows take that role once they exist. Those
writes come back as `labeled` / `unlabeled` webhooks. An event is an
**echo** when both of these hold:

- the event's sender is one of the configured Fleet GitHub Apps, matched
  by app id. Login is not used, because it rotates among three Apps
  (`gh.rs`);
- an outbox row exists (`reprioritize_sync_outbox`, or a REL-04 intent
  later) for the same repository, number, label and action, and is
  pending or was delivered within the reconcile window.

An echo is `projection_echo`: no mutation, no hint. The receipt names
the matching operation id. An event from a Fleet App with **no**
matching effect is not treated as human intent and is not trusted as an
echo either. It is `label_conflict`, with a hint that names the App, so
that someone can find the writer that bypassed the outbox.

A human's label event never matches an outbox effect, because the
sender check comes first.

## 7. Authorization

The sink authorizes the event's `sender` (login and numeric id from the
signed payload) as the actor, exactly as DSP-01 does for comment
commands:

- **Grant:** DSP-01 `authorize(principal, Verb::Priority, repo)`. With
  no grant, the result is `GRANT_MISSING`.
- **Floor:** DSP-08 gives `priority` a GitHub floor of `maintain`. Fleet
  reads the sender's repository permission; the Projector does not
  supply it. Below the floor, the result is `FLOOR_NOT_MET`.
- **Owner policy:** the write reaches PRI-01 as
  `WriteSource::LabelNormalization`, or `LabelRemoval` for the triage
  case, with the current `owner_policy_ref`. A protected item returns
  `OWNER_POLICY_PROTECTED` whoever the sender is, including the owner's
  own account. A label is not an owner action, and removing `expedite`
  does not unprotect the item. A stale policy reference returns
  `OWNER_POLICY_VERSION_CONFLICT`. A missing or unverifiable policy
  fails closed.

Refusals change nothing in Fleet and write nothing to GitHub. Nothing
in this path reverts the human's label. A refused label stays on the
issue until reconcile shows the projection disagrees, which is the
consumers' "stale/conflict shown explicitly".

## 8. The envelope

The label path adds one envelope kind beside DSP-01's comment
envelope:

```text
LabelEnvelope {
  kind:            "priority-label"
  idempotencyKey:  "ghlabel:v1:…"           // §4
  deliveryId:      string                   // provenance only
  event:           "issues" | "pull_request"
  action:          "labeled" | "unlabeled"
  sender:          { login, id, appId? }    // from the signed payload
  repoId:          number
  repo:            "owner/repo"             // display only
  number:          number
  priorityLabels:  string[]                 // sorted resulting set, §3
  changedLabel:    string
  updatedAt:       string                   // identity only, §4
}
```

DST-03's `distribution.rs` maps it to the shared `Command`:
- `kind` is `priority.set_band` or `priority.request_triage`;
- `actor_ref` is `github:user:<id>`, which is recorded and never used
  for authority;
- `source_event_ref` is `github:delivery:<guid>`;
- `observed_snapshot_ref` is the label-set digest;
- the reason code comes from §3;
- `owner_policy_ref` is read fresh by the sink.

## 9. Errors and results

| Result                                 | When                                           | Mutates |
| -------------------------------------- | ---------------------------------------------- | ------- |
| accepted, `label_band_change`          | §3 one label; authorized; fresh; not protected | yes     |
| accepted, `label_removed_needs_triage` | §3 none; authorized; fresh                     | hint    |
| `label_conflict`                       | two or more labels, or an unmatched App sender | hint    |
| `projection_echo`                      | §6                                             | no      |
| `stale_snapshot`                       | §5 set differs from the fresh read             | no      |
| `no_change`                            | band already held                              | no      |
| `LABEL_MAPPING_UNCONFIGURED`           | category label would move a value; no mapping  | hint    |
| `AMBIGUOUS_RESOURCE_REF`               | repo id and name disagree, or not resolvable   | no      |
| `GRANT_MISSING`, `FLOOR_NOT_MET`       | §7                                             | no      |
| `OWNER_POLICY_PROTECTED`               | §7, protected item                             | no      |
| `OWNER_POLICY_VERSION_CONFLICT`        | §7, stale policy ref                           | no      |
| `VERSION_CONFLICT`                     | §5, work version moved                         | no      |
| sink unavailable                       | Fleet authority down                           | no      |

When the sink is unavailable, the Projector records the delivery as an
error. It is not counted as handled, so GitHub's redelivery or a manual
redelivery retries it, and the Projector never fabricates a receipt
(DSP-01). Every result is a Fleet receipt; the Projector stores none.

## 10. Tests and gates

First failing oracles (from the issue):

1. `pmo_distribution_priority_webhook_maps_to_shared_command` (Fleet):
   with a fixture label mapping, a signed `labeled priority/critical`
   envelope on an item at 50 maps to `priority.set_band` with the
   fixture's mapped value, `label_band_change`, and the §4 key.
2. `pmo_distribution_protected_expedite_label_event_is_denied` (Fleet):
   a label event on a protected item returns `OWNER_POLICY_PROTECTED`,
   including when the sender is the owner's account. Removing
   `expedite` leaves membership unchanged.

Further oracles:

3. `pmo_distribution_label_redelivery_returns_stored_receipt`.
4. `pmo_distribution_two_priority_labels_conflict_without_mutation`.
5. `pmo_distribution_final_label_removal_keeps_value_and_requests_triage`.
6. `pmo_distribution_fleet_app_echo_is_not_intent`.
7. `pmo_distribution_unmatched_fleet_app_label_is_conflict`.
8. `pmo_distribution_out_of_order_label_is_stale_snapshot`.
9. `pmo_distribution_label_values_are_not_snapped`: `priority/high` on
   an item at 15 or 25 is `no_change`.
10. `pmo_distribution_unconfigured_category_mapping_changes_nothing`.

The Projector suite, `test/priority-webhook.test.ts`, checks four
things:
- a priority label event forwards exactly one envelope;
- a non-priority label forwards none;
- a reconcile poll forwards none;
- with no sink configured, nothing is forwarded and the projection
  applies as today.

**Done command:** list both names with `-- --list`, then
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib
--features client,queue-broker pmo_distribution_`. Run the Projector
suite with `bun test test/priority-webhook.test.ts`, dispatched
remotely.

**Gates.** Code waits on DST-03 (#9453), which owns `distribution.rs`
and the accepted-intent write. Attaching a sink in production, and any
deployment, needs its own approval. Until a sink is attached, the label
path is inert in the same way the DSP-01 comment path is.

**Rollback.** Stop building label envelopes. Projection carries on
unchanged, accepted intents stay, and no label on GitHub is touched.

## 11. Requirement trace (design-level)

| Issue requirement                                         | Where      |
| --------------------------------------------------------- | ---------- |
| Map authenticated, delivery-deduped label events          | §3, §4, §8 |
| Reconcile echoes without granting the projector authority | §6, §7     |
| Reuse HMAC intake and ledger; nothing new in `webhook.ts` | §2, §4     |
| Delivery dedupe distinct from operation idempotency       | §4         |
| Negative: stale input                                     | §5         |
| Negative: duplicate / replay                              | §4         |
| Negative: ambiguous resource                              | §9         |
| Negative: unauthorized actor                              | §7         |
| Negative: unknown version                                 | §3, §7     |
| Negative: no false verified result                        | §9         |
| Protected expedite gate (#7030)                           | §7         |
| Final-label removal keeps the score and requests triage   | §3         |

## 12. Non-goals

- No change to `webhook.ts`, the delivery ledger or the hot cursor.
- No Projector write to GitHub, Project or Fleet state.
- No label taxonomy change, no representative values for category
  labels, and no re-ratification of 15 and 25.
- No automatic revert of a refused human label.
- No `do-now` or urgency mapping from labels.

## 2026-11-25T22:40Z Correction:

The first version of §3 assumed GitHub labels named `expedite` and
`next-up` that map one to one to bands. On main, Fleet's own labels are
the lossy `priority/<category>` family, and the console reads the rank
labels `expedite`, `now` and `next`. §3 now maps labels through a
versioned mapping. A label that agrees with the stored value changes
nothing, and a category label with no configured value changes nothing
and asks for triage. §6 names `reprioritize_sync_outbox` as today's
echo source. `UNKNOWN_PROFILE_LABEL` is replaced by
`LABEL_MAPPING_UNCONFIGURED`, and oracle 10 is added.
