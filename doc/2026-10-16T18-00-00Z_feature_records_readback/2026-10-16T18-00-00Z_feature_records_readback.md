# Records readback: a `records.read` capability

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [capability protocol module](../../pkgs/id/src/world_caps.rs), and the [RocCap record](../2026-10-16T16-00-00Z_feature_roc_cap/2026-10-16T16-00-00Z_feature_roc_cap.md).

## Intent

A world publishes a JSON `records` projection, but only sessions and
replication could read it: a program could not consume what another
participant or another instance published. That blocks shared canvases,
multi-terminal dashboards, and any app that reacts to peer state. Add a
`records.read` capability request.

## Decisions

- **Same page shape as the session protocol.** `records.read` takes `prefix`
  (optional string) and `after` (exclusive key cursor, `""` starts at the
  beginning) and returns `{"records":[{"key":...,"value":...}],
"next_after": <cursor or absent>}`. Follow `next_after` while it is
  non-empty. Keys are served in key order.
- **Byte-bounded pages.** One page is capped by `MAX_RECORDS_PAGE_BYTES`
  (768 KiB), the same bound the session transport enforces, so a program cannot
  ask for an oversized result. A single record larger than a page is a
  `BadArgs` refusal rather than a silent partial value.
- **Read-only, grant-gated.** `records.read` requires only the `join` scope,
  matching the existing public-records session query: any participant who may
  join may read the published projection. It is still refused unless the world
  grants it, and refusals are counted like every other capability.
- **No new storage.** The provider reads the actor's current published
  `Arc<Records>` snapshot; nothing is journaled, so reads never advance the
  sequence.
- **RocCap helpers.** `records_request(id, prefix, after)`, `records_page`,
  and `next_after` keep the JSON shape out of apps.

## Acceptance

- `records_read_returns_a_bounded_prefix_scoped_page` runs a program that
  publishes `{"count": n, "read": ...}`, asks for `records.read` with prefix
  `co`, and asserts the program sees its own `count` key. The same test asserts
  an ungranted request is refused and counted before the grant.
- `grant_names_must_exist` accepts `records.read`.
- Full library suite passes.

## Follow-ups

- A `since`/snapshot cursor so a program can poll for changes without
  re-reading a prefix.
- A checked-in example that syncs a canvas across instances through
  `records.read`.
