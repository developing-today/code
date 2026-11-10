# Investigation: the did-it tracker data model

- **Date:** 2026-10-29
- **Issue:** [#2029](https://github.com/developing-today/code/issues/2029) — LIST 7a, "did-it tracker data model".
- **Directive:** Drew, 2026-10-28 (ID platform expansion, epic [#2056](https://github.com/developing-today/code/issues/2056)).
- **Subject system:** `pkgs/id` in developing-today/code — the signed event store
  (`src/event_store/`), the chain envelope (`src/chain.rs`), groups and their fold
  (`src/groups/`), documents (`src/documents.rs`), and tag claims (`src/tag_claims.rs`).
- **Method:** every substrate claim below was read at `origin/main` on 2026-10-29 and is cited
  with `path:line`. Design choices are labelled **Decision** and carry their reasons; they are
  proposals for the review round, not settled API.
- **Scope:** the data model only. No runtime code, no permanent tracker. The consumer is the
  app-packaging work ([#2026](https://github.com/developing-today/code/issues/2026), design
  packet queued as task 3389); this doc names that dependency and does not wait on it.

## Verdict up front

The tracker does not need new infrastructure. Everything hard about it is **modelling on top of
the event substrate that already exists**: events are signed, append-only, causally ordered, and
grouped; a payload is a separate hash-committed body; and group state is a deterministic fold.
The tracker is therefore a set of **event kinds** plus a **pure evaluation function**, not a
service.

Three findings shape the design:

1. **Expectations are not events and must not be stored as if they were.** "Twice a day" and
   "every 2–4h while awake" are _predicates over windows of recorded events_. They are declared
   as data, evaluated offline against the event set, and never materialised as a mutable "due"
   row. This keeps them correct across devices with no server clock and no write contention.
2. **One record with a subject set and per-subject detail slots** answers per-entity cardinality
   better than one event per entity. The dog-care examples force this: "walked both, one peed and
   the other did not" is _one act_ with divergent detail, and splitting it into two events both
   loses the shared act and doubles the write.
3. **Opinions are separate, later events that name the record.** The substrate is append-only
   (`event_store/mod.rs:1-30`), so mutation is not on the table anyway — but the useful part is
   that an opinion carries its own author, time, and group, which is exactly what "I think that
   walk was too short" needs to mean something.

What remains genuinely unsolved is not schema: it is **whose clock decides a window boundary**
(§9) and **what a foreign app may see without the detail map** (§7). Both are stated as open
questions rather than papered over.

---

## 1. What the substrate gives the tracker

### 1.1 The envelope

An event is a signed header plus a separately stored body:

```rust
// pkgs/id/src/chain.rs:262
pub struct Event {
    /// The operation ID: BLAKE3 over `bytes`, signature included.
    pub id: Hash,
    /// The encoded header, signature included. This is what is stored and sent.
    pub bytes: Vec<u8>,
    /// The header, with its extension decoded.
    pub header: Header<ChainExt>,
}
```

The header commits to the payload: "the header commits to its BLAKE3 hash, and the body itself is
not part of the event" (`chain.rs:282-289`). So the body is _content-addressed by the signature_,
which matters twice: an app can store bodies separately from headers, and a projection can carry
the header (and therefore authorship) without the body.

**The envelope carries no timestamp.** `ChainExt` has no time field, and time appears in the store
only as the local arrival clock on waiting rows (`event_store/waiting_rows.rs:39-40`). That is not
an oversight to route around: it is why a tracker record's `at` lives in the _body_ (§3.2), and why
§9's clock question cannot be answered by the substrate.

The signed extension is the extension point a new application kind uses:

```rust
// pkgs/id/src/chain.rs:97
pub struct ChainExt {
    /// What the event is: a group operation, a message, a tag, and so on.
    pub kind: String,
    /// The group the event belongs to. A node can route by it without
    /// decrypting the payload.
    pub group: Option<Hash>,
    /// The author's causal frontier: the latest events from any author that the
    /// author had seen when writing this one.
    pub after: Vec<Hash>,
    pub tag: Option<TagOp>,
    pub document: Option<DocumentRef>,
    pub removes: Option<Hash>,
}
```

`kind` is a free string; existing kinds are namespaced by area — `tag.set` / `tag.clear`
(`tag_claims.rs:34,37`), `device.add` / `device.revoke` (`groups/mod.rs:41,44`),
`group.policy` (the S5 golden body, `chain.rs:733`). `after` is the causal frontier, so a record
can name what it supersedes or responds to _without_ an extra index.

**Decision D0 — the tracker is four new `kind` values, no new envelope fields.** `didit.entity`,
`didit.record`, `didit.expectation`, `didit.opinion`, all `didit.`-prefixed. Reasons: `kind` is
already the extension point, and `LocalEvents::record(kind, tag, document, removes, required_after)`
is already a generic constructor for a new kind (`local_events.rs:499-523`) — no store change, no
migration. Adding envelope fields would change the signed header format and therefore every
existing verifier. A prefix keeps the tracker's kinds from colliding with `tag.*`, `document.*` and
`device.*`.

One hard limit comes with this choice: signed header CBOR caps text at 512 bytes and arrays at 512
(`chain.rs:37-46`), so subject sets and detail maps belong in the body, not the header.

### 1.2 Group state is a fold, not a row

Groups change only by events, and `fold` computes state from the whole set so the result does not
depend on arrival order (`groups/mod.rs:1-10`; `groups/fold.rs:350`). Membership operations go
through `crate::membership`, which wraps p2panda-auth, and roles are ordered
`Pull < Read < Write < Manage < Admin` (`groups/mod.rs:64`). `device.add` / `device.revoke`
already exist, which is how one account writes from more than one device.

Two limits on that precedent, stated because they are easy to over-read. The group fold runs over
`groups.jsonl`, an append-only JSONL log whose events are **not signed** (`groups/log.rs:1-14`),
not over the event store; and encryption epochs — which rotate on create, update, add, remove and
recovery, never on a timer (`encryption/mod.rs:18-19`; `encryption/scheme.rs:236`) — live in the
encryption scheme, not on `ChainExt`.

**Consequence for the tracker:** evaluation has the same _shape_ as `fold` — a pure function over
`(expectations, records, entities)` producing `(satisfied, due, overdue, violated)` per expectation
per window — but it is its own function over signed events, not a reuse of `groups::fold`. It can be
recomputed on any device from the same event set, which is what makes offline and multi-device use
work.

### 1.3 Documents are opaque blobs

`DocumentRef` is `{ media_type, content: Hash, size }` (`documents.rs:43-50`): the store names
content by hash, it does not interpret structure. So a tracker record's structure must live in the
**event body** (CBOR, hash-committed), not in a document. Photos attached to a record are
`document` events or blob references alongside it — a separate concern from the schema.

### 1.4 What is missing

There is no entity concept distinct from an author: an event's author is a key, and groups are
keys. A dog is neither. That gap is why `didit.entity` exists as its own kind rather than being
folded into the group membership record.

---

## 2. The five hard requirements, restated

From the issue and the triage on it (comment 6494358185):

1. **Windowed expectations as data.** "Fed the dogs twice a day" and "dogs outside every 2–4 hours
   during waking hours, which may not be daytime" must be expressible, per household and per
   entity, and evaluable against recorded events.
2. **Per-entity cardinality with divergent detail.** One act may cover one dog or both; the detail
   may differ per dog within the same act.
3. **Non-metric records.** "Drank water" has no quantity. The schema must not force one.
4. **Opinions attach later.** An opinion is added after the fact and never mutates the record.
5. **Cross-app retrieval, possibly without full structural detail.** Another app in the same
   household's graph can retrieve records, and the default projection may omit detail.

---

## 3. The schema sketch

All bodies are CBOR maps, signed via the header's payload hash. Field names below are the
proposal; types are given in a compact schema notation (`?` optional, `*` repeated).

### 3.1 `didit.entity` — what a record can be about

```
didit.entity {
  entity: bytes32,          // stable ID, derived (see below)
  household: bytes32,       // group hash the entity belongs to
  kind: "dog" | "cat" | "plant" | "person" | string,
  name: string,
  aliases: string*?,
  retired_at: int?          // unix seconds; a retired entity keeps its history
}
```

The entity ID is `BLAKE3(household || kind || name_at_creation)`, so two devices that create the
same dog independently converge instead of forking the household's entity set. Renames are new
events with `after` naming the previous one; the ID never changes, because records reference it.

**Why not a tag?** Tag claims (`tag.set`/`tag.clear`, `tag_claims.rs:34-37`) give a live key→value
index, and an entity _could_ be a tag. It is a separate kind because an entity has a lifecycle
(retired, renamed, aliased) that a last-writer-wins tag cannot express, and because records must
be able to reference an entity that a later `tag.clear` should not erase.

### 3.2 `didit.record` — what happened

```
didit.record {
  id: bytes32,              // = event id; carried for convenience in projections
  subjects: bytes32*,       // entity IDs; one or more
  act: string,              // "fed" | "walked" | "outside" | "water" | open string
  at: int,                  // author's unix seconds
  tz_offset_minutes: int,   // author's UTC offset at `at` (see §9)
  details: { bytes32 -> detail }?,   // per-subject, keyed by entity ID
  shared: detail?,          // applies to every subject
  measure: measure?,        // absent for non-metric acts
  note: string?,
  place: geo?,
}

detail {
  note: string?,
  outcome: string?,         // "peed", "did not pee", "refused", free text
  measure: measure?,
}

measure {
  value: number,
  unit: string,             // "g", "ml", "min", "km"
}
```

- **`subjects` is a set, `details` is keyed by subject.** That is the whole answer to requirement 2:
  the act is one event (one author, one time, one causal position), while the divergent part lives
  in a map. "Walked both; A peed, B did not" is one record with two detail entries. Query
  consequences are in §6.
- **`measure` is optional at both levels.** "Drank water" is `{act: "water", subjects: [a, b]}`
  with no measure and no details — requirement 3 is satisfied by omission, and there is no
  `quantity: 0` sentinel to misread.
- **`at` is the author's clock**, not a server's. This is the only honest option in a P2P system,
  and it makes the window-boundary question in §9 unavoidable rather than hidden.

### 3.3 `didit.expectation` — what should happen

```
didit.expectation {
  id: bytes32,
  subjects: bytes32*,          // entities the expectation covers
  act: string,                 // must match record.act
  window: window,              // see below
  active: window_active,       // when the expectation applies at all
  supersedes: bytes32?,        // previous expectation event id, if this replaces it
  ends_at: int?,               // optional stop date
  note: string?,
}

window =
  | { kind: "count_per_period", count: int, period: "day" | "week" | period_custom }
  | { kind: "interval", min_seconds: int, max_seconds: int }
  | { kind: "at_least_once_per", period: period_custom }

period_custom { anchor: int, length_seconds: int }   // e.g. a 24h cycle anchored at 08:00 local

window_active {
  kind: "always"
  | { kind: "hours", hours: [ { from_minute: int, to_minute: int, days: int*? } ],
      // minutes since local midnight; days 0=Mon..6=Sun, absent = every day
      tz: string }          // IANA name, e.g. "America/Chicago"
  | { kind: "sleep_window", from_minute: int, to_minute: int, tz: string }
}
```

Worked meaning:

- **"Fed twice a day"** →
  `{kind: "count_per_period", count: 2, period: {anchor: <local midnight>, length_seconds: 86400}}`
  with `active: {kind: "always"}`.
- **"Outside every 2–4 hours during waking hours"** →
  `{kind: "interval", min_seconds: 7200, max_seconds: 14400}` with
  `active: {kind: "hours", hours: [{from_minute: 420, to_minute: 1320}], tz: "America/Chicago"}`
  (07:00–22:00). "Which may not be daytime" is handled by `hours` being arbitrary data: a
  night-shift household sets `from_minute: 1200, to_minute: 420` (wrapping), and nothing in the
  schema assumes daylight.

**Decision D1 — an expectation is a declaration, superseded by a new declaration, never edited.**
`supersedes` names the prior event. Evaluation uses the latest non-superseded expectation per
`(subjects, act)` in causal order, so a household that changes "twice a day" to "three times a day"
keeps the history of what was expected when, which is what makes a later "you missed Tuesday"
opinion fair or unfair.

**Decision D2 — windows are data, and evaluation is pure.** No `due_at` is written to the store.
Reasons: (a) a stored due date is a mutable row, and the substrate has no mutable rows for
content; (b) two devices offline would compute different due dates and both would be "right";
(c) a pure `evaluate(events, now, tz) -> Vec<Verdict>` can be replayed for any past instant, which
is what the "was I neglecting the dogs last week" question actually needs.

### 3.4 `didit.opinion` — what someone thinks about it

```
didit.opinion {
  about: bytes32*,          // record and/or expectation event IDs
  stance: "good" | "bad" | "unsure" | open string,
  note: string?,
  at: int,
}
```

An opinion is its own event: own author, own `after` frontier (so it can be ordered against
records it responds to), own group. It never edits the record. Requirement 4 is satisfied
structurally — there is no update path to misuse.

**Decision D3 — opinions may target expectations too, not only records.** "This 4-hour rule is
unrealistic for our household" is a real and useful opinion, and it is an opinion about an
expectation. Allowing `about` to name either kind costs nothing and avoids a second opinion kind.

---

## 4. Five worked examples, end to end

The household: group `H`, entities `dogA` and `dogB`, two humans `m1` (phone + laptop) and `m2`
(phone). All examples are the dog-care cases from the issue.

### (a) Fed the dogs twice a day

```cbor
// didit.expectation, m1, group H
{ subjects: [dogA, dogB], act: "fed",
  window: { kind: "count_per_period", count: 2,
            period: { anchor: 1761771600, length_seconds: 86400 } },
  active: { kind: "always" } }

// didit.record, m2, 08:12 local
{ subjects: [dogA, dogB], act: "fed", at: 1761779520, tz_offset_minutes: -300,
  shared: { measure: { value: 200, unit: "g" } } }

// didit.record, m1, 19:40 local
{ subjects: [dogA, dogB], act: "fed", at: 1761820800, tz_offset_minutes: -300 }
```

Evaluation for the day window: two records whose `subjects` cover both entities → `satisfied`.
The second record has no measure, and the schema is content: a household that weighs food in the
morning and free-feeds at night is not violating anything.

### (b) Outside every 2–4 hours during waking hours

```cbor
// didit.expectation, m1
{ subjects: [dogA, dogB], act: "outside",
  window: { kind: "interval", min_seconds: 7200, max_seconds: 14400 },
  active: { kind: "hours", hours: [{ from_minute: 420, to_minute: 1320 }],
            tz: "America/Chicago" } }
```

Records at 07:05, 09:30, 12:10, 16:45, 21:00 → each gap is inside 2–4h except 16:45→21:00
(4h15m, over the max) → one `violated` interval. The evaluation is a scan of consecutive records
inside each active span, with the span's start counting as the first anchor; the first record of a
span is not "late" because the span begins the clock.

Night shift variant: `from_minute: 1200, to_minute: 420` — the wrap is part of the data, and the
evaluator handles a wrapping span as two segments. Nothing in the schema says "daytime".

### (c) One dog or both, with divergent detail

```cbor
// didit.record, m2, walked both, only A peed
{ subjects: [dogA, dogB], act: "walked", at: 1761800400, tz_offset_minutes: -300,
  shared: { measure: { value: 25, unit: "min" } },
  details: { dogA: { outcome: "peed" },
             dogB: { outcome: "did not pee" } } }
```

One event, one author, one timestamp. A query for dogB's history returns this record with
`details[dogB]`; a query for "walks where a dog did not pee" is a filter over the detail map. If
the same walk had been recorded as two events, the pair would be distinguishable only by
timestamp coincidence, and the 25-minute measure would be duplicated or invented.

**Decision D4 — subject sets, not one event per subject.** The alternative (split events joined by
a shared link) was considered: it makes per-entity filtering trivial and each write independent,
but it loses act identity (two events are not one act), doubles the causal noise, and makes
"both dogs were walked for 25 minutes" a join that can half-succeed. The cost of the chosen shape
is that a subject-set query must test set membership rather than equality — a cheap filter (§6).

### (d) Non-metric record: drank water

```cbor
{ subjects: [dogA], act: "water", at: 1761814800, tz_offset_minutes: -300 }
```

No `measure`, no `details`, no `note`. The record exists to say _it happened_. If a household
later wants amounts, they add a `measure` in new records; old records remain valid, and an
expectation of `count_per_period` does not care about the measure at all.

### (e) An opinion attached later

```cbor
// didit.opinion, m1, about the (c) walk, three hours later
{ about: [<record id from (c)>], stance: "bad",
  note: "25 minutes at 4pm in July is too hot for B; let's move this walk after sunset.",
  at: 1761811200 }
```

The record is untouched. The opinion is signed by `m1`, arrives whenever it arrives, and is
retrievable from any app that can read the group. Because it names the record ID, it survives the
record being projected without its detail map (§7) — the opinion still points at the act.

---

## 5. Decision records

**D0 — tracker kinds are `didit.`-prefixed strings in `ChainExt.kind`.** No envelope change.
Envelope fields are signed and shared by every verifier; new kinds cost nothing.

**D1 — expectations are append-only declarations with `supersedes`.** History of what was
expected when is preserved; no edit path exists.

**D2 — evaluation is a pure function over events; no `due` state is stored.** Offline-safe,
multi-device-safe, replayable. The cost is that every client must implement the evaluator
identically — addressed by making it a library function with golden tests (§8).

**D3 — opinions are events that name their subject by ID, and may target expectations as well as
records.** One opinion kind, two targets, no mutation.

**D4 — a record carries a subject set plus a per-subject detail map.** Act identity is preserved;
the query cost is set-membership filtering, not joins.

**D5 — entities are their own kind with derived, content-addressed IDs.** Renames and retirement
are events, so records never dangle and two devices converge on the same ID.

**D6 — records live in the household group; the group's roles are the access model.** Reading
records requires `Read`; writing requires `Write` (`groups/mod.rs:64`). No tracker-specific ACL.

---

## 6. Query shapes the schema must support

All of these are filters over an event set the client already replicates. Nothing needs a new
index: reads walk every applied event today (`local_events.rs:18-20`), and the network side is
whole-store set reconciliation, not a filtered query (`event_store/sync.rs:193-201`). So the
tracker's cost model is "scan the household's applied events, filter by kind and subject", and the
only thing the schema must guarantee is that the filters are cheap — which is why `act`, `subjects`
and `at` are top-level body fields rather than nested structure.

| Question                                 | Shape                                                                                 |
| ---------------------------------------- | ------------------------------------------------------------------------------------- |
| Did dogB go out today?                   | records where `act = "outside"` and `dogB ∈ subjects` and `at` in today's active span |
| What did we do for dogA this week?       | records with `dogA ∈ subjects`, grouped by `act`, ordered by `at`                     |
| Which walks had a bad outcome?           | records where `act = "walked"` and any `details[*].outcome` matches                   |
| Was the twice-daily feed met on Tuesday? | `evaluate(expectations, records, tuesday)`                                            |
| What do people think of that walk?       | opinions where `record_id ∈ about`                                                    |
| One dog vs both                          | `len(subjects)` — cardinality is a field, not a count of events                       |

The set-membership cost of D4 is real but small: the tracker's working set is a household's
records over weeks, not a global corpus.

---

## 7. Cross-app retrieval and the default projection

Requirement 5 says a different app may retrieve records "possibly without full structural
detail". Two mechanisms already exist and the tracker should use both rather than invent a third:

1. **Group-scoped replication.** A record names its `group` in `ChainExt.group`, and "a node can
   route by it without decrypting the payload" (`chain.rs:97-107`). An app that holds the
   household group's keys can read; one that does not, cannot — and can still route and count
   ciphertext.
2. **Projection by role.** `Pull` replicates ciphertext without keys; `Read` reads content
   (`groups/mod.rs:64`). So "full structural detail" is a `Read`-scoped thing by construction.

**Decision D7 — two projections, both derived, neither stored.** The **summary projection** keeps
`id, subjects, act, at, tz_offset_minutes, len(subjects)` and drops `details`, `shared`, `note`,
`measure`. The **full projection** keeps the body. An app with `Read` may request full; the
default a foreign app receives is summary. Because both are derived, no stored copy can drift, and
redaction is a projection choice rather than a delete.

What is _not_ solved: summary projection still reveals `subjects` and `act`, which is a
behavioural graph (who walked which dog when). That is a real privacy decision for the design
round, not a schema defect — noted in §9.

---

## 8. Verification story

- **Golden evaluation tests.** A fixed event set plus expected verdicts at fixed instants,
  including a DST transition in `America/Chicago` and a wrapping active span, so every client's
  evaluator agrees.
- **Schema round-trip tests.** Every worked example in §4 encodes, signs, decodes, and re-encodes
  to identical bytes — the same discipline as the existing golden header tests
  (`chain.rs:733-852`).
- **Supersede ordering.** Two expectations where the older has a later causal position must not
  win; the test asserts the fold order, not arrival order.
- **Projection tests.** Summary projection drops exactly the named fields and keeps IDs stable, so
  an opinion fetched from summary still resolves.

No test here needs a running node; all of it is library-level.

---

## 9. Open questions for the design phase

1. **Whose clock decides a window boundary?** A record carries the author's `at` and
   `tz_offset_minutes`; the _expectation_ carries an IANA `tz`. When a travelling author records
   at 23:00 UTC-8 for a household on UTC-5, "today" is ambiguous. Proposal: evaluate in the
   expectation's `tz`, using record `at` converted to it, and let a `tz_offset_minutes` disagreeing
   with the expectation's tz be visible in the UI rather than silently coerced. This is the one
   place the model can produce a verdict a human disputes.
2. **Does a subject set need a cardinality constraint?** "Fed both dogs" vs "fed the household" —
   should `subjects: []` mean "all current members" (and therefore shift when a dog is added), or
   be invalid? Proposal: invalid; explicit IDs only, so history is stable.
3. **Summary projection still leaks a behavioural graph** (§7). Is `act` itself redactable, or is
   the whole point that a foreign app can see the pattern?
4. **Expectation applicability vs entity retirement.** A retired dog with a live expectation is a
   contradiction; the fold should probably drop expectations whose every subject is retired, but
   that is a rule to state, not to assume.
5. **Do opinions need a stance taxonomy?** `good/bad/unsure` plus free text may be enough; a
   rating scale invites aggregation that means nothing across authors.

---

## 10. Follow-ups this design implies

These are the buildable slices; each should be filed as its own issue when the design is accepted
(filed now as one tracking issue, see the PR description):

1. **`pkgs/id/src/didit/` schema types + evaluator**, library-only, with the §8 golden tests.
   This is the whole of the model and can land before any app exists.
2. **Summary/full projection helpers** on top of the schema types.
3. **App-side rendering** (the tracker half of the first ID app) — belongs to the app-packaging
   work ([#2026](https://github.com/developing-today/code/issues/2026)), not here.
4. **Expectation editor UX** — changing "twice a day" to "three times" is a `supersedes` event;
   the UI must show the history, or D1's fairness benefit is invisible.

## Sources

- Issue [#2029](https://github.com/developing-today/code/issues/2029) (LIST 7a requirements) and its triage, comment 6494358185.
- Parent epic [#2056](https://github.com/developing-today/code/issues/2056).
- Code, read at `origin/main` 2026-10-29: `pkgs/id/src/chain.rs` (envelope, `ChainExt`, golden header tests), `pkgs/id/src/event_store/mod.rs` (store tables and append-only shape), `pkgs/id/src/groups/mod.rs` (roles, `device.add`/`device.revoke`, fold), `pkgs/id/src/groups/fold.rs`, `pkgs/id/src/documents.rs` (`DocumentRef`), `pkgs/id/src/tag_claims.rs` (`tag.set`/`tag.clear`).
- Sibling wave-1 investigations: [#2040](https://github.com/developing-today/code/issues/2040) (TigerBeetle), [#2044](https://github.com/developing-today/code/issues/2044) (Workers fit).
