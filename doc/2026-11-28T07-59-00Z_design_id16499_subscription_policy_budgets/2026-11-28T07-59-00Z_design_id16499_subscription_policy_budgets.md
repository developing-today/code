# Discovery v4 — subscription policy budgets design (#16499)

Design record for issue #16499 ("id(phase-14): Discovery v4 —
subscription policy budgets design"). This is §9 item 1 of the v4
curation design (PR #16496, merged 2026-11-28T06:22:44Z, merge commit
`0d7612ff5d1c3f1dd4cecdcfedde73ac8f5a9491`; record
`doc/2026-11-28T06-10-00Z_design_id6290_open_discovery_v4/`, §3 and §8).
Parent outline #6290; core topic #6277. Predecessor semantics this
record builds on and does not re-decide: the v3 federation design
(record `doc/2026-11-27_design_id16051_federation_protocol_v3.md`,
merged in PR #16144) §3 mirror-propagation rules 1–5.

Status: design only. No code in this issue — this record names the
setting shape, the two defaults, and the tests that will prove the
semantics when the implementation slice is taken. Nothing here
authorizes a protocol change: budgets are local policy inputs to
`Registry::import`, in the same class as the existing policy string
(v4 §3 item 1).

**Grounding — verified 2026-11-28 (lane 0152-F, firsthand from
`main` at `a2c474014904c5b83b7eebeb108f2d28389e768a`).**

- `pkgs/id/crates/id-sync/src/registry.rs`: `Registry::import`
  (line 410) takes `source: &SignedRegistryList`,
  `source_key: Option<&[u8; 32]>`, `now: u64`, and an `accept`
  closure; it verifies first (`verify_from` when a key is given,
  else `verify`), then per entry, in order: expired →
  `delisted_here` (own tombstone stands) → `kept_own` (own entry at
  same or newer seq) → `declined` (`accept` says no) → include.
  Results are reported in `ImportReport` (`included`, `kept_own`,
  `declined`, `delisted_here`, `expired`, `source_tombstones`).
  There is no budget type, no per-source import history, and no
  `over_budget` report bucket anywhere in `id-sync` today.
- `pkgs/id/crates/id-sync/src/discovery_client.rs`:
  `ClientDiscoveryConfig` (line 37) is the client-config home —
  `subscribed_registries: Vec<[u8; 32]>` plus `relay_required`.
  `fresh_self_host()` is `Self::default()` (no subscriptions);
  `hosted_with_curated(key)` subscribes to exactly the curated
  registry. Budgets belong beside the subscription set, in this
  struct.
- An announcement verifies against its own `world_id`
  (`SignedRegistryList::verify` re-checks each entry against its
  world key, v3 §3). One world key therefore signs for exactly one
  world — see the rejected alternative in §2 for why the velocity
  key is the registry key, not the world key.

## 1. Setting shape

One new settings type, owned by the client config, consumed by
`Registry::import`:

```rust
// pkgs/id/crates/id-sync/src/discovery_client.rs

/// Local subscription budget for one subscribed registry.
/// `None` on a limit means that limit is not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubscriptionBudget {
    /// Max NEW entries imported from this registry inside one
    /// velocity window (see §3 for what counts as new).
    pub max_new_entries_per_window: Option<u32>,
    /// Velocity window length, in seconds. Must be > 0 whenever
    /// `max_new_entries_per_window` is `Some`.
    pub window_secs: u64,
    /// Max entries accepted from a single fetched list (§3).
    pub max_list_entries: Option<u32>,
}

impl SubscriptionBudget {
    /// No velocity limit, no list cap.
    pub const UNLIMITED: Self = Self {
        max_new_entries_per_window: None,
        window_secs: 0,
        max_list_entries: None,
    };

    /// The hosted curated-registry budget (§4).
    pub fn hosted_curated_default() -> Self { /* §4 values */ }
}
```

`ClientDiscoveryConfig` gains one field, keyed by the same registry
key as the subscription itself:

```rust
pub struct ClientDiscoveryConfig {
    pub subscribed_registries: Vec<[u8; 32]>,
    /// Budget per subscribed registry key. A subscribed key with no
    /// entry here is UNLIMITED (the §4 fresh-self-host rule).
    pub subscription_budgets: BTreeMap<[u8; 32], SubscriptionBudget>,
    pub relay_required: bool,
}
```

Config rules:

- `subscribe(key)` adds no budget entry (unlimited until the
  operator sets one); a new `subscribe_with_budget(key, budget)`
  sets both. `unsubscribe(key)` removes the budget entry with the
  subscription — a budget never outlives its subscription.
- A budget for a key the client does not subscribe to is inert:
  budgets are consulted only for the registry a list is imported
  from, and imports happen only for subscribed registries.
- Budgets are settings, never wire data. `SubscriptionBudget` is
  not part of `RegistryList`, is never encoded into a published
  list, and never appears in a served view — the v4 §7 locality
  rule and v3 §3 rule 5 (policy never crosses) apply to budgets
  exactly as they apply to the policy string.

Consumption by `registry.rs` — additive, no existing caller changes
meaning:

- `Registry::import` keeps its current signature and behaves
  exactly as today (unlimited). A new
  `Registry::import_with_budget(source, source_key, now, budget: Option<&SubscriptionBudget>, accept)`
  carries the budget logic; `import` delegates to it with `None`.
- `ImportReport` gains one bucket: `pub over_budget: Vec<[u8; 32]>`
  — worlds that would have been included but for the budget.
  `over_budget` is distinct from `declined` (a policy verdict) and
  from `expired`/`delisted_here`; it is a local report to the
  importing operator only and is never published or served.

## 2. What "per-key" means — decided

The v4 §3 phrase is "per-key entry velocity". The key is the
**subscribed registry key** — the key of the registry the list was
fetched from (the `source_key` / `list.registry_key` of the
import). The rejected alternative, per announcing world key, cannot
bound the flood this design exists to stop: an announcement
verifies against its own `world_id`, so one world key signs for
exactly one world, and a spammer minting ten thousand worlds mints
ten thousand keys with a per-key velocity of one each. The flood
surface is the source list, so the velocity is counted per source
registry. Per-list size cap is likewise per subscribed registry:
each subscribed registry's budget carries its own cap.

## 3. Import semantics under a budget

Order inside `import_with_budget` — verification and every existing
gate are unchanged and run first:

1. **Verify first, whole-list drop unchanged.** `verify_from` /
   `verify` run exactly as today (v3 §3 rule 1, restated in the
   #16499 scope). A list that fails verification imports nothing,
   budget or no budget; a budget never rescues, partially admits,
   or re-orders an unverifiable list.
2. **Existing per-entry gates, existing order.** Expired,
   `delisted_here`, `kept_own`, and `accept`-declined entries are
   classified exactly as today. None of them counts against either
   budget limit: the budget bounds *new inclusions*, and a
   `kept_own` re-announcement is not new.
3. **List-size cap.** Count inclusions from this fetched list in
   list order. Once `max_list_entries` inclusions are reached,
   every further otherwise-includable entry from this list is
   `over_budget`. The cap is per import of one fetched list; it
   needs no stored state.
4. **Velocity.** The registry keeps a local, never-published
   import history: per source registry key, the `now` timestamps
   of new inclusions. An otherwise-includable entry is
   `over_budget` when the count of that source's inclusions with
   timestamps in `[now - window_secs, now]` (this import's
   inclusions so far included) has reached
   `max_new_entries_per_window`. Included entries append their
   timestamp. The history is in-memory only, is not part of
   `RegistryList`, and starts empty on `Registry::resume` — a
   resumed registry's window restarts, which is safe because the
   cap in step 3 still bounds any single import.
5. **Include.** Anything not classified above is included exactly
   as today (`include`, own `included_at`, own `registry_seq`).

**Over-budget silence semantics (v3 §3 rule 2, v4 §3 item 1).** An
over-budget entry is *absent*, never a tombstone: import creates no
tombstone for it, the published list contains neither the entry nor
any mark against the world, and no other registry can observe that
a budget was applied. Over-budget is also not a verdict: the same
entry, still live in a later fetch of the source list, is
includable again once the window has slid or the cap/budget is
raised — nothing about the world, the source registry, or this
registry's policy has been decided by the earlier refusal.

**Interaction with the whole-list `verify_from` drop rule:**
unchanged, stated once — the drop rule is about *trust in the
list*; budgets are about *attention budget for a trusted list*.
The two compose in exactly one order (verify, then budget) and
neither weakens the other.

## 4. The two defaults

|                     | Fresh self-host (`fresh_self_host()`) | Hosted (`hosted_with_curated(curated)`) |
| ------------------- | ------------------------------------- | --------------------------------------- |
| Subscriptions       | none (unchanged)                      | the curated registry only (unchanged)   |
| Budget for a registry the operator adds | `SubscriptionBudget::UNLIMITED` — no budget entry | n/a |
| Budget for the curated registry | n/a | `hosted_curated_default()`: velocity **100 new entries per 86 400 s (24 h) window**, list cap **10 000 entries** |

Why unlimited for a fresh self-host: a self-host inherits nothing
from our hosted instance (v4 §1) — a finite default silently
applied to a registry the operator personally chose would be a
hidden policy that drops worlds without the operator ever having
set it. The operator who wants a bound sets one explicitly.

Why finite for the hosted curated subscription: the hosted
instance republishes to everyone, so its import budget is the
public backstop's flood bound. 100 new entries per day is far
above organic curated growth and far below a flood; a single
fetched list is capped at 10 000 entries so one import's work and
memory are bounded regardless of what the source publishes. Both
numbers are ordinary operator settings on our instance, not
protocol constants — another operator's hosted instance picks its
own.

## 5. Named tests that will prove it

Implementation-slice tests, in the existing sentence style of the
`registry.rs` test module, each with its oracle. The first seven
live in `pkgs/id/crates/id-sync/src/registry.rs` over
`Registry::import_with_budget` / `SignedRegistryList::verify_from`;
the last lives in `discovery_client.rs`.

1. `import_over_the_velocity_budget_leaves_the_excess_absent_without_a_tombstone`
   — Oracle: with velocity max 2, importing a verified 4-entry
   list includes exactly the first 2 in list order, reports the
   other 2 in `over_budget` only, and the published list contains
   neither those entries nor any tombstone for their worlds.
2. `import_over_the_list_size_cap_imports_only_the_first_cap_entries_in_list_order`
   — Oracle: with cap 1 and no velocity limit, a verified 3-entry
   list yields 1 inclusion and 2 `over_budget`, in list order.
3. `an_over_budget_entry_is_included_by_a_later_import_once_the_window_slides`
   — Oracle: an entry refused under test 1's budget is included
   when the same list is re-imported at `now + window_secs + 1`;
   refusal left no state that blocks it.
4. `an_unlimited_budget_imports_exactly_like_the_existing_import`
   — Oracle: `import_with_budget(.., None)` and
   `import_with_budget(.., Some(&SubscriptionBudget::UNLIMITED))`
   produce `ImportReport`s and published lists identical to
   `import` on the same inputs, with `over_budget` empty.
5. `a_list_that_fails_verify_from_is_dropped_whole_even_under_budget`
   — Oracle: a list signed by the wrong key imports nothing and
   returns the verification error, with a generous budget set —
   the budget path is never reached (v3 §3 rule 1 unchanged).
6. `kept_own_entries_do_not_consume_the_velocity_budget_or_the_cap`
   — Oracle: re-importing a list whose entries are all already
   held at the same or newer seq reports them all `kept_own`,
   consumes no budget, and a following new entry still imports
   under a velocity max of 1.
7. `budgets_and_import_history_never_appear_in_a_published_list`
   — Oracle: after a budgeted import, the published list
   round-trips through `to_bytes`/`from_bytes`/`verify_from`,
   encodes no budget or history data, and carries the mirror's own
   policy and seq only (v3 §3 rules 4–5).
8. `fresh_self_host_has_no_budgets_and_the_hosted_default_budgets_only_the_curated_registry`
   — Oracle (in `discovery_client.rs`): `fresh_self_host()` has an
   empty budget map; `hosted_with_curated(k)` budgets exactly `k`
   with the §4 hosted values; `subscribe` adds no budget entry;
   `unsubscribe` removes the budget with the subscription.

## 6. Out of scope (explicit, from the issue)

- Protocol throttles of any kind; cross-mirror budget sharing; any
  central gate (v4 §10 upheld).
- The attestation format (§9 item 2, #16500), local reputation
  (§9 item 3, #16501), and the hosted delist-transparency practice
  (§9 item 4, #16502) — sibling records, not this one.
- Any code in this issue: the implementation slice that adds
  `SubscriptionBudget`, `import_with_budget`, and the §5 tests is
  follow-on work filed from this record.

---

**Verified when:** this record is merged with the `doc/README.md`
index line, naming the §1 setting shape, the §4 two defaults, and
the §5 named tests over `registry.rs` import/verify — the #16499
Done check — and the outcome (record link + merge SHA) is posted
on #16499.
