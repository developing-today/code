# HYD-07 — Operation/session attribution and external-only semantics

**Status:** design proposal — not an authorization to implement. Design of record for #9509 (HYD-07, parent #9207, program #9196). The issue stays open as the tracker; implementation waits for its gates (§9).

**Governing decisions consumed:**

- #9207 component §4 ("External attribution maps Fleet `operation_id`/`session_id`/`outbox_id` to remote returned IDs and content markers. Shared GitHub App/bot login is not attribution. For edits without remote ID mapping, classify origin `uncertain`; `--external-only` includes uncertain by default and reports confidence.") and the HYD-07 slice text.
- #9208 AUTH-05 (#9432): `authority.rs` owns the trusted principal/operation provenance schema; HYD-07 is the one canonical classifier and does not get a second one.
- REL-03 design record (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`): the receipt's immutable identity block and `operation_id`.
- HYD-06 (#9508): per-source revisions and tombstones; an edit is a revision of the same logical item.

## Verified state at `origin/main` `66a102585`

- `pkgs/fleet/crates/fleet-queue/src/work/activity.rs` does not exist. There is no activity store, no attribution map and no `--external-only` flag in Fleet.
- `work/operations.rs` (DSP-01): `Principal` is `{ login, authenticated }`; `Receipt` is `{ idempotency_key, sequence, verb, resource }` and `Effect` adds `principal` (a login string) and `delivery_id`. Nothing records a remote ID returned by a write, a `session_id`, or an `outbox_id`.
- `work/authority.rs` holds AUTH-04's managed-section policy only; AUTH-05's provenance schema has not landed.
- In the repo today, the visible signals that "Fleet wrote this" are content markers (`<!-- agent-note:v1 … -->`, `<!-- fleet-claim v1 … -->`, the Claude Code footer) and the posting login. Every lane and session posts as the same GitHub login, so the login identifies no session, and a marker is text anyone can paste.

So the slice starts from nothing in Fleet: the contract below is new, and its inputs (REL-03 receipts, HYD-06 revisions, a session identity from CLI-05/#9502) are each still gated.

## 1. Intent

Answer one question for every observed source activity revision: **did a known Fleet operation cause this exact revision?** The answer is one of three origins, never a guess from who posted it:

| origin      | meaning                                                                                                        |
| ----------- | -------------------------------------------------------------------------------------------------------------- |
| `fleet`     | a current mapping ties this exact remote ID **and** revision to a recorded operation (and its session, if any) |
| `external`  | positive evidence it is not Fleet's: no mapping, and the actor is not a principal Fleet writes as              |
| `uncertain` | anything else: Fleet's shared login with no mapping, a mapped ID whose revision moved, a stale or legacy map   |

`--external-only` returns `external` **and** `uncertain` by default and says which is which. Hiding an `uncertain` event can make an agent miss a human's instruction; showing a Fleet echo costs only a line. The asymmetry is the point.

## 2. Ownership

| concern                                             | owner                                       |
| --------------------------------------------------- | ------------------------------------------- |
| operation identity, receipt, remote IDs returned    | REL-03 receipt identity block (#9513)       |
| session identity                                    | CLI-05 / #9502 lifecycle receipts           |
| provenance schema, which principals Fleet writes as | AUTH-05 `authority.rs` (#9432)              |
| per-source revision and tombstone store             | HYD-06 (#9508)                              |
| **classification, confidence, `--external-only`**   | **HYD-07 `work/activity.rs` (this record)** |

HYD-07 reads all four and writes only its classification. It adds no store, no GitHub client and no CLI registration (CLI-13 owns that).

## 3. The attribution map

One row per remote object a Fleet operation created or changed, written when the operation's receipt records the remote response:

```
AttributionEntry {
  operation_id,            // REL-03 identity block
  session_id: Option,      // CLI-05; None for operations with no session
  outbox_id: Option,       // #9209 outbox, when the write went through it
  source_ref,              // e.g. github:developing-today/code
  remote_kind,             // issue_comment | review_comment | issue_body | pr_body | label_event | review
  remote_id,               // the ID the provider returned, opaque string
  remote_revision,         // provider revision or updated_at returned with the write
  content_digest,          // sha256 of the body Fleet sent, normalized line endings
  recorded_at,
}
```

Rules:

- **Only a provider response creates a row.** A planned write, an outbox intent or a timeout after send creates none. A write whose response was lost (DSP-09 G1/G2) leaves the object unmapped, so it reads `uncertain`, which is the honest answer until reconciliation finds the remote ID.
- **The key is `(source_ref, remote_kind, remote_id)`;** one row per revision Fleet produced. A later Fleet edit of the same object adds a row; it never rewrites the old one.
- **Login is never a key or a tiebreak.** `Principal.login` is recorded for audit only. The slice text's migration rule ("username never assigns session") is a hard rule here.
- **Content markers are hints, not keys.** A marker can raise `uncertain` confidence to `likely_fleet` in the report; it can never produce `fleet`, because a human can paste one and Fleet's own write can be edited after posting.

## 4. Classification

For an observed revision `r` of `(source_ref, remote_kind, remote_id)`:

1. Look up rows for the object. **None:**
   - actor is not one of the AUTH-05 principals Fleet writes as → `external`, confidence `high`;
   - actor is a Fleet principal → `uncertain` (`shared_actor_unmapped`).
2. **Rows exist, and one matches `r`'s revision and its content digest** → `fleet`, with that row's `operation_id`/`session_id`/`outbox_id`.
3. **Rows exist but none matches `r`:** the object was Fleet's, and someone changed it since → `uncertain` (`revision_moved`). A human edit of a Fleet comment is exactly what an agent must see.
4. **The map is stale** (its high-water mark is older than the HYD-06 cursor for the source) → `uncertain` (`map_stale`) for every unmatched lookup, never `external`, because a missing row may be one not written yet.
5. **Legacy objects** (created before the map existed, per a recorded cut-over revision) → `uncertain` (`legacy`).
6. **Tombstones** (HYD-06) keep the origin of the last revision seen and add `deleted`; a delete is never attributed to Fleet without an operation row for the delete itself.

Each result carries `{ origin, reason, confidence, operation_id?, session_id?, outbox_id? }`. `reason` is a closed enum (`mapped`, `unmapped_external_actor`, `shared_actor_unmapped`, `revision_moved`, `map_stale`, `legacy`, `marker_hint_only`) so a test can assert it.

### "Own session" exclusion

A session may exclude its own prior writes only by `session_id` on a `fleet` row. Two sessions sharing one login and one operation map is the slice's first failing oracle: session A's mapped comment is excluded for A, shown to B as `fleet` with A's `session_id`, and session B's **unmapped** comment under the same login is `uncertain` to both.

## 5. `--external-only` and the result envelope

- Default: include `external` and `uncertain`; report `counts { fleet, external, uncertain }` and `excluded_fleet` IDs so nothing is silently dropped.
- `--strict-external` (name reserved for CLI-13 to register or reject): only `external`. It is never the default and its help says it can hide human input.
- `--require-clean-reconciliation` (allocated to this slice): fails the command with `uncertain` outcome when any result has reason `map_stale`, instead of returning a partial answer as complete.
- Results use the shared `contracts.rs` envelope; a classification is never `verified_complete` while the map or cursor is stale.

## 6. Acceptance oracle

All in `fleet-queue` (`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`), each listed with `-- --list` first. Names use the `pmo_hyd_` prefix.

| test                                                      | asserts                                                                                                           |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `pmo_hyd_external_only_includes_uncertain_shared_actor`   | the slice's named test: one login, two sessions, one mapped comment; the unmapped one is `uncertain` and included |
| `pmo_hyd_attribution_login_never_assigns_session`         | same login, no row → never `fleet`, never a `session_id`                                                          |
| `pmo_hyd_attribution_same_body_human_comment_is_external` | a human comment byte-identical to a Fleet comment, different remote ID → `external`                               |
| `pmo_hyd_attribution_marker_only_is_hint`                 | pasted `agent-note` marker, no row → `uncertain` with `marker_hint_only`, never `fleet`                           |
| `pmo_hyd_attribution_edited_fleet_comment_is_uncertain`   | mapped comment edited by a human → `revision_moved`                                                               |
| `pmo_hyd_attribution_stale_map_is_never_external`         | map older than cursor → `map_stale`, and `--require-clean-reconciliation` fails                                   |
| `pmo_hyd_attribution_lost_response_stays_uncertain`       | write sent, response lost (DSP-09 B-cut) → no row → `uncertain` until reconcile records it                        |
| `pmo_hyd_attribution_legacy_is_uncertain`                 | revision before the recorded cut-over → `legacy`                                                                  |
| `pmo_hyd_attribution_tombstone_needs_delete_operation`    | delete with no delete row → not `fleet`                                                                           |
| `pmo_hyd_external_only_reports_counts_and_exclusions`     | counts and `excluded_fleet` sum to the input                                                                      |

The oracle is a reference model over fixtures: an in-memory map plus a revision list, compared field by field with the classifier's output. No live GitHub call is part of acceptance.

## 7. Compatibility and migration

- Additive. No existing command changes meaning; the flags are new and registered by CLI-13.
- Every object created before the map exists is `legacy` → `uncertain`. No backfill guesses from login or markers.
- Rollback disables the classifier and the flags; the map rows stay (they are receipts' children, not a cache).

## 8. Requirement trace (design level)

Outcome text quoted from `census/requirements-crosswalk.jsonl` of the 2026-11-13 design package; only the IDs this record answers are listed.

- C-R118 ("`fleet work changes <id> --since-start --external-only` should detect actionable activity since session start that did not originate from that session"): §4 own-session exclusion, §5.
- C-R108 (common flags include `--session-id`, `--since-start`, `--external-only`): §5; registration stays with CLI-13.
- O-R030 / P-R018 ("track … Fleet-operation session attribution; support since-start/checkpoint/timestamp and external-only"): §3, §4.
- P-R066 ("Activity acceptance: … session attribution, external-event detection, deduplication"): §6.
- H-R014 ("Preserve … session attribution"): §3 rules.
- H-R016 ("crash after remote mutation, duplicate webhook"): §3 first rule, §6 lost-response test.

The other allocated IDs (C-R005, C-R009, C-R107, C-R111, C-R112, C-R119–C-R122, C-R150, C-R151, H-R017, H-R024, H-R031, O-R022, O-R023, O-R028, O-R049, O-R050, P-R009, P-R012, P-R015, P-R016, P-R027, P-R039, P-R050, P-R051, P-R065) are hydration, cursor, CLI-inventory or end-to-end outcomes that HYD-04/HYD-06/HYD-08 and CLI-13 answer; this record consumes them and does not restate them.

## 9. Gates

Implementation of `activity.rs` waits for all of:

1. REL-03 (#9513) receipt identity block landed, with remote ID and revision recorded from the provider response;
2. CLI-05 (#9502) session identity landed;
3. HYD-06 (#9508) revision and tombstone store landed;
4. AUTH-05 (#9432) provenance schema landed, naming the principals Fleet writes as.

Until then this record is the contract those slices consume. Exposing `--external-only` before this classifier lands is out of scope; the slice text calls HYD-07 hard before that flag ships.

## 10. Non-goals

- No live GitHub reads, webhook routing or dispatch (#9203, #6533).
- No second classifier in pm-stack or the projector.
- No attribution from login, display name, App installation, marker text or body similarity.
- No change to how Fleet posts comments today.
