# DST-07: Typed degraded, unsupported and stale priority states, and payload limits

Issue: [#9457](https://github.com/developing-today/code/issues/9457)
(slice `DST-07` of [#9200](https://github.com/developing-today/code/issues/9200),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **V3 design proposal; not an authorization to implement.**

This record elaborates the DST-07 slice of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the contract a separately scoped implementation issue
can be filed against: the typed states a distributed priority value
can be in when a consumer reads it, how each state is detected, what
each kind of consumer must do in each state, the payload limits on
the distribution wire with the reason for each number, the refusal
behaviour when a limit is exceeded, and the recovery intent kept for
an authorized input that could not be applied. It authorizes no code,
schema, migration, deployment or live acceptance; those remain
separately gated stages (section 12).

The goal, from the slice text (`issues/bodies/DST-07.md:10-12`,
component `components/9200-priority-distribution.md:148`): "Typed
degraded/unsupported/stale behavior and payload limits." The oracle
(`components/9200-priority-distribution.md:148`): "Stale/unsupported
never reports fresh/applied; oversize/unknown version is typed and
retains authorized recovery intent."

The decision in one line: **every distributed priority observation a
consumer reads is classified into exactly one of eight typed states
by a pure function of its bytes and the consumer's pinned context;
only `fresh` may be used as an input and nothing but a broker
revalidation may act on it; every other state either shows the value
behind a marker or shows a marker with no value; no consumer ever
substitutes a default, a zero, a snapped value or a nearest version;
and an over-limit or unsupported input is refused whole, never
truncated, with a recovery intent kept only when the input came from
an authenticated, authorized source.**

All paths below are relative to the repository root unless they start
with `components/`, `decisions/`, `census/`, `sources/`, `issues/` or
`architecture.md`, which are relative to the design package directory.
Line numbers are at checkout HEAD `23f1f34d` (2026-11-25T20:08Z)
unless marked as the package's pinned baseline `a038f084d`.

## 0. Governing decisions consumed, not re-opened

- **Component #9200, reason-code registry note**
  (`components/9200-priority-distribution.md:77`): "Unknown code
  under a supported schema returns typed
  `unsupported_reason_code`/`rejected` with no canonical mutation;
  unknown major schema/policy returns `unsupported_version` and
  read-only/quarantine. Never alias unknown codes."
- **Component #9200, snapshot identity** (`:92`): every projection
  snapshot identifies desired work version, effective-policy
  version, as-of time, graph/eligibility snapshot revision,
  comparator version, order hash and watermark; "Snapshot is **not**
  proof the external write was applied"; use the shared eight
  `Result.outcome` values and "don't invent a second receipt state
  machine."
- **Component #9200, reconciliation** (`:133`): "Never silently drop
  oversized/invalid payload: return typed `payload_too_large`,
  `invalid_schema`, or `unsupported_version` and retain recoverable
  intent where authorized."
- **Component #9200, replay/version semantics** (`:135`): "Unknown
  major schema or policy version is quarantined/read-only; no 'best
  effort' numeric interpretation. Known additive fields may be
  ignored only if marked optional and negotiated. Same revision with
  divergent payload is terminal conflict; missing revision is
  detectable gap."
- **Component #9200, consumers** (`:53`): "Consumers show
  stale/conflict explicitly."
- **Component #9200 V4** (`:152`): hosted targets "return same typed
  conflict/unsupported/stale statuses"; "During version skew, older
  target remains read-only for unsupported major versions and caller
  sees `unsupported_version`; no silent coercion or divergent order."
- **`architecture.md`, shared interface** (`:131-132`, `:143-148`,
  `:153-159`): parse the complete invocation before side effects;
  snake_case wire keys; integers that may exceed the JavaScript safe
  range cross JSON as canonical decimal strings; the eight outcomes;
  "Capability discovery advertises only the commands and guarantees
  that target actually supports."
- **`architecture.md`, ordering** (`:274-279`): a projection cannot
  grant a lease; "A changed input requires recomputation/reselection
  or a typed stale result."
- **`architecture.md`, digests** (`:245-248`, `:254`): a partial
  population "must not emit a conformant `order_hash`"; "Artifact
  digests cover the exact stored artifact bytes."
- **Component #9201, rules** (`components/9201-pm-projection-mobile-control.md:17`,
  `:28`, `:65`): "A stale/offline board is read-only"; source
  freshness, missing facets and stale feed "must remain visible";
  "An absent dimension is absent, not zero, not guessed from a label,
  and not synthesized from another score."
- **FDR-PMO-001** (`decisions/FDR-PMO-001-canonical-broker-command-seam.md:46`):
  "Read projections may be stale but cannot grant a claim."
- **FDR-PMO-002** (`decisions/FDR-PMO-002-priority-profiles-ordering-provenance.md:60`,
  `:72`): typed outcomes for unsupported profile/version, "no hidden
  rounding"; "unknown major version is typed unsupported/quarantined,
  not coerced."
- **FDR-PMO-004** (`decisions/FDR-PMO-004-hydration-cursors-attribution.md:48`):
  "Request/time/byte/page caps are explicit capability limits, not
  truncation hidden from the caller." Adopted here for distribution
  payloads.
- **DST-01** (#9451, closed; `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs`):
  the shared envelope, `ContractError`, `Outcome`, `ReasonCode`,
  `canonical_json`, `digest`, `is_canonical_decimal`. Consumed and
  extended additively (section 5.1), never changed in meaning.
- **DST-06 record**
  (`doc/2026-11-25T01-19-37Z_design_dst06_order_hash_fixture/2026-11-25T01-19-37Z_design_dst06_order_hash_fixture.md`):
  `SelectionOrder` versus `PartialOrder` (`:111-137`), unknown
  `schema_version` or `comparator_ref` is `unsupported_version` and
  is not hashed, compared or displayed as an order (`:243-251`), the
  fixture directory convention (`:255-258`), and "No producer stores
  the hash" (`:357-359`). This record reuses those names and adds no
  second order type.
- **SEL-02 record**
  (`doc/2026-11-25T01-18-23Z_design_sel02_claim_revalidation/2026-11-25T01-18-23Z_design_sel02_claim_revalidation.md:331-338`):
  "Stale is not an error to retry blindly"; the broker's claim
  transaction is the only place a choice becomes a lease.

## 1. Intent

A priority value leaves the broker and is read in many places: the
PM aggregator's queue panels, the mobile projection (#9201), the
DST-05 reconciler that projects to Project #4 and labels, hosted
targets (#9079; #9081 is closed), and agents that choose what to
propose for a claim. Every one of those readers can be holding a
value that is old, computed from incomplete inputs, written in a
version it does not understand, too large to have arrived whole, or
simply missing. The failures this slice prevents:

1. A stale or partial value is displayed or relayed as if it were
   current, and a reader acts on it.
2. A reader that does not understand a schema, profile, taxonomy,
   policy or comparator version interprets the numbers anyway
   ("best effort"), so two readers disagree silently.
3. A missing value is filled in with something plausible: `0`, the
   profile default, the nearest scale value, the last value seen, or
   a value guessed from a label.
4. An oversized payload is cut to fit, so a population, an evidence
   list or a reference changes meaning without anyone being told.
5. An authorized producer's input is refused for size or version
   and then lost, so it cannot be recovered after the reader is
   upgraded.
6. A relay that wrote nothing, or wrote from a stale input, reports
   the write as accepted or verified.

## 2. What exists today vs what this record proposes

### 2.1 Exists at HEAD (source facts, not deployment proof)

**Shared envelope (DST-01).** `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs`:
schema identifiers `fleet.command/v1`, `fleet.result/v1` (`:43-45`);
the three-entry digest-domain registry (`:51-63`); `ContractError`
with `invalid_schema`, `unsupported_version`,
`unsupported_reason_code` and others, all non-mutating (`:65-142`);
`is_canonical_decimal` with no length bound (`:194-207`); the
fifteen-code `ReasonCode` registry including `stale_snapshot`
(`:320-408`); the eight-value `Outcome` (`:508-567`); `Command`,
whose `from_json` rejects a top-level `authorized` key and otherwise
deserializes with serde defaults (`:662-682`); `Command::validate`
returns `UnsupportedVersion` for any `schema_version` other than
`fleet.command/v1` (`:693-698`); `Effect` with separate
`requested`/`observed`/`verified` (`:825-840`); `CommandResult`
(`:860-916`); `canonical_json` (`:1031-1040`). No struct in
`work/*.rs` uses `#[serde(deny_unknown_fields)]`, so unknown
non-`authorized` keys are ignored today. There is no
`payload_too_large` code and no size check anywhere in `contracts.rs`.

**Priority model (PRI-01, PRI-02).** `work/priority.rs`:
`PriorityError::UnsupportedVersion`, `OffProfile` ("Never snapped"),
`ProfileMismatch`, `StaleSnapshot` with wire code `stale_snapshot`
(`:130-216`); `PriorityProfile.default_name` documented as "Reads
never fabricate it: an untriaged stored value stays `None`"
(`:300-304`); `PriorityValue {profile_id, profile_version, value}`
(`:309-318`); `PinnedInputs` with `as_of: i64` (`:2131-2146`);
`PriorityExplanation` refusing an unknown schema version and reading
missing optional dimensions as `None` "never a fabricated value"
(`:2148-2190`).

**Other freshness and degraded names in the same module tree.**
- `work/degraded.rs` is the #8581 verb-routing failure-mode contract:
  `FailureLeg` (`:9-19`), `Mode` with labels `normal`,
  `gh-write-degraded`, `gh-read-degraded`, `db-degraded` (`:21-58`),
  and `ModeLine {verb, mode, route}` (`:60-80`). It describes **how
  a Fleet verb was served** when a dependency failed. It has no
  label parser.
- `work/completion.rs` (REL-09) owns `Freshness {Current, Limited}`
  for completion receipts against an activity watermark
  (`:126-142`).

Neither describes the state of a distributed priority value as seen
by its reader. DST-07 therefore introduces new names that clash with
neither (section 5.3).

**Distribution module.** `pkgs/fleet/crates/fleet-queue/src/work/distribution.rs`
does not exist; `work/mod.rs:7-18` lists no `distribution` module.
The DST-06 record proposes its first contents (section 0).

**Existing size limits in the crate and its neighbours.**
- `pkgs/fleet/crates/fleet-queue/src/validate.rs:301-306`: task
  payload design target 4,096 bytes and hard ceiling 16,384 bytes
  ("evidence and pointers, not full diffs/logs", `:1575-1584`);
  metadata object 8,192 bytes, 64 keys, 64-byte keys, 2,048-byte
  values.
- `pkgs/fleet/crates/fleet-queue/src/uds_http.rs:32-37`: broker
  request head 64 KiB, body 16 MiB on any transport, "rejected
  before the body is read, never clamped, which would dispatch a
  silently truncated request"; `tcp_http.rs:132-150` answers 413.
- `pm-stack/projector/src/webhook.ts:7-29`: `readBoundedBody`
  cancels the stream and throws `body_limit` past the limit;
  `pm-stack/projector/src/server.ts:327-330` reads GitHub webhook
  bodies at 10,000,000 bytes and answers 413 `body_too_large`;
  `:97` `MAX_WRITE_BODY_BYTES = 10_000_000`; `:103-107` string-field
  caps (`MAX_CONSUMER_CHARS = 256` and others); `:79-84` list caps
  (`MAX_ITEMS_LIMIT = 1000`).

**Existing staleness handling in consumers.**
- `pm-stack/aggregator/src/feed.ts:472-481`: `FEED_STALE_MS = 5 *
  60_000` because "The collector is scheduled every five minutes";
  `GENERATED_AT_FUTURE_SKEW_MS = 60_000`.
- `pm-stack/aggregator/src/queue.ts:20-26`, `:146-164`: the
  read-only queue collector; a snapshot is stale when the broker read
  failed or its age exceeds `QUEUE_STALE_MS` (5 minutes); on failure
  it "hold[s] the last good read"; "Panels must show stale data
  behind a warning, never as silently current."
- `pm-stack/aggregator/src/queue-view.ts:49-71`: `queueInputsStale`
  returns stale when health is missing, a facet is not `live`, or any
  input is older than `FEED_STALE_MS`.
- **Invented value at a consumer:** `pm-stack/aggregator/src/queue.ts:195`
  projects a broker task row with `priority: row.priority ?? 0`. A
  missing or null priority becomes `0`, which on this scale is sooner
  than critical (`10`). The broker itself orders `NULL` priority last
  (`pkgs/fleet/crates/fleet-queue/src/turso_queue/lifecycle.rs:491`,
  `turso_queue/admin.rs:1899`). This is exactly the class of defect
  section 3.5 forbids; DST-07 does not own that file (gap G5).

**Producer-side normalization at HEAD.** `pm-stack/dispatch/src/validate.ts:15-19`
now carries `expedite = 15` and `next-up = 25` losslessly (DST-02,
#9452 closed); other in-range off-scale integers still snap to the
nearest standard value (`:65-72`), and an absent request priority
resolves to `DEFAULT_PRIORITY = 40` (`:28`, `:42`). Both are
enqueue-time producer behaviours owned by DST-02/PRI-01, not
consumer reads; DST-07 forbids applying either to a distributed
observation (section 3.5).

**Existing quarantine precedent.** A task whose stored payload fails
revalidation at claim is moved to `blocked` with an event
`via: claim_payload_quarantine` and rechecked later
(`pkgs/fleet/crates/fleet-queue/src/quarantine_transition.sql:1-14`,
bounded per call by `MAX_CLAIM_QUARANTINES_PER_CALL = 512`,
`turso_queue/lifecycle.rs:655-664`). DST-07 does not change it; it
is cited as the crate's existing "fail closed, keep, recheck"
pattern.

### 2.2 Absent at HEAD

- No typed state for a distributed priority value as read by a
  consumer; no classifier; no consumer-action rule.
- No observation header that carries producer health or route mode
  alongside a priority value.
- No `payload_too_large` code in the shared contract; no limits on
  the size of a `fleet.command/v1` or `fleet.result/v1` object, on
  reference lengths, on canonical decimal digit counts, or on JSON
  depth.
- No recovery intent type.
- No test named `pmo_distribution_unknown_version_degrades` (the
  2026-11-13 triage comment on #9457 recorded the done command
  matching 0 tests; not re-run here).

### 2.3 Proposed by this record

A pure classifier and its types in `work/distribution.rs`, an
additive `payload_too_large` error and a versioned limits table in
`work/contracts.rs`, and a fixture corpus. No new table, column,
queue, scheduler, service, resolver, executable, GitHub client or
outbox producer (`issues/bodies/DST-07.md:27`). Nothing here is
persisted by DST-07; the recovery intent is a value its caller
persists through the owner named in section 4.5.

## 2a. What "a distributed priority value" means here

DST-07 classifies **observations**: one work item's priority
dimensions (PRI-02's intrinsic, importance, effective and factors)
as read by a consumer from a producer, plus the header that says
where and when they came from. It also classifies **order
payloads** (DST-06's `fleet.selection-order/v1` DTO or its pages)
and, on the producer side, **commands** that carry a priority intent.

The full observation DTO belongs to `DST-SNAPSHOT-01`, which the
packet normalizes to DST-05 (`issues/bodies/DST-07.md:36`). DST-05
has no record yet. DST-07 therefore fixes only the **header fields
it needs to classify**; whichever DTO DST-05 adopts must carry them
under these names, or DST-05 records the mapping. This is a
dependency, not a takeover (gap G2).

Required header (all keys present; nullable keys present as `null`):

| Key              | Type                                 | Used to detect              |
| ---------------- | ------------------------------------ | --------------------------- |
| `schema_version` | string                               | unsupported                 |
| `work`           | `{authority_id, work_id}` (`WorkRef`) | invalid, conflicted         |
| `work_version`   | canonical decimal string             | stale (by version), conflicted |
| `as_of_utc`      | authority canonical UTC string       | stale (by age), invalid (future) |
| `snapshot_ref`   | string                               | carried, compared by DST-05 |
| `profile_ref`    | string `profile_id@version`          | unsupported                 |
| `taxonomy_ref`   | string                               | unsupported                 |
| `policy_ref`     | string                               | unsupported                 |
| `comparator_ref` | string or `null`                     | unsupported (when present)  |
| `producer`       | `ProducerHealth` (section 5.2)       | degraded, unsupported       |
| `value`          | priority dimensions or `null`        | displayed, never invented   |

`value: null` means the producer states the item is untriaged. That
is a **fresh, legitimate** observation of an absent dimension, not an
unknown one (component #9201 `:65`; priority.rs `:300-304`).

## 3. The consumer states

### 3.1 The eight states

| State         | Meaning                                                                                   |
| ------------- | ----------------------------------------------------------------------------------------- |
| `fresh`       | Supported versions, complete inputs, normal producer route, within the age budget, not superseded, within limits. |
| `stale`       | Parsed and supported, but older than the consumer's age budget, or superseded by a newer `work_version` the consumer has already seen, or the latest refresh failed and this is the retained last good read. |
| `degraded`    | Parsed, supported and within budget, but computed from incomplete or fallback inputs: a partial population, an input facet marked `fallback` or `unavailable`, or a producer route other than `normal`. |
| `unsupported` | Parsed, but a `schema_version`, `profile_ref`, `taxonomy_ref`, `policy_ref`, `comparator_ref`, route-mode label or reason code is not one this build knows. |
| `too_large`   | Exceeds a limit of section 4. Detected before or instead of interpretation.               |
| `invalid`     | Not a well-formed object of its schema: unparseable JSON, a missing required key, an unknown key in a strict type, a non-canonical decimal, an invalid `work` pair, an `as_of_utc` that does not parse or lies in the future beyond tolerance. |
| `conflicted`  | Same `work` and same `work_version` as an observation the consumer already holds, but different canonical bytes. |
| `unknown`     | The consumer has no observation at all: never received, or the source is unreachable and nothing was retained. |

The test name `pmo_distribution_unknown_version_degrades` (the slice's
first failing test) means "an unknown version makes the consumer fall
back to read-only behaviour". Its asserted state is **`unsupported`**,
not `degraded`: component #9200 `:77` and `:135` name the outcome
`unsupported_version` and read-only/quarantine. The test keeps its
issued name (gap G6).

### 3.2 Precedence

When more than one condition holds, the reported `state` is the first
in this list, and every detected cause is still listed in `causes`
(section 5.2), most severe first:

`too_large` > `invalid` > `unsupported` > `conflicted` > `unknown` >
`stale` > `degraded` > `fresh`.

Why this order: size is checked on raw bytes before parsing, so a
too-large payload is never interpreted; a shape that did not parse
cannot be version-checked; an unsupported version's fields cannot be
trusted enough to compute age or compare revisions; a conflict at one
revision makes age irrelevant; `unknown` has no bytes. `stale` ranks
above `degraded` because staleness alone already blocks every use but
display, and the label should name the condition a re-read fixes.

### 3.3 Detection

Detection runs in this order, and stops interpreting at the first
fatal step (steps 1 to 4); steps 5 to 7 all run so every cause is
recorded.

1. **Size, before parse.** The raw byte length is checked against the
   raw bound (section 4.2) without parsing. Over the bound:
   `too_large`, cause `oversize`, no interpretation.
2. **Shape.** Parse with duplicate-key rejection and the depth bound;
   check required keys, canonical decimals (with the digit bound),
   reference lengths, the `work` pair (`WorkRef::validate`,
   `contracts.rs:251`), the canonical-bytes size bound, and
   `as_of_utc` parsing. Failure: `invalid` (or `too_large` for a
   bound).
3. **Version support.** `schema_version` against the set this build
   supports; `profile_ref`, `taxonomy_ref`, `policy_ref`,
   `comparator_ref` against the consumer's supported set; the
   `producer.route_mode` label against the #8581 labels; any reason
   code against `ReasonCode::parse` (`contracts.rs:401-408`). Any
   miss: `unsupported`, cause naming the field and the value seen.
   There is no "closest known version" (DST-06 record `:248`).
4. **Revision consistency.** If the consumer supplies a prior
   observation for the same `work` and `work_version`, compare
   canonical bytes (`canonical_json`, `contracts.rs:1036`). Equal: a
   duplicate, classified as the prior was, no conflict. Different:
   `conflicted`. No digest is computed for this, so no new digest
   domain is needed.
5. **Age.** `age_s = now_s - as_of_s`, both in whole seconds of UTC
   (the broker writes second-resolution `YYYY-MM-DDTHH:MM:SSZ`,
   `turso_queue/core.rs:56-64`). `age_s > max_age_s`: cause
   `age_exceeded`. `as_of_s - now_s > max_future_skew_s`: `invalid`
   with cause `future_as_of` (a producer clock ahead of the
   consumer's by more than the tolerance is not trusted).
6. **Supersession.** If the consumer has seen a strictly newer
   `work_version` for the same `work` (from any source, by
   `cmp_canonical_decimal`, `contracts.rs:209`), cause
   `newer_version_seen`; the observation is `stale` regardless of
   age.
7. **Producer health.** `producer.route_mode != "normal"`: cause
   `producer_route`. Any `producer.inputs` entry not `live`: cause
   `input_unavailable` or `input_fallback`. `producer.complete ==
   false` or a DST-06 `PartialOrder`: cause `partial_population`.
   Any of these: `degraded` (unless something above won).

A failed refresh with a retained last good observation is classified
from that observation's own bytes plus cause `source_unavailable`, so
it is `stale` at best and never `fresh` (this matches
`pm-stack/aggregator/src/queue.ts:150-156`, where the last good read
is kept but marked stale).

**Age budgets (proposed defaults).** Budgets are part of the
consumer's context and are echoed in every classification, so a
classification is reproducible from its inputs.

| Consumer class | `max_age_s` | `max_future_skew_s` | Reason |
| -------------- | ----------- | ------------------- | ------ |
| `display`      | 300         | 60                  | Same as `FEED_STALE_MS` / `QUEUE_STALE_MS` (`feed.ts:478`, `queue.ts:26`), set because the collector runs every five minutes; skew matches `GENERATED_AT_FUTURE_SKEW_MS` (`feed.ts:481`). |
| `relay`        | 300         | 60                  | A relay re-reads before applying anyway (DST-05 compare-and-apply); the budget only decides whether it may skip the re-read for planning. |
| `decision`     | 300         | 60                  | A decision never relies on age: the broker revalidates at commit (SEL-02). The budget only labels the input. |

These are defaults, not policy; a consumer may pass a smaller budget,
and the chosen budget is recorded in the classification.

### 3.4 What a consumer must do

Three consumer classes:

- **decision**: chooses what to propose to the broker (an agent's
  candidate pick, a Do Now route, an exploration draw), or uses the
  value as a precondition. The broker's claim or command transaction
  revalidates; the observation never grants anything (FDR-PMO-001
  `:46`, SEL-02).
- **relay**: projects the value onward to another target (DST-05 to
  Project #4 and labels; hosted targets).
- **display**: shows the value to a person (aggregator, mobile,
  hosted dashboard).

| State         | decision     | relay                         | display                                         |
| ------------- | ------------ | ----------------------------- | ----------------------------------------------- |
| `fresh`       | `use_as_input` | `use_as_input` (apply still compares versions) | `show`                       |
| `stale`       | `fail_closed`  | `fail_closed` (re-read first) | `show_with_marker`: value, `as_of_utc`, age, "stale" |
| `degraded`    | `fail_closed`  | `fail_closed`                 | `show_with_marker`: value plus the degraded inputs named |
| `unsupported` | `fail_closed`  | `fail_closed`, keep read-only | `show_marker_only`: "unsupported version X", no value |
| `too_large`   | `fail_closed`  | `fail_closed`                 | `show_marker_only`: size and limit, no value    |
| `invalid`     | `fail_closed`  | `fail_closed`                 | `show_marker_only`: "invalid", no value         |
| `conflicted`  | `fail_closed`  | `fail_closed`, hand to DST-05 reconcile | `show_marker_only`: "conflict at version N" |
| `unknown`     | `fail_closed`  | `fail_closed`                 | `show_marker_only`: "unknown"                   |

Rules that hold in every cell:

- `fail_closed` means: do not use the value for the decision or the
  write; re-read from the authority or stop; report the state. It
  never means "use a fallback value".
- `show_with_marker` keeps the value **and** a visible marker in the
  same rendered element; a value shown without its marker is a
  defect. The marker names the state and its causes.
- `show_marker_only` shows no number, no band name and no order
  position for the item.
- A `degraded` value is not made acceptable for a decision by this
  slice. A decision owner that wants to act in a #8581 degraded route
  (for example the #8583 replica path) must say so in its own slice
  and report its `ModeLine`; DST-07 grants no such exception.
- An untriaged item (`fresh`, `value: null`) is shown as untriaged.
  It is not `unknown`, and it is not given a number.

### 3.5 Never invent a value

No consumer, in any state, may substitute for a missing, stale,
unsupported or refused value:

- a default (`PriorityProfile.default_name`, dispatch's
  `DEFAULT_PRIORITY = 40`);
- `0` or any sentinel number (the `?? 0` at
  `pm-stack/aggregator/src/queue.ts:195` is the existing counterexample);
- the nearest profile value (snapping), or a value mapped through any
  profile mapping other than an explicit named-intent
  `ProfileMapping` (`priority.rs`, PRI-01);
- a value read from a mutable mirror label or Project field;
- the last value seen, presented as current (it may be shown only as
  `stale` with its own `as_of_utc`);
- a value from a "closest known" schema, profile, taxonomy, policy or
  comparator version.

The classifier enforces this structurally: `Classification` carries
the observation's value only when `state` permits display, and
carries no field into which a substituted value could be written
(section 5.2).

## 4. Payload limits

### 4.1 The limits table (`fleet.payload-limits/v1`)

Limits are a versioned table so a refusal can name which table
refused it and a producer can learn a consumer's limits through
capability discovery (`architecture.md:158-159`). Changing any number
is a new table reference, not an edit.

| Limit                     | Value       | Unit   | Applies to                                                         | Reason |
| ------------------------- | ----------- | ------ | ------------------------------------------------------------------ | ------ |
| `max_command_bytes`       | 16,384      | bytes  | canonical JSON of one `fleet.command/v1`                           | Same as the queue's task payload hard ceiling (`validate.rs:302`), whose stated rule is "evidence and pointers, not full diffs/logs" (`:1578`). A priority command carries references; raw comment text stays untrusted data kept by reference or hash (component #9200 `:90`). |
| `max_result_bytes`        | 16,384      | bytes  | canonical JSON of one `fleet.result/v1`                            | A priority result names a handful of field effects; one constant for both envelopes keeps producer and consumer checks symmetric. |
| `max_observation_bytes`   | 8,192       | bytes  | canonical JSON of one per-item observation                         | Same as the queue metadata object cap (`validate.rs:303`). A header at the reference caps below is roughly 2.5 KiB, leaving roughly 5.5 KiB for the value block and its evidence references. |
| `max_page_items`          | 1,000       | items  | one page of an order or observation list                            | Same as the projector's `MAX_ITEMS_LIMIT` (`pm-stack/projector/src/server.ts:80`), the existing pagination convention. A page is always a DST-06 `PartialOrder`. |
| `max_snapshot_items`      | 10,000      | items  | `ordered_items` of one complete `fleet.selection-order/v1` on the wire | At the decimal and timestamp caps an ordered item is under about 300 canonical bytes, so 10,000 items stay under `max_snapshot_bytes`. Current live queue sizes were not measured for this record (unverified); see Q3. |
| `max_snapshot_bytes`      | 4,194,304   | bytes  | canonical JSON of one complete order DTO on the wire               | Under the projector's 10,000,000-byte write cap (`server.ts:97`) and the broker's 16 MiB body cap (`uds_http.rs:37`) with room for the envelope and `lane_context`. |
| `max_ref_bytes`           | 256         | bytes  | each identifier or reference string: `operation_id`, `idempotency_key`, `actor_ref`, `source_ref`, `source_event_ref`, `delivery_id`, `session_ref`, `profile_ref`, `taxonomy_ref`, `policy_ref`, `comparator_ref`, `owner_policy_ref`, each evidence reference, `authority_id` | Same number as the projector's caps on compared identifier fields (`server.ts:105-107`). Those count JavaScript string length; this counts UTF-8 bytes, which is stricter for non-ASCII. A value longer than this is a payload, not a reference. |
| `max_composite_ref_bytes` | 1,024       | bytes  | `snapshot_ref`, `observed_snapshot_ref`, `queue_snapshot_ref`      | These concatenate several revisions (component #9200 `:115` shows `queue:rev:...;graph:rev:...`). |
| `max_decimal_digits`      | 39          | digits | every canonical decimal string (sign not counted)                  | 39 digits hold any `u128`. The broker's SQLite integers are 64-bit today; the bound leaves headroom for a wider store while bounding `cmp_canonical_decimal` work and rejecting padded inputs. |
| `max_evidence_refs`       | 32          | items  | `reason.evidence_refs`, and any evidence list in an observation    | 32 references at 256 bytes is 8 KiB, which fits inside `max_command_bytes` with the rest of the envelope. |
| `max_warnings`            | 16          | items  | `Result.warnings`                                                   | Human-readable lines; detail belongs in a referenced artifact. |
| `max_warning_bytes`       | 512         | bytes  | each warning                                                        | As above. |
| `max_error_message_bytes` | 1,024       | bytes  | `Result.error.message`                                              | As above. |
| `max_json_depth`          | 32          | levels | nesting depth of any payload                                        | The fixed v1 shapes nest at most about four levels; the margin is for typed `arguments` and the order DTO's `filter_ast`, whose maximum depth is not specified anywhere (Q4). Bounds parser recursion. |
| `raw_slack_factor`        | 4           | factor | raw bytes may be up to 4x the canonical limit before parsing        | Producers may pretty-print; the canonical-bytes check after parsing is the binding limit. The raw check stops a far oversized payload before any parse work. |

Transport bounds stay as they are and are outer bounds only: the
broker's 16 MiB (`uds_http.rs:37`) and the projector's 10,000,000
bytes (`server.ts:97`, `:327`). DST-04 owns the webhook intake; a
webhook payload may be large, but the Fleet command DST-04 derives
from it must fit `max_command_bytes` (gap G7).

### 4.2 How sizes are measured

- **Object limits** are measured on the RFC 8785 canonical bytes
  produced by `canonical_json` (`contracts.rs:1036`): deterministic
  across Rust and TypeScript, independent of whitespace and key
  order.
- **Raw bound** is `raw_slack_factor x` the object limit, checked on
  received bytes before parsing, in the same spirit as
  `readBoundedBody` (`webhook.ts:7-29`) and `uds_http.rs:33-37`
  (refuse, never clamp).
- **Item limits** count array elements.
- **Digit limits** count ASCII digits after an optional leading `-`
  where the field is signed (DST-06 record `:148-151`).
- **Depth** counts nested arrays and objects from the top object
  (depth 1).

### 4.3 Refusal and truncation

- **Nothing on the wire is truncated.** No value, identity, version,
  reference, evidence list, warning list, population or page is cut
  to fit. Truncation changes meaning and, for hashed content,
  silently produces a different digest; the architecture already
  forbids a hash over a partial population (`architecture.md:245-246`).
- **Refusal is whole-object and typed.** The refusing side returns
  `payload_too_large` with `field` (a JSON pointer, `"/"` for the
  whole object), `unit`, `limit`, `observed` and `limits_ref`. It
  mutates nothing (`ContractError::mutates_canonical_state` stays
  `false`, `contracts.rs:139-141`).
- **The only sanctioned way to move more data is pagination**, where
  the reader asked for a page: each page is complete as a page and
  labelled partial (DST-06 `PartialOrder`), and its consumer
  classifies the set as `degraded` with cause `partial_population`
  until it holds the complete population at one snapshot. A complete
  population too large for `max_snapshot_bytes` or
  `max_snapshot_items` cannot be sent as one conformant DTO; the
  producer answers the complete form with `payload_too_large` and
  may serve pages. Computing an `order_hash` in memory over a
  complete population is not limited by this table (DST-06 record
  `:341-347`); only transmission is (Q5).
- **Display elision is not truncation.** A rendering layer may
  shorten a long reference to fit a screen if the full value stays
  reachable and the priority value itself is never altered. That is
  presentation, outside this contract.
- **Producers must not emit over-limit payloads.** A producer that
  would exceed a limit returns `payload_too_large` itself rather than
  sending and relying on the consumer to refuse.

### 4.4 Producer-side refusal of a command

A `fleet.command/v1` carrying a priority intent that is too large,
malformed, or of an unsupported version is answered with outcome
`rejected` and the typed error (`payload_too_large`,
`invalid_schema`, `unsupported_version`, `unsupported_reason_code`),
no canonical mutation, no journal fact and no outbox intent. A stale
`expected_work_version` remains DST-03's `conflict` with reason
`stale_snapshot`; DST-07 does not restate it.

### 4.5 Recovery intent

The oracle requires that "oversize/unknown version is typed and
retains authorized recovery intent." A `RecoveryIntent` is built
**only** when both hold:

1. the input came with an `AuthenticatedPrincipal` from the auth
   layer (`contracts.rs:770-778`), never from `actor_ref`; and
2. `evaluate_authorization` (`contracts.rs:803-823`) returns
   `Authorized` for the capability the input would have needed
   (for a priority write, `priority.write`, `priority.rs:115`).

It is built for states `too_large` and `unsupported` only (the two
the oracle names). An unauthenticated or unauthorized oversize or
unknown-version input gets the typed refusal and **no** recovery
intent: keeping attacker-chosen material would turn the refusal path
into storage. Transport bookkeeping that already exists (the
projector's delivery record, `server.ts:338`) is unchanged.

A recovery intent holds references and a digest, never the payload
(component #9200 `:90` prefers references and hashes):
`source_ref`, `source_event_ref`, `delivery_id`, the authenticated
`principal_ref`, the refused state and causes, the `schema_version`
seen if one parsed, the raw byte length, `raw_bytes_sha256` (a plain
SHA-256 over the exact received bytes, an artifact digest in the
sense of `architecture.md:254`, not a domain-registry digest, so the
registry is not amended), the `limits_ref`, and `received_at_utc`.

Recovery means re-obtaining the input from its source after the
consumer is upgraded or the producer re-sends in a supported, smaller
form, then submitting it through the ordinary path with its own
idempotency key. Whether every source can re-deliver (for example a
GitHub webhook redelivery) was not verified for this record (Q6).
A recovery intent is not an accepted command, does not enter the
idempotency ledger as accepted, and never mutates work state.

DST-07 does not persist it. The shared `CommandResult` has no slot
for it (`contracts.rs:860-891`); the stable receipt that should carry
it belongs to REL-03 (#9513), and an accepted-intent record to DST-03
(#9453). Until one of them places it, DST-07 returns it as a value
and sets `next_action` to `"recover"` (gap G4).

## 5. Proposed Rust API

All types are pure: no I/O, no queue access, no clock reads (the
caller passes `now`). Wire keys are snake_case; integers on the wire
are canonical decimal strings, matching DST-06's DTOs.

### 5.1 `work/contracts.rs` (additive)

```rust
/// Identifier of the limits table in force.
pub const PAYLOAD_LIMITS_REF: &str = "fleet.payload-limits/v1";

/// Unit of a payload limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitUnit { Bytes, Items, Digits, Depth }

/// The versioned limits table (section 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadLimits {
    pub max_command_bytes: u64,        // 16_384
    pub max_result_bytes: u64,         // 16_384
    pub max_observation_bytes: u64,    // 8_192
    pub max_page_items: u64,           // 1_000
    pub max_snapshot_items: u64,       // 10_000
    pub max_snapshot_bytes: u64,       // 4_194_304
    pub max_ref_bytes: u64,            // 256
    pub max_composite_ref_bytes: u64,  // 1_024
    pub max_decimal_digits: u64,       // 39
    pub max_evidence_refs: u64,        // 32
    pub max_warnings: u64,             // 16
    pub max_warning_bytes: u64,        // 512
    pub max_error_message_bytes: u64,  // 1_024
    pub max_json_depth: u64,           // 32
    pub raw_slack_factor: u64,         // 4
}

impl PayloadLimits {
    pub const V1: PayloadLimits = PayloadLimits { /* values above */ };
}

// New ContractError variant (code "payload_too_large"):
//     PayloadTooLarge {
//         field: String,      // JSON pointer; "/" for the whole object
//         unit: LimitUnit,
//         limit: u64,
//         observed: u64,
//     },
// `mutates_canonical_state()` stays false for it.

impl Command {
    /// Bounded parse: raw bound, parse with duplicate-key and depth
    /// checks, canonical-bytes bound, per-field reference, digit and
    /// list bounds, then the existing `validate`. `from_json` keeps
    /// its current behaviour for compatibility.
    pub fn from_json_bounded(raw: &[u8], limits: &PayloadLimits)
        -> Result<Command, ContractError>;
}

impl CommandResult {
    /// Producer-side check before emitting a result.
    pub fn check_limits(&self, limits: &PayloadLimits) -> Result<(), ContractError>;
}
```

`from_json_bounded` is new rather than a change to `from_json`, so
DST-01's existing tests and callers keep their behaviour (DST-07's
compatibility rule, `issues/bodies/DST-07.md:59`). The only other
module that converts `ContractError` exhaustively-or-not is
`work/journal.rs:103-114`, which has a catch-all arm, so the new
variant compiles there and maps to `invalid_schema` with the code in
its detail.

Whether `fleet.command/v1` should reject unknown keys is not decided
here (Q1). DST-07's own new types are strict
(`#[serde(deny_unknown_fields)]`).

### 5.2 `work/distribution.rs` (DST-07 section)

```rust
pub const PRIORITY_OBSERVATION_HEADER_KEYS: [&str; 11] = [
    "schema_version", "work", "work_version", "as_of_utc", "snapshot_ref",
    "profile_ref", "taxonomy_ref", "policy_ref", "comparator_ref",
    "producer", "value",
];
pub const DISTRIBUTION_STATE_SCHEMA_VERSION: &str = "fleet.distribution-state/v1";

/// The consumer-side state of one distributed priority payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionState {
    Fresh, Stale, Degraded, Unsupported, TooLarge, Invalid, Conflicted, Unknown,
}

impl DistributionState {
    pub const ALL: [DistributionState; 8];        // in precedence order (3.2)
    pub fn as_str(&self) -> &'static str;
    pub fn may_show_value(&self) -> bool;          // Fresh | Stale | Degraded
    pub fn read_outcome(&self) -> Outcome;         // section 6.2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerClass { Decision, Relay, Display }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerAction { UseAsInput, Show, ShowWithMarker, ShowMarkerOnly, FailClosed }

/// The section 3.4 matrix; total over both enums.
pub fn action_for(state: DistributionState, class: ConsumerClass) -> ConsumerAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgeBudget {
    #[serde(with = "decimal_u64")] pub max_age_s: u64,          // 300
    #[serde(with = "decimal_u64")] pub max_future_skew_s: u64,  // 60
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputState { Live, Fallback, Unavailable }

/// Producer health carried in every observation header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerHealth {
    /// A `work::degraded::Mode::label()` spelling.
    pub route_mode: String,
    /// Named inputs ("queue", "graph", "policy", "owner_policy", ...).
    pub inputs: BTreeMap<String, InputState>,
    /// False when the population or inputs are known to be partial.
    pub complete: bool,
}

/// Why a state was assigned. Tagged by `kind` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateCause {
    Oversize { field: String, unit: LimitUnit, limit: String, observed: String },
    Malformed { detail: String },
    FutureAsOf { skew_s: String, tolerance_s: String },
    UnknownVersion { field: String, value: String },
    UnknownReasonCode { code: String },
    DivergentRevision { work_version: String },
    NoObservation,
    SourceUnavailable { source: String },
    AgeExceeded { age_s: String, budget_s: String },
    NewerVersionSeen { observed: String, newer: String },
    ProducerRoute { mode: String },
    InputFallback { input: String },
    InputUnavailable { input: String },
    PartialPopulation { reason: String },   // DST-06 PartialReason spelling
}

/// What this build understands.
pub struct SupportedVersions<'a> {
    pub schema_versions: &'a [&'a str],
    pub profile_refs: &'a [&'a str],
    pub taxonomy_refs: &'a [&'a str],
    pub policy_refs: &'a [&'a str],
    pub comparator_refs: &'a [&'a str],
}

/// A previously held observation, for duplicate/conflict detection.
pub struct PriorObservation<'a> {
    pub work: &'a WorkRef,
    pub work_version: &'a str,
    pub canonical_bytes: &'a str,
}

/// Everything the classifier reads besides the payload. Pinned, so a
/// classification is a pure, replayable function of its inputs.
pub struct ConsumerContext<'a> {
    pub class: ConsumerClass,
    pub now_utc: &'a str,                       // consumer clock, pinned
    pub budget: AgeBudget,
    pub supported: &'a SupportedVersions<'a>,
    pub limits: &'a PayloadLimits,
    pub newest_seen_work_version: Option<&'a str>,
    pub prior: Option<PriorObservation<'a>>,
    pub source_unavailable: Option<&'a str>,    // set when the latest refresh failed
}

/// Inputs that make a refused payload recoverable (section 4.5).
pub struct RecoverySource<'a> {
    pub principal: Option<&'a AuthenticatedPrincipal>,
    pub required_capability: &'a str,
    pub source_ref: &'a str,
    pub source_event_ref: Option<&'a str>,
    pub delivery_id: Option<&'a str>,
    pub received_at_utc: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryIntent {
    pub source_ref: String,
    pub source_event_ref: Option<String>,
    pub delivery_id: Option<String>,
    pub principal_ref: String,
    pub refused_state: DistributionState,      // TooLarge | Unsupported
    pub causes: Vec<StateCause>,
    pub schema_version_seen: Option<String>,
    pub raw_len_bytes: String,
    pub raw_bytes_sha256: String,              // "sha256:" + 64 lowercase hex
    pub limits_ref: String,
    pub received_at_utc: String,
}

/// The classifier's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub schema_version: String,                // DISTRIBUTION_STATE_SCHEMA_VERSION
    pub state: DistributionState,
    pub causes: Vec<StateCause>,               // most severe first
    pub consumer_class: ConsumerClass,
    pub action: ConsumerAction,
    pub evaluated_at_utc: String,
    pub budget: AgeBudget,
    pub limits_ref: String,
    /// The parsed header, when parsing got that far.
    pub header: Option<serde_json::Value>,
    /// The value, only when `state.may_show_value()`; otherwise None.
    pub value: Option<serde_json::Value>,
    pub recovery_intent: Option<RecoveryIntent>,
}

/// Classify one per-item observation. `raw == None` is `unknown`.
pub fn classify_observation(
    raw: Option<&[u8]>,
    ctx: &ConsumerContext<'_>,
    recovery: Option<&RecoverySource<'_>>,
) -> Classification;

/// Classify an order payload: a complete `fleet.selection-order/v1`
/// DTO, or a set of pages (`PartialOrder`, DST-06). Uses the DST-06
/// builder's checks; never re-sorts and never hashes a partial set.
pub fn classify_order(
    raw: Option<&[u8]>,
    ctx: &ConsumerContext<'_>,
    recovery: Option<&RecoverySource<'_>>,
) -> Classification;

/// Relay gate: Ok only for `fresh`. Anything else is the typed error
/// the relay must return instead of writing (section 6.2).
pub fn relay_gate(c: &Classification) -> Result<(), ContractError>;
```

Notes on the shapes:

- `value` is `serde_json::Value` at this layer because the value
  block's DTO is DST-05's; once DST-05 fixes it, this becomes that
  type (expected to reuse PRI-02's `PriorityValue`,
  `ImportanceAssessment`, `PressureFactor`). Either way the only
  constructor is the classifier, and it copies the observed block
  verbatim; there is no API to set `value` independently.
- Times are the authority's canonical UTC strings, passed through
  verbatim and parsed only to compute age, with second resolution.
  The crate's existing parser is `crate::disposition::parse_rfc3339_epoch`
  (`src/disposition.rs:874-876`), which is compiled only with the
  `client` or `queue-broker` feature (`src/lib.rs:54-55`), while
  `work` is ungated (`src/lib.rs:60`). Proposed default: the
  classifier functions are gated on the same features (the done
  command enables both); moving the parser into an ungated module is
  the crate owner's call (Q2).
- `decimal_u64` is a small serde helper that writes `u64` as a
  canonical decimal string and refuses anything else; it lives in
  `distribution.rs`.

### 5.3 Relation to `degraded.rs` and `completion.rs`, and names avoided

- **`work/degraded.rs` (#8581)** answers "which route did this verb
  take when a dependency failed". DST-07 answers "what may a reader
  do with this priority payload". They meet in exactly one place: an
  observation's `producer.route_mode` holds a `Mode::label()`
  spelling, and any value other than `normal` adds cause
  `producer_route` and makes the observation `degraded`. DST-07 does
  not edit `degraded.rs` (not an owned file). To read a label it uses
  a private `fn mode_from_label(&str) -> Option<Mode>` in
  `distribution.rs` whose body is an exhaustive `match` over every
  `Mode` variant producing its label, so a new #8581 variant fails to
  compile there instead of being silently unknown. An unrecognized
  label is cause `unknown_version` on field `producer/route_mode`,
  state `unsupported`.
- **`work/completion.rs` (REL-09)** owns `Freshness`. DST-07 uses
  `AgeBudget` and `DistributionState::Stale`, not `Freshness`.
- **`work/contracts.rs`** owns `Reason`/`ReasonCode` (the priority
  reason registry). DST-07 uses `StateCause`, not "reason", so a
  state cause is never mistaken for a registry code; no code is
  added to the registry.
- **Names not used:** `Mode`, `FailureLeg`, `ModeLine`, `Freshness`,
  `Degraded` (as a type), `Reason`, `ReasonCode`, `Outcome` (DST-07
  maps onto the shared `Outcome`; it does not define another).

## 6. JSON and error contract

### 6.1 Classification (illustrative)

```json
{
  "schema_version": "fleet.distribution-state/v1",
  "state": "stale",
  "causes": [
    { "kind": "age_exceeded", "age_s": "412", "budget_s": "300" },
    { "kind": "producer_route", "mode": "gh-read-degraded" }
  ],
  "consumer_class": "display",
  "action": "show_with_marker",
  "evaluated_at_utc": "2026-11-25T20:20:00Z",
  "budget": { "max_age_s": "300", "max_future_skew_s": "60" },
  "limits_ref": "fleet.payload-limits/v1",
  "header": {
    "schema_version": "<the DST-05 observation schema>",
    "work": { "authority_id": "fleet-main", "work_id": "15981" },
    "work_version": "41",
    "as_of_utc": "2026-11-25T20:13:08Z",
    "snapshot_ref": "queue:rev:...;graph:rev:...",
    "profile_ref": "fleet.queue@1",
    "taxonomy_ref": "...",
    "policy_ref": "...",
    "comparator_ref": null,
    "producer": { "route_mode": "gh-read-degraded", "inputs": { "queue": "live" }, "complete": true }
  },
  "value": { "intrinsic": { "profile_id": "fleet.queue", "profile_version": "1", "value": 30 } },
  "recovery_intent": null
}
```

An `unsupported` example carries `"value": null`, a cause
`{"kind": "unknown_version", "field": "/schema_version", "value":
"fleet.priority-observation/v2"}`, action `show_marker_only` for
display, and, where section 4.5 allows, a `recovery_intent`.

The `profile_ref` spelling `profile_id@version` matches the
`ProfileMismatch` message format in `priority.rs:159-166`.
`PriorityValue.value` is a JSON number today (`priority.rs:316-317`);
queue values are small, so that is within the architecture's "may
exceed" rule and is not changed here.

### 6.2 Mapping onto the shared `Result`

DST-07 adds no outcome. A read operation that returns one classified
payload maps its `state` to the shared eight (`contracts.rs:513-534`):

| `state`       | `outcome`           | `error.code`                                         | `next_action` |
| ------------- | ------------------- | ---------------------------------------------------- | ------------- |
| `fresh`       | `verified_complete` | none                                                 | none          |
| `stale`       | `partial`           | none; causes in `warnings`                           | `reread`      |
| `degraded`    | `partial`           | none; causes in `warnings`                           | `reread`      |
| `unknown`     | `uncertain`         | none                                                 | `reread`      |
| `conflicted`  | `conflict`          | `divergent_revision`                                 | `reconcile`   |
| `unsupported` | `rejected`          | `unsupported_version` or `unsupported_reason_code`   | `recover` when a recovery intent exists, else none |
| `too_large`   | `rejected`          | `payload_too_large`                                  | as above      |
| `invalid`     | `rejected`          | `invalid_schema`                                     | none          |

`verified_complete` on a read means "this read returned a fresh,
supported, complete observation"; it never completes the work item
(`contracts.rs:508-511`).

A **relay** (a write that projects the value onward) calls
`relay_gate` first. For any state but `fresh` it writes nothing and
returns: `stale` as `conflict` with `stale_snapshot` (the existing
code, `contracts.rs:393`, `priority.rs:212`); `degraded` as
`rejected` with `degraded_input`; the other states as in the table.
Its `effects` entry keeps `requested` set and `observed` and
`verified` `null`. A relay result built from a non-`fresh` input can
therefore never be `accepted`, `pending_sync` or `verified_complete`.
That is the oracle's "Stale/unsupported never reports fresh/applied"
made into a type rule.

### 6.3 Error codes

| Code                      | Where                          | New?                                   |
| ------------------------- | ------------------------------ | -------------------------------------- |
| `payload_too_large`       | `ContractError::PayloadTooLarge` | new, in `contracts.rs`                 |
| `invalid_schema`          | existing                       | no                                     |
| `unsupported_version`     | existing                       | no                                     |
| `unsupported_reason_code` | existing                       | no                                     |
| `stale_snapshot`          | existing reason and error code | no                                     |
| `divergent_revision`      | `Result.error.code` only       | new string, not a `ContractError` variant and not a registry reason code |
| `degraded_input`          | `Result.error.code` only       | new string, as above                   |

`payload_too_large` is a `ContractError` because it is a contract
failure of the payload itself. The two new strings describe a
relay's refusal to act on a classified observation; following the
DSP-05 record's treatment of its own codes (that record's
section 8), they are `Result.error.code` values and are not added to
the priority `ReasonCode` registry.

### 6.4 Refusal body for `payload_too_large`

```json
{
  "schema_version": "fleet.result/v1",
  "operation_id": "op_...",
  "work": { "authority_id": "fleet-main", "work_id": "15981" },
  "work_version": "41",
  "outcome": "rejected",
  "effects": [],
  "warnings": ["limits_ref=fleet.payload-limits/v1 field=/reason/evidence_refs unit=items limit=32 observed=40"],
  "error": { "code": "payload_too_large", "message": "payload_too_large: /reason/evidence_refs has 40 items, limit 32" },
  "next_action": "recover",
  "observed_at": "2026-11-25T20:20:00Z"
}
```

When the refused command could not be parsed far enough to yield
`work` and `operation_id`, there is no `fleet.result/v1` to return
(those keys are required, `contracts.rs:862-890`); the transport
answers with its existing refusal (for HTTP, 413 as today) and the
recovery intent, if any, is the only structured record (gap G4).

## 7. Interactions with neighbouring slices

- **DST-06 (#9456).** DST-07 classifies DST-06 payloads and adds no
  order type. A `PartialOrder` page set is `degraded`
  (`partial_population`); an unknown `schema_version` or
  `comparator_ref` is `unsupported`, matching DST-06 `:243-251`; a
  `SelectionOrder` refused by DST-06's builder is `invalid` with
  DST-06's `OrderError` name in `malformed.detail`. `max_snapshot_*`
  limit transmission only, not DST-06's in-memory builder.
- **DST-05 (#9455).** Owns the observation DTO and reconciliation.
  DST-07 supplies the header contract (section 2a), the classifier and
  the relay gate; DST-05 decides what "re-read" and "reconcile" do.
- **DST-03 (#9453) and REL-03 (#9513).** Own accepted intent and the
  stable receipt; one of them persists `RecoveryIntent` (section 4.5).
- **DST-04 (#9454).** Owns webhook intake and maps a verified sender
  to an `AuthenticatedPrincipal`; DST-07's recovery rule depends on
  that mapping. DST-04 is also the shared-file predecessor on
  `distribution.rs` (`issues/bodies/DST-07.md:83`).
- **SEL-02 (#9492).** Uses claim-specific result strings
  (`stale_item`, `stale_policy`, ...; SEL-02 record `:296-316`) for
  the claim path. DST-07 does not adopt or change them: a `decision`
  consumer's `fail_closed` simply means it does not propose; if it
  proposes anyway, SEL-02's revalidation answers.
- **PRI-01/PRI-02 (#9478, #9479; closed).** DST-07 consumes
  `PriorityValue`, `ProfileMismatch` spelling and the "never
  fabricate" rule; it changes neither module.
- **#8581 / #8583.** Consumed through `Mode` labels only (section 5.3).

## 8. Failure modes

| Scenario                                                                 | State / behaviour                                                                 |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| Observation 7 minutes old, display consumer                              | `stale`; value shown with marker and age                                          |
| Same observation, relay consumer                                         | `stale`; no write; `conflict` / `stale_snapshot`; re-read                        |
| Broker read fails, last good read retained                               | `stale` with `source_unavailable`; never `fresh`                                  |
| Broker read fails, nothing retained                                      | `unknown`; marker only; no default, no `0`                                        |
| Item untriaged                                                           | `fresh` with `value: null`; shown as untriaged                                    |
| Producer served from the #8583 replica (`db-degraded`)                   | `degraded`; display marked; decision and relay fail closed                        |
| Graph input `unavailable` in producer health                             | `degraded`, `input_unavailable: graph`                                            |
| Paged order, 3 of 12 pages held                                          | `degraded`, `partial_population`; no hash compared                                |
| `schema_version` from a newer producer                                   | `unsupported`; read-only; recovery intent if authenticated and authorized         |
| Unknown `comparator_ref` on an order                                     | `unsupported`; not hashed, compared or shown as an order (DST-06)                 |
| Unknown reason code inside a supported schema                            | `unsupported`, cause `unknown_reason_code`; never aliased                         |
| Unknown `route_mode` label                                               | `unsupported`                                                                     |
| 40 evidence refs in a command                                            | `rejected` / `payload_too_large`; nothing truncated                               |
| A 60-digit `work_version`                                                | `invalid` via `payload_too_large` on `max_decimal_digits` (state `too_large`)    |
| Raw body 70 KiB for a command                                            | refused before parse (`too_large`)                                                |
| Duplicate identical observation at the same version                      | same classification as the first; no conflict                                     |
| Different bytes at the same `work_version`                               | `conflicted`; relay hands to DST-05 reconcile                                     |
| `as_of_utc` 5 minutes in the future                                      | `invalid`, `future_as_of`                                                         |
| Observation whose `work.authority_id` differs from the prior for the same `work_id` | not a duplicate or conflict: identities are authority-qualified; classified on its own |
| Unauthenticated oversize webhook-derived command                          | `rejected` / `payload_too_large`; no recovery intent                             |
| Rollback of DST-07                                                       | remove the classifier's callers; nothing persisted by DST-07 to revert; retained recovery intents stay with their owner (REL-03/DST-03) |

## 9. Commands allocated

None. The packet states "Command allocation: `none; this is not a
command-owning slice`" (`issues/bodies/DST-07.md:68`). Any read verb
that returns a classification belongs to its own command's slice
(CLI-02 / CLI-13 own shared CLI glue, `issues/bodies/DST-07.md:27`).

## 10. Open questions

- **Q1. Unknown keys in `fleet.command/v1`.** DST-01's `from_json`
  ignores unknown keys other than `authorized`
  (`contracts.rs:666-682`; no `deny_unknown_fields`). Component
  #9200 `:135` allows ignoring additive fields "only if marked
  optional and negotiated", and no negotiation mechanism exists. Does
  v1 reject unknown keys (a DST-01 behaviour change) or keep ignoring
  them? DST-07's new types are strict either way.
- **Q2. Timestamp parser placement.** Gate the classifier on
  `client`/`queue-broker` to reuse `disposition::parse_rfc3339_epoch`,
  or move a parser into the ungated `work` tree?
- **Q3. Real population size.** What is the largest eligible
  population per lane snapshot today and expected under hosted
  targets? `max_snapshot_items = 10,000` is a sizing assumption, not
  a measurement.
- **Q4. `filter_ast` depth.** The order DTO's `lane_context.filter_ast`
  (`architecture.md:229-233`) has no stated depth or size; is 32
  levels enough, and should `filter_ast` have its own byte limit?
- **Q5. Hash with pages.** DST-06 `:341-347` lets a producer return
  an `order_hash` once it has assembled the complete population, even
  if it serves pages. A consumer holding only pages cannot reproduce
  that hash. Should such a hash be marked "producer-asserted" in the
  classification, or not sent with pages at all?
- **Q6. Re-delivery sources.** Which sources can re-deliver a refused
  input on request (GitHub webhook redelivery, PM action store,
  CLI re-run)? Not verified here; this bounds what a recovery intent
  can actually recover.
- **Q7. Observation schema name.** The header in section 2a needs a
  schema identifier from DST-05 (for example a
  `fleet.priority-observation/v1`); the name is DST-05's to choose.
- **Q8. Display budgets per surface.** Mobile (#9201) and hosted
  dashboards may need budgets other than five minutes; who sets them
  and where are they recorded?

## 11. Conflicts and gaps with neighbouring sources

- **G1. Owned files disagree inside the issue.** The title and
  acceptance line name `distribution.rs`, `contracts.rs` and fixture
  tests (`issues/bodies/DST-07.md:1`, `:49`); the owned-files list
  names only `distribution.rs` (`:23`). This record proposes additive
  changes to `contracts.rs` (section 5.1). `contracts.rs` is also
  owned by DST-01 (#9451, closed) and DST-04 (#9454, open); landing
  order on it must follow DST-04.
- **G2. DST-SNAPSHOT-01 has no contract yet.** It normalizes to DST-05
  (`:36`), which has no record. DST-07 fixes only the header it needs
  (section 2a). DST-05 is not a native blocker of #9457 even though it
  is the normalized alias of a named dependency.
- **G3. Blocker labels in a triage comment.** The 2026-11-16T13:17Z
  comment on #9457 calls #9454 "DST-CMD-01 transport" and #9513
  "DST-SNAPSHOT-01". By title, #9454 is DST-04 (the shared-file
  predecessor) and #9513 is REL-03 (the normalized hard blocker,
  `:31`, `:34`). The native blocked-by list returns exactly #9513 and
  #9454. This record uses the titles.
- **G4. No envelope slot for a recovery intent.** `CommandResult`
  (`contracts.rs:860-891`) cannot carry it, and a command too
  malformed to yield `work`/`operation_id` cannot produce a
  `fleet.result/v1` at all. REL-03 or DST-03 must place it.
- **G5. Invented priority in the aggregator.** `pm-stack/aggregator/src/queue.ts:195`
  maps a missing priority to `0`; section 3.5 forbids that for any
  distributed priority consumer. The file is not DST-07's; the
  aggregator owner (or the PM projection slices, for example PRI-06's
  `pm-stack/aggregator/src/priority-actions.ts` in
  `components/9199-distributed-priority.md:91`) should carry the fix.
  The same function maps a missing `id` to `0` (`:192`), outside this
  slice's scope.
- **G6. Test name versus state.** `pmo_distribution_unknown_version_degrades`
  asserts state `unsupported`, not `degraded` (section 3.1).
- **G7. Webhook payload versus command size.** The projector accepts
  webhook bodies up to 10,000,000 bytes (`server.ts:327`); a Fleet
  command derived from one must fit 16,384 bytes. DST-04 must derive
  by reference, not by copying event bodies.
- **G8. FDR status wording.** The DST-07 body says all FDR-PMO
  records "remain proposed, not ratified" (`issues/bodies/DST-07.md:70`);
  `decisions/README.md:20-27` lists each as "accepted (design only)",
  and component #9200 `:159` says they are proposed until the shared
  index records adoption. This record treats them as accepted for
  local design scope only, which all three texts permit.
- **G9. Internal sequencing text.** `issues/bodies/DST-07.md:32`
  says no file-collision edges were added, while `:79-83` adds the
  DST-04 shared-file edge. The `:83` edge is the one in force.
- **G10. Crosswalk staleness.** The H-R005 crosswalk reason
  (`census/requirements-crosswalk.jsonl:172`) still says 15/25 are
  snapped to 10/20; at HEAD `pm-stack/dispatch/src/validate.ts:15-19`,
  `:63` preserves them (DST-02 closed). The P-R026 crosswalk line
  numbers (`:333`) run two lines ahead of
  `sources/03_full_orchestration_proposal.md` at HEAD (for example
  P-1612 "Phase 1" is cited at 2249 and is at 2247). Neither changes
  this record.
- **G11. `PinnedInputs.as_of` is an integer.** PRI-02's
  `PinnedInputs.as_of: i64` (`priority.rs:2144-2145`) differs from the
  architecture's UTC strings and DST-06's `as_of_utc`. DST-07 uses
  `as_of_utc`; DST-05 should state the mapping if it reuses
  `PinnedInputs`.
- **G12. #9241 closed.** The body calls QTI unlocated and #9241 the
  recovery issue (`:56`); #9241 is closed at 2026-11-25. Its outcome
  was not read for this record. DST-07 persists no field, so it is
  not gated on it.

## 12. Gates and sequencing

Implementation of this slice starts only after:

- PR #9225 merged (done, 2026-11-13).
- #9200 design settlement (open), per the packet's runtime gate
  (`issues/bodies/DST-07.md:57`) as the triage comments on #9457
  apply it.
- REL-03 (#9513, open): the hard blocker (`:31`), for operation
  identity and the stable receipt that carries `RecoveryIntent`.
- DST-04 (#9454, open): shared-file predecessor on `distribution.rs`
  (`:83`), itself after DST-03 and DST-06 on the same file (DST-03
  body `:101`, DST-04 body `:110`).
- DST-01 (#9451) and CLI-02 (#9438): closed; consumed.
- DST-05 (#9455, open): a soft predecessor for the observation DTO
  (G2). The classifier and fixtures can be written against the
  section 2a header before it lands; the header is re-pinned if DST-05
  renames a key.
- Q1 answered before `from_json_bounded` decides unknown-key
  behaviour for commands.

Owned writable file: `pkgs/fleet/crates/fleet-queue/src/work/distribution.rs`
(DST-07 section), with additive changes to `work/contracts.rs` under
G1's landing order, and fixtures under
`pkgs/fleet/crates/fleet-queue/tests/fixtures/distribution-state/`
(sibling of DST-06's `selection-order/`; DST-06 record `:255-258`).
Read-only consumers: `work/degraded.rs`, `work/priority.rs`,
`work/completion.rs`.

Compatibility: additive only. Existing `Command::from_json`,
`ContractError` codes, `Outcome`, `ReasonCode`, the queue's stored
values and every pm-stack surface keep their behaviour; adoption by
each consumer is that consumer's change. Rollback: remove the
callers of the classifier and `from_json_bounded`; nothing DST-07
persists needs reverting; recovery intents already stored by their
owner are kept, per `issues/bodies/DST-07.md:60`.

## 13. Acceptance oracle, specified

Future tests, written failing first, each listed by exact name with
`-- --list` before running (a zero-test filter is not acceptance,
`issues/bodies/DST-07.md:45`), under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.

Fixture format (one JSON file per case):

```json
{
  "fixture_schema": "fleet.distribution-state-fixture/v1",
  "case": "unknown_schema_version",
  "description": "observation with a schema_version this build does not know",
  "context": {
    "class": "display",
    "now_utc": "2026-11-25T20:20:00Z",
    "budget": { "max_age_s": "300", "max_future_skew_s": "60" },
    "supported": { "schema_versions": ["..."], "profile_refs": ["fleet.queue@1"] },
    "newest_seen_work_version": null,
    "prior": null,
    "source_unavailable": null,
    "recovery": null
  },
  "input_raw": "{...exact bytes as a JSON string...}",
  "expected": {
    "state": "unsupported",
    "cause_kinds": ["unknown_version"],
    "action": "show_marker_only",
    "value_present": false,
    "read_outcome": "rejected",
    "error_code": "unsupported_version",
    "recovery_intent": false
  }
}
```

`expected` is written by hand and reviewed; unlike DST-06's
generated byte blocks there is nothing here to regenerate.

1. `pmo_distribution_unknown_version_degrades` (the slice's first
   failing test): an unknown `schema_version`, `profile_ref`,
   `taxonomy_ref`, `policy_ref` and `comparator_ref`, each alone,
   yields `unsupported`, no `value`, `fail_closed` for decision and
   relay, `show_marker_only` for display, read outcome `rejected` /
   `unsupported_version`; with an authorized principal, a recovery
   intent; without one, none.
2. `pmo_distribution_state_precedence`: inputs that satisfy several
   conditions report the section 3.2 winner and list every cause in
   order.
3. `pmo_distribution_stale_by_age_and_by_version`: age over budget,
   a newer `work_version` seen, and a failed refresh over a retained
   read each yield `stale`; at exactly the budget it is still `fresh`.
4. `pmo_distribution_future_as_of_invalid`: beyond the skew
   tolerance is `invalid`; within it is not.
5. `pmo_distribution_degraded_route_and_partial_population`: each
   non-`normal` route label, each non-`live` input, `complete:
   false`, and a DST-06 `PartialOrder` yield `degraded`.
6. `pmo_distribution_unknown_label_and_reason_unsupported`: unknown
   `route_mode` label and unknown reason code yield `unsupported`;
   the reason code is never aliased.
7. `pmo_distribution_payload_limits_refuse_not_truncate`: for every
   row of section 4.1, a payload one unit over the limit is refused
   with `payload_too_large` naming field, unit, limit and observed;
   one unit under passes; the refused payload's classification
   carries no value and no shortened list.
8. `pmo_distribution_oversize_retains_authorized_recovery_intent`:
   authenticated and authorized oversize and unknown-version inputs
   yield a `RecoveryIntent` with `raw_bytes_sha256` equal to SHA-256
   of the exact input bytes and no payload copy.
9. `pmo_distribution_unauthorized_input_retains_no_intent`: no
   principal, a principal without the capability, and a payload whose
   `actor_ref` names the owner all yield the typed refusal and no
   recovery intent (`actor_ref` is never read).
10. `pmo_distribution_never_invents_value`: for every non-`fresh`
    state the classification's `value` is either the observed block
    unchanged (stale, degraded) or absent; never `0`, a default, a
    snapped or mapped value; an untriaged observation stays `null`
    and `fresh`.
11. `pmo_distribution_stale_never_reports_fresh_or_applied`:
    `relay_gate` refuses every non-`fresh` state with the section 6.2
    code; a relay result built after refusal has `observed` and
    `verified` `null` and an outcome outside `accepted`,
    `pending_sync`, `verified_complete`; `read_outcome` matches the
    section 6.2 table.
12. `pmo_distribution_duplicate_and_divergent_revision`: an identical
    re-delivery classifies identically; different canonical bytes at
    the same `work` and `work_version` yield `conflicted`.
13. `pmo_distribution_identity_is_authority_qualified`: the same
    `work_id` under two `authority_id`s is neither a duplicate nor a
    conflict; an invalid `work` pair is `invalid`.
14. `pmo_distribution_action_matrix`: `action_for` over all 8 x 3
    pairs equals the section 3.4 table.
15. `pmo_distribution_mode_labels_cover_degraded_rs`: every
    `degraded::Mode` label is recognized; `normal` adds no cause.
16. `pmo_distribution_classification_round_trips`: every
    `Classification`, `StateCause` and `RecoveryIntent` fixture
    serializes to the documented keys with decimal strings and
    rejects unknown keys.
17. `pmo_distribution_command_bounded_parse`: `Command::from_json_bounded`
    enforces the command limits and then behaves as `from_json`;
    DST-01's `pmo_distribution_command_identity_reason_and_version`
    and `pmo_distribution_command_policy_ref_round_trips`
    (`contracts.rs:1198`, `:1540`) still pass unchanged.
18. `pmo_distribution_state_fixture_corpus`: every file in
    `tests/fixtures/distribution-state/` loads and matches its
    `expected` block.

Issue negative coverage (`issues/bodies/DST-07.md:51`): stale input
(tests 3, 11), duplicate/replay (12), ambiguous resource (13),
unauthorized actor (9), partial remote effect (11: a relay over a
non-fresh input reports no applied effect; DST-05 owns real partial
remote writes), unknown version (1, 6), no false verified or
completed result (11).

## 14. Requirement trace (design-level)

Both allocated IDs reach DST-07 through the parent-approved semantic
support mapping, "versioned/degraded/partial output method; no
invented runtime capability", which is explicitly not an Apply
approval and not a direct `implementation_slice_keys` match
(`issues/bodies/DST-07.md:16-17`, `:67`). In the crosswalk, H-R005's
slice keys are DST-01 to DST-06 and P-R026's are CEN-01, CEN-02,
ROADMAP-01 and DOC-01. "Addresses" means this record defines the
behaviour for DST-07's part; "contributes" means it supplies one input
and another slice owns the outcome.

| ID     | Crosswalk line / status                         | Source (occurrences)                                                                 | Here |
| ------ | ----------------------------------------------- | ------------------------------------------------------------------------------------ | ---- |
| H-R005 | `census/requirements-crosswalk.jsonl:172`, partially implemented | `sources/01_handoff_pm_stack_fleet_orchestration.md:71` (H-092 to H-096): treat priority distribution as its own design; trace producers and consumers; "stable IDs and versioned payloads, provenance and reason codes, replay and reconciliation when views diverge, and how to validate the same ordering across targets"; keep urgent dispatch separate. | Addresses "versioned payloads" and "when views diverge" for the consumer side: sections 3 (states, detection, actions), 4 (bounded payloads), 6 (typed results). Contributes to "trace every producer and consumer" (section 2.1 lists today's consumers and one defect, G5) and to "validate the same ordering across targets" (section 7: unsupported and partial orders are never compared; DST-06 owns the hash). Does not touch urgent dispatch. |
| P-R026 | `census/requirements-crosswalk.jsonl:333`, design-only (candidate) | `sources/03_full_orchestration_proposal.md:2245-2322` (P-1610 to P-1660): "Suggested Implementation Phases", Phase 1 to 7. | Contributes to phase ordering: DST-07 lands after the shared contract and stable references (Phase 1, P-1615) and before consumers rely on distributed values (section 12). Contributes to Phase 3 "Ensure state changes are durable and recoverable" (P-1634) through the recovery intent (4.5), to Phase 4 "remote mutation verification" (P-1638) through the relay gate (6.2), and to Phase 5 "fleet sync verify" (P-1644) through stale and conflict detection (3.3). Adopts no phase as a production constant. |

Parser tests, deployment and live acceptance remain open evidence for
both IDs (crosswalk `missing_evidence` on both lines).

## 15. Out of scope

- Implementation of any of the above; gated per section 12.
- The observation DTO, reconciliation, re-read and Project/label
  projection (DST-05); accepted-intent writes (DST-03); webhook intake
  and sender-to-principal mapping (DST-04); the order DTO, builder,
  fixture and hash (DST-06); the stable receipt and its storage
  (REL-03).
- Claim revalidation and claim result strings (SEL-02); the
  comparator (PRI-03); priority values, profiles and mappings
  (PRI-01, PRI-02); the dispatch adapter's snapping of non-ratified
  off-scale values (DST-02 / PRI-01 owners).
- Changes to `work/degraded.rs`, `work/completion.rs`, the #8581
  route contract or the #8583 replica.
- Fixing the aggregator's `?? 0` (G5) or any pm-stack, mobile or
  hosted consumer; each adopts the classifier in its own slice.
- Changing transport limits (`uds_http.rs`, projector) or the
  queue's task and metadata limits (`validate.rs`).
- Persisting recovery intents, order hashes or classifications;
  any schema, migration, telemetry, rollout or live acceptance.
- Adding a digest domain, a reason code to the registry, or an
  outcome to the shared vocabulary.
