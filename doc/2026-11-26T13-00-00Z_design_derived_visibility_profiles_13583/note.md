# Derived-visibility profiles: policy record and Atlas search fixture

Refs #13583. Builds on the proposal in
[`2026-11-23T14-05-00Z_design_id_derived_visibility`](../2026-11-23T14-05-00Z_design_id_derived_visibility/2026-11-23T14-05-00Z_design_id_derived_visibility.md)
and the peer classes of
[`2026-11-17T05-15-00Z_design_id9141_peer_trust_classes`](../2026-11-17T05-15-00Z_design_id9141_peer_trust_classes.md).

This record states, for each reader and peer class, which derived facts it may
learn about rows it cannot read. It then pins the Atlas lexical search half of
the two-store fixture against that policy. The fixture lives at
[`pkgs/atlas-search/crates/atlas-core/tests/derived_visibility.rs`](../../pkgs/atlas-search/crates/atlas-core/tests/derived_visibility.rs).

## Observables

A reader can learn about hidden rows through five observables:

| Observable | Meaning |
|---|---|
| IDs | Which rows appear in a result |
| Count | How many rows a query or aggregate reports |
| Order | The relative rank of visible rows |
| Cursor | Whether a next page exists and where it starts |
| Topology | Graph shape, inventory summaries, replication digests |

A **strict** profile means every observable is identical between two stores
that differ only in rows above the reader's ceiling. A **looser** profile names
which observables may vary and why.

## Reader and peer classes

| Class | Who | Profile |
|---|---|---|
| Strict reader | An Atlas caller below the `secret-bearing` ceiling; an ID reader principal | Strict on IDs and count everywhere. Order and cursor are strict only where the table below says so. |
| `fully_trusted` peer (#9141) | Configured replication partner | May receive the full inventory summary in the native exchange greeting, including counts and topology of rows it later cannot open. |
| `send_only` peer | Outbound only | Learns nothing inbound; receives only what the sender's own ceiling allows. |
| `accept_signed_from` peer | Inbound gated data only | Strict reader for anything it queries; it receives no outbound inventory. |
| `unknown` peer | Anyone else | Denied; learns nothing beyond the refusal. |

## Surface decisions

| Surface | IDs and count | Order | Cursor | Status |
|---|---|---|---|---|
| Atlas FTS on a single monolithic index | Strict | **Looser**: `bm25(chunks_fts)` uses document frequencies from the whole index, hidden rows included | Looser, follows order | Pinned by the fixture |
| Atlas FTS on a partitioned index (one index per ceiling) | Strict | Strict | Strict | Pinned by the fixture as the strict profile |
| Atlas vector branch | Strict per tier | Strict per tier | Strict per tier | Recorded only. Tiers are physical (`shareable` → public, `personal` and `local-only` → internal, `secret-bearing` → secret), so `personal` and `local-only` share one tier and the scan applies no row-level filter inside it. That is an observation to check, not a confirmed exposure. |
| ID graph query | Strict | Strict | Strict | Covered by #13781 |
| Federation and export adapter | Not yet decided | | | Remaining work |
| Cached derivatives (summaries, embeddings, digests) | Not yet decided | | | Remaining work |

### The shared-statistics decision

The production FTS statement filters by ceiling in its `WHERE` clause but ranks
with `bm25(chunks_fts)` over the whole FTS table. Hidden rows therefore change
inverse document frequency, which can reorder visible rows, and when the pool
is truncated (`LIMIT ?`) it can change which visible rows make the cut.

The decision is to keep the monolithic index under the looser contract:
membership and count are strict, order and the contents of a truncated pool
are not. A deployment that needs a strict reader on order must index each
ceiling separately. The fixture proves both halves:

1. `hidden_rows_never_change_visible_membership_or_count`: for `shareable`,
   `personal` and `local-only` readers, the full visible set is the same in
   both stores, and no row above the ceiling is returned.
2. `shared_lexical_statistics_can_reorder_visible_rows`: with the hand-computed
   corpus, the hidden rows reverse the order of two visible rows, and at a pool
   of two the returned sets differ. If ranking ever becomes strict on the
   monolith, this test fails and this record should be updated.
3. `partitioned_index_gives_identical_order_at_the_same_visible_cut`: with one
   index per ceiling, order and truncated pools match exactly.
4. `full_trust_view_still_sees_hidden_rows`: a `secret-bearing` reader sees
   every row, so the fixture is not passing by hiding everything.

## Remaining work on #13583

- The federation and export adapter half of the two-store fixture.
- The cached-derivative case: whether a summary or digest built from hidden
  rows may be shown to a strict reader.
- A follow-up check of the shared internal vector tier noted above.
