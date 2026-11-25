# DST-06 — Cross-target fixture and canonical order hash

Issue: [#9456](https://github.com/developing-today/code/issues/9456)
(slice `DST-06` of [#9200](https://github.com/developing-today/code/issues/9200),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the order-hash slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the Rust types, the fixture format, the divergence
report and the hosted-conformance split a separately scoped
implementation issue can be filed against. `architecture.md`
already fixes the `fleet.selection-order/v1` DTO key-for-key and
the digest framing; `work/contracts.rs` already implements the
framing (`digest`, `canonical_json`) and reproduces the framing
vectors. What is missing is the thing that turns a snapshot into
that DTO without ever hashing a partial population, the fixture
every target runs, and the rule for what a mismatch means. This
record _decides_ those. It authorizes no code, schema, migration,
deployment or live acceptance; those remain separately gated
stages (§9).

Governing decisions consumed, not re-opened:

- **`architecture.md`, "Digest and serialization contract"** —
  the exact `fleet.selection-order/v1` keys; framing
  `ASCII(domain) || 0x00 || UTF8(JCS(dto))`, SHA-256, `sha256:` +
  64 lowercase hex; `ordered_items` is the **entire** eligible
  population in comparator order; a partial or truncated
  population "must not emit a conformant `order_hash`"; a matching
  hash "is a fingerprint, not authority, proof of complete inputs,
  or a claim grant"; the three-domain registry.
- **Component #9200, "Cross-target order and reconciliation" and
  V4** — DST-06 "tests framing vectors, included-input changes and
  cross-target hashes"; same total order only when targets share
  identities, taxonomy, policy/comparator versions, predicates,
  eligibility, as-of and snapshot; hosted targets must "calculate
  same order hash on same complete snapshot", and during version
  skew the caller sees `unsupported_version`, never silent
  coercion.
- **PRI-03** (record
  `doc/2026-11-23T15-34-52Z_design_pri03_peer_ordering_rank_rebalance/`,
  the package's `DST-ORDER-01`/`PRI-CMP-01`) — the one comparator
  tuple and its `comparator_ref`; `peer_rank` in each ordered item
  as a canonical signed decimal string, never null; the one
  golden parity fixture executed three ways (§3.5 there). DST-06 is
  that fixture's host-side and cross-target consumer.
- **`work/contracts.rs`** — `digest`, `canonical_json`,
  `DIGEST_DOMAIN_SELECTION_ORDER`, `is_canonical_decimal`,
  `cmp_canonical_decimal` and the framing-vector test. Read-only
  for this slice: DST-06 calls them and changes none of them.
- **SEL-02** (record
  `doc/2026-11-25T01-18-23Z_design_sel02_claim_revalidation/`) —
  "normal selection is always a broker pick"; a matching hash
  never claims anything.

## 1. Intent

Fleet's order exists in more than one place: the broker's claim
and list paths today, and later the hosted PM service (#9079) and
the hosted Fleet-tool equivalent (#9081). The order hash lets two
of them say "we computed the same order from the same inputs"
with one 71-character string, and lets a test prove it. The
failures this slice prevents:

1. A target hashes the first page it displayed and calls that the
   order.
2. Two targets disagree and nobody can tell whether the inputs
   differed (expected, re-sync) or the comparator did (a bug).
3. A target running an older comparator or schema produces a hash
   that is compared as if it were current.
4. Something that is not an ordering input (a title, a Project
   board position, a local row number) changes the hash, so equal
   orders look different.

## 2. Ownership and the hard/soft blocker split

| Concern                                                       | Owner                 |
| ------------------------------------------------------------- | --------------------- |
| DTO keys, framing, domain registry                            | `architecture.md` (fixed) |
| `digest`, `canonical_json`                                    | `work/contracts.rs` (existing; read-only here) |
| Comparator tuple, `comparator_ref`, broker parity fixture     | PRI-03                |
| Order DTO builder, completeness gate, fixture format, divergence report | **DST-06a (this record)** |
| Hosted targets running the fixture                            | **DST-06b (this record)**, inside #9079 / #9081 |

Every lane that has triaged #9456 flagged the same tension: the
slice header names #9079/#9081 as hard blockers while the
component paragraph calls them "consumers, not starter blockers",
and lanes took the conservative reading and parked the whole
slice. **Decision: split the slice.**

- **DST-06a — Rust builder, fixture and hash.** Needs nothing from
  either hosted target: the framing already exists, the DTO is
  fixed, and the order comes from PRI-03's comparator. Hard
  prerequisites: PRI-03 (comparator and `comparator_ref`) only.
  It owns `work/distribution.rs` (builder, report) and the fixture
  files (§5). Its first failing test is the slice's
  `pmo_distribution_host_order_hash`.
- **DST-06b — hosted conformance.** Each hosted target loads the
  DST-06a fixture in its own test surface and must produce the
  same canonical bytes and hash. This is where #9079/#9081 are
  hard: there is no hosted surface to test until they exist. It
  adds no Rust code; it is an acceptance item those issues carry.

This resolves the tension without weakening either reading: the
hosted targets block hosted conformance, and nothing else.

## 3. The order DTO in Rust

### 3.1 Two types, and only one can be hashed

`work/distribution.rs` gains:

```rust
/// A complete, validated fleet.selection-order/v1 object.
/// Only constructible by SelectionOrder::complete(..).
pub struct SelectionOrder { dto: serde_json::Value /* exact keys */ }

/// An order that is known to be partial: a page, a truncated
/// population, or a population whose completeness the producer
/// cannot state. Carries the same items for display, no hash.
pub struct PartialOrder { items: Vec<OrderedItem>, reason: PartialReason }

impl SelectionOrder {
    pub fn complete(inputs: OrderInputs, items: Vec<OrderedItem>)
        -> Result<SelectionOrder, OrderError>;
    pub fn order_hash(&self) -> String;          // digest(DIGEST_DOMAIN_SELECTION_ORDER, &self.dto)
    pub fn canonical_bytes(&self) -> String;     // canonical_json(&self.dto)
}
// PartialOrder deliberately has no order_hash().
```

The architecture's "a partial population must not emit a
conformant hash" becomes a type rule: there is no function that
hashes a `PartialOrder`, so a list page, a top-20 board view or a
paginated hosted response cannot produce one by accident. A
producer that can only page builds a `PartialOrder` with
`PartialReason::Paginated` and says so.

### 3.2 What `complete` checks

`SelectionOrder::complete` refuses (typed `OrderError`, no DTO)
unless all of these hold:

1. **Exact keys.** The DTO has exactly the architecture's
   top-level keys and `lane_context` keys, every nullable key
   present as `null`, no extras. Built from typed fields, so this
   is a construction property, checked again by a key-set test.
2. **Canonical scalars.** `eligible_count`, every `work_id`,
   `work_version`, `effective_urgency` and `peer_rank` pass
   `is_canonical_decimal` (signed where the profile permits
   negatives); no JSON numbers anywhere in the DTO, so values above
   2^53 never pass through a float.
3. **Count matches.** `eligible_count` equals `ordered_items.len()`
   and `complete_candidate_set` is `true` — set by `complete`, never
   by a caller.
4. **Unique identities.** No `work_id` appears twice.
5. **Sorted capabilities.** `lane_context.capabilities` is sorted
   and unique by the same UTF-16 ordering `canonical_json` uses for
   keys.
6. **In comparator order.** Each adjacent pair satisfies PRI-03's
   comparator over the item's own keys (`effective_urgency`, then
   `peer_rank`, then `created_at`, then `work_id` by
   `cmp_canonical_decimal`, so `9` sorts before `10`). The builder
   does not sort; it verifies. A producer that hands in a
   mis-ordered population has a comparator bug, and silently
   re-sorting would hide exactly the divergence the hash exists to
   expose.
7. **Known versions.** `schema_version` is
   `fleet.selection-order/v1` and `comparator_ref` is one this
   build knows. Anything else is `unsupported_version` (§4.3).

`created_at` and `as_of_utc` are carried as the authority's
canonical UTC strings, byte for byte as stored; the builder never
parses and re-serializes them (architecture: "preserved without
parse/reserialize changes").

### 3.3 Included and excluded inputs

Included — changing any one must change the hash:
`authority_id`, `queue_snapshot_ref`, `graph_revision`,
`policy_ref`, `taxonomy_ref`, `comparator_ref`, `owner_policy_ref`,
`as_of_utc`, every `lane_context` field, `eligible_count`, and for
each item `work_id`, `work_version`, `effective_urgency`,
`peer_rank`, `created_at`, plus the items' order.

Excluded by construction — they are not keys of the DTO, so no
change to them can move the hash: titles, bodies, labels, display
priority names, Project board `POSITION`, assignees, the store's
row number on a target whose identities are not the authority's,
excluded (ineligible) items, and anything wall-clock that is not
the pinned `as_of_utc`. A target that wants to show these shows
them beside the order, never inside it.

## 4. Comparing two targets

### 4.1 The divergence report

Two hashes that match are the end of the check. Two that differ
need a reason, and the hash alone cannot give one. When both
targets can export their canonical DTO bytes (the fixture always
can; a live target can on request), `distribution.rs` provides

```rust
pub fn compare(a: &SelectionOrder, b: &SelectionOrder) -> OrderComparison;

pub enum OrderComparison {
    Identical,                                   // same hash
    InputsDiffer { keys: Vec<&'static str> },    // e.g. ["graph_revision", "as_of_utc"]
    PopulationDiffers { only_in_a: Vec<String>, only_in_b: Vec<String> },
    OrderDiffers { first_index: usize, a: OrderedItem, b: OrderedItem },
    ItemKeysDiffer { work_id: String, keys: Vec<&'static str> },
}
```

The report is checked in that sequence and stops at the first
class that applies, because the classes mean different things:

- **InputsDiffer** — the two targets ordered different snapshots
  (different graph revision, as-of, policy). Expected during
  normal operation; the remedy is to re-run on a pinned snapshot,
  not to fix code. Order beneath it is not inspected: different
  inputs may legitimately produce different orders.
- **PopulationDiffers** — same pinned inputs, different eligible
  set. An eligibility disagreement (predicates or hard gates).
- **ItemKeysDiffer** — same population, an item's comparator keys
  differ (e.g. one target normalised `peer_rank` differently).
- **OrderDiffers** — same inputs, same items, same keys, different
  sequence: a comparator bug. This is the only class that is
  always a defect.

`compare` refuses to run (`NotComparable`) on two different
`schema_version`s or `comparator_ref`s; see §4.3.

### 4.2 Nothing is granted by a match

A matching hash is evidence the two targets ran the same tuple
version over the same pinned inputs. It is not a claim grant (SEL-02:
claims are broker picks or revalidated intents), not proof the
inputs were complete or current, and not a reason to skip any check
at commit. No code path reads an `order_hash` to decide
eligibility or ownership.

### 4.3 Version skew

A target that receives an order DTO with an unknown
`schema_version` or `comparator_ref` returns `unsupported_version`
and does not hash, compare or display it as an order (#9200 V4).
There is no "closest known version" fallback. A comparator change
is a new `comparator_ref` (PRI-03 §3.2), so an old target cannot
silently produce a hash under the old tuple and have it compared
with a new one.

## 5. The fixture

### 5.1 Location and format

One directory, consumed by every target:
`pkgs/fleet/crates/fleet-queue/tests/fixtures/selection-order/`.
Each case is one JSON file with:

```json
{
  "fixture_schema": "fleet.selection-order-fixture/v1",
  "case": "tie_nine_before_ten",
  "description": "two items equal on urgency, rank and created_at; ids 9 and 10",
  "inputs": { "...": "the OrderInputs fields" },
  "population": [ { "work_id": "10", "...": "..." }, { "work_id": "9", "...": "..." } ],
  "expected": {
    "ordered_work_ids": ["9", "10"],
    "canonical_json": "{...exact JCS bytes...}",
    "order_hash": "sha256:<64 hex>"
  }
}
```

`population` is deliberately **unsorted**: the target under test
runs its own comparator over it and builds its own DTO. The case
passes only if the target's canonical bytes equal
`expected.canonical_json` byte for byte and its hash equals
`expected.order_hash`. Comparing bytes as well as the hash means a
failure points at the first differing byte instead of at a digest.

The `expected` block is generated once by the Rust builder from
the comparator, reviewed, and committed. Regeneration is an
explicit, reviewed change (a dedicated test helper behind an
environment flag), never something a failing test does to itself.

### 5.2 Cases

Positive cases (each must reproduce exactly):

| Case                         | What it pins                                                |
| ---------------------------- | ----------------------------------------------------------- |
| `single_item`                | smallest complete population                                 |
| `empty_population`           | `eligible_count` `"0"`, `ordered_items` `[]` — complete and hashable |
| `tie_nine_before_ten`        | numeric identity tie order despite string encoding           |
| `null_rank_normalised`       | a stored NULL rank becomes PRI-03's normalized `"0"`, never null |
| `negative_rank_and_urgency`  | signed canonical decimals                                    |
| `beyond_u53`                 | a `work_version` above 2^53 survives unchanged               |
| `unicode_capabilities`       | capability strings outside ASCII sort by UTF-16 code units   |
| `nullable_fairness_ref`      | `fairness_state_ref: null` present, not omitted              |
| `pri03_parity`               | PRI-03's own parity snapshot, so the broker's three executions and every target share one case |

Perturbation cases: for every included input of §3.3, one case
that changes only that input from a base case and asserts the hash
changes. For every excluded input, one case that changes only that
field on the population records and asserts the hash does **not**
change (proving it never reached the DTO).

Refusal cases (the builder must return the named `OrderError` and
no hash): duplicate `work_id`, mis-ordered population handed in as
ordered, `eligible_count` mismatch, a JSON number where a decimal
string belongs, a non-canonical decimal (`"010"`, `"+1"`, `"1.0"`),
unknown `schema_version`, unknown `comparator_ref`, unsorted
capabilities, and an attempt to complete a population flagged as
paginated.

Framing: the existing `contracts/digest-vectors.json` vectors stay
where they are and keep being reproduced by `contracts.rs`; the
fixture does not duplicate them.

### 5.3 Exclusion reasons are compared, not hashed

The slice's acceptance line asks for byte-identical "canonical
IDs/reasons/hash". Reasons for _excluded_ items are not part of
`fleet.selection-order/v1` (the architecture fixes its keys and its
population is eligible items only), and adding a fourth digest
domain would amend the architecture's registry. **Decision:** each
fixture case may carry an `expected.exclusions` object
(`work_id` → sorted array of SEL-01 reason codes), compared as
canonical JSON bytes, not hashed. A target passes only if its
exclusions also match byte for byte. If a later decision wants a
hash over exclusions, it registers a new domain in
`architecture.md` first; this slice does not.

## 6. Where the hash is produced

- **Broker.** A read-only order export (owned by the command that
  lists the queue in full for one lane context, reconciled by
  CLI-02/CLI-13) builds a `SelectionOrder` over the complete
  eligible population at one snapshot and returns the DTO and hash.
  If the export is paginated it returns `PartialOrder` pages and no
  hash, plus a separate `order_hash` only once the producer has
  assembled the complete population at one snapshot.
- **PRI-03's `priority move` and before/after flags** (the census
  lists DST-06 as supporting): when the move's result reports the
  order around the moved item, it reports a `PartialOrder` window;
  it may report a full `order_hash` only if it computed the complete
  population. The move command does not need the hash to work.
- **Hosted targets (DST-06b).** Same rules; a target that cannot
  produce a complete population at one snapshot advertises no
  order-hash capability.

No producer stores the hash. It is recomputed from a snapshot on
demand; persisting it would create a second fact that can drift
from the snapshot it describes.

## 7. Persisted fields

**None.** The DTO is computed from data other slices own (REL-02
work versions, PRI-03 ranks, DEP-03 graph revision, PRI-02 effective
urgency). Fixtures are test files, not store schema. No CEN-04
§5 admission entry is needed.

## 8. Failure and recovery

| Scenario                                                  | Behaviour                                                     |
| --------------------------------------------------------- | ------------------------------------------------------------- |
| Producer has only a page                                  | `PartialOrder`, no hash                                        |
| Producer hands in an unsorted "ordered" population        | `OrderError::NotInComparatorOrder`; comparator bug surfaced    |
| Two targets differ on `graph_revision`                    | `InputsDiffer`; re-run on a pinned snapshot                    |
| Same inputs, different order                              | `OrderDiffers` with the first index: a defect                 |
| Target on an older comparator                             | `unsupported_version`; not compared                            |
| Title or board position changed                           | hash unchanged (excluded by construction)                      |
| A value above 2^53                                        | carried as a decimal string; hash stable across languages      |
| Fixture `expected` block edited by hand to make a test pass | caught in review; regeneration is an explicit flagged helper  |
| Rollback                                                  | remove the export's hash field; nothing persisted to revert    |

## 9. Gates and sequencing

- **DST-06a** starts after PRI-03 is implemented (comparator,
  `comparator_ref`, normalized `peer_rank`). It does not wait for
  #9079/#9081, PRI-02 or DEP-03: until PRI-02 and DEP-03 land, the
  broker's effective urgency is its stored `priority` (PRI-03 §3.3)
  and `graph_revision` is a fixed placeholder ref documented as
  such in the fixture inputs, and the cases are re-pinned when those
  land, through the explicit regeneration helper.
- **DST-06b** is an acceptance item of #9079 and #9081: each runs
  every DST-06a case in its own test surface. Until a target exists,
  there is nothing to run, and no parity is claimed.
- Live comparison between running targets, and any use of the hash
  in reconciliation (#9200 DST-05), are separately authorized.

## 10. Acceptance oracle, specified

Future tests, written failing first, listed by exact name with
`-- --list` before running (a zero-test filter is not acceptance),
under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.

1. `pmo_distribution_host_order_hash` — the slice's first failing
   test: every positive fixture case reproduces its canonical bytes
   and hash through `SelectionOrder::complete`.
2. `pmo_distribution_partial_order_has_no_hash` — compile-level and
   runtime: `PartialOrder` exposes no hash; completing a paginated
   population is refused.
3. `pmo_distribution_included_inputs_move_hash` — every inclusion
   perturbation changes the hash.
4. `pmo_distribution_excluded_inputs_do_not` — every exclusion
   perturbation leaves it unchanged.
5. `pmo_distribution_builder_refusals` — each refusal case returns
   its named error and no DTO.
6. `pmo_distribution_divergence_classes` — `compare` returns each
   class for a constructed pair, in the §4.1 precedence.
7. `pmo_distribution_unknown_version_not_compared` — unknown schema
   or comparator is `unsupported_version` for build and compare.
8. `pmo_distribution_exclusions_byte_identical` — exclusion reasons
   compared as canonical bytes.
9. `pmo_distribution_pri03_parity_case_shared` — the PRI-03 parity
   snapshot's hash equals across the pure comparator, the claim
   ordering and the list ordering.

DST-06b's hosted acceptance is each target's own test runner
loading the same fixture directory; its exact commands belong to
#9079/#9081.

## 11. Requirement trace (design-level)

The slice's allocated IDs (C-R005, C-R009, C-R107, C-R108, H-R004,
H-R005, H-R017, H-R024, H-R031, O-R023, P-R009) are carried by the
component trace in the issue body. At design level this record
addresses the slice acceptance line "same authority/profile/snapshot
yields byte-identical canonical IDs/reasons/hash" (§3, §5, tests
1, 8) and "version/target divergence fails" (§4, tests 6–7), and the
issue's negative coverage where it applies: stale input (§4.1
`InputsDiffer`), duplicate (§3.2 check 4), unknown version (§4.3),
no false verified result (§4.2, a match grants nothing). Parser
tests, deployment and live acceptance remain open evidence.

## 12. Non-goals

- Changing the DTO keys, the framing or the domain registry.
- The comparator itself (PRI-03) or effective urgency (PRI-02).
- Building either hosted target, or any live cross-target sync.
- Persisting order hashes or using them to grant claims.
- A hash over excluded items (needs an architecture amendment
  first, §5.3).
