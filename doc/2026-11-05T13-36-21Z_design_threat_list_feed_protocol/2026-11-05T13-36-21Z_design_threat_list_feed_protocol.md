# Design: shared threat-list feed protocol — versioned, signed, subscribable feeds with local override lists

- **Date:** 2026-11-05
- **Status:** design spec only — no implementation built from it in this change.
  Implementation slices are named in §10 and must be filed as their own issues.
- **Issue:** [#6006](https://github.com/developing-today/code/issues/6006).
  **Program:** [#5955](https://github.com/developing-today/code/issues/5955)
  (worlds design program). **Parent topic:**
  [#5973](https://github.com/developing-today/code/issues/5973) (world
  governance — moderation as shared libraries; the moderation pack a
  subscription lives under). **Wave-2 follow-up** from the prior-art research
  under #5955 (Lane B item 25); precedent: cross-server network blacklists
  maintained by third-party bots (Paladin: on by default, opt-out per server).
- **Composes with, does not replace:**
  [#5545](https://github.com/developing-today/code/issues/5545) (writer-set
  authority — whose key vouches for a feed is a writer-set question, §4) and
  the signed-artifact substrate in `pkgs/id` (Ed25519 author keys, BLAKE3
  object ids, per-author sequence + prev-hash chains; see
  `doc/2026-10-10T04-58-36Z_design_signed_artifacts/` and
  `doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`).
- **Scope guard (from #6006):** this spec is the protocol only. Implementing
  the publisher or the subscriber is out of scope here (§10 names the slices).

---

## 0. The rules everything follows

1. **A feed is advice signed by its author, never authority over a world.**
   A world enforces a feed entry only because the world's own moderation
   pack (§5) subscribed to that feed. The feed author gains no power over
   the world beyond the entries the subscription accepts.
2. **Local lists always win.** A world's local override and exemption
   lists outrank every feed entry, in both directions: a local exemption
   defeats any feed listing, and a local listing is never removed by a
   feed. No feed message can edit, shadow, or expire a local entry.
3. **Ingest is a validity gate, never a trust gate — and validity is
   strict.** An update that fails signature, chain, or version checks is
   rejected whole; the subscriber keeps serving its last good state
   (§7.2). Partial application of a feed update is forbidden.
4. **Staleness is a stated state, not silent decay.** Every feed head and
   every entry carries an expiry. What a subscriber does at each
   staleness threshold is defined in §7.1; nothing is ever extended
   silently.
5. **Deterministic merge.** Two subscribers at the same feed heads, the
   same pack configuration, and the same local lists compute the same
   effective decision for every subject (§6). No wall-clock, ordering, or
   transport dependence beyond the stated tie-breaks.
6. **Every decision is explainable.** For any subject, a subscriber can
   answer "why is this blocked / allowed?" with the exact chain: local
   list hit, or feed id + entry + feed head seq, or pack default.

## 1. Objects and wire format

The protocol reuses the `pkgs/id` signed-object shape rather than
inventing a parallel one: each object has an `author` (the feed's
Ed25519 verifying key, 32 bytes), a per-author `seq`, a `prev` hash link,
a `kind`, and a `body`; the object `id` is BLAKE3 over the encoded bytes
including the signature. A world verifies the signature against
`author`, checks the hash chain for gaps, and applies the kinds it
understands; unknown kinds are stored and not applied.

Three object kinds define a feed:

| Kind | Purpose |
|---|---|
| `FeedHead` | Declares a new feed version: snapshot/delta pointers, validity window, entry counts. |
| `FeedDelta` | A batch of entry additions and tombstones against the previous head. |
| `FeedSnapshot` | The full entry set at a seq, content-addressed, so a new subscriber need not replay history. |

### 1.1 Feed identity

A **feed id** is the BLAKE3 hash of the feed's genesis `FeedHead`
(seq 1), which names: the feed author key, a human-readable `name`, the
entry-type profile (§2), the declared publish cadence (§3.3), and the
default entry TTL (§3.4). A feed's identity is its genesis object, not
a URL: any server may mirror a feed, and the server that served an
object is never trusted — the signature is.

### 1.2 FeedHead body

```
FeedHead {
  feed:        FeedId,
  seq:         u64,            // strictly +1 per head
  prev:        Hash,           // previous head's id (empty at seq 1)
  issued_at:   Timestamp,
  expires_at:  Timestamp,      // head validity; see §7.1
  snapshot:    Hash,           // latest FeedSnapshot object id at or before seq
  deltas:      Hash,           // the FeedDelta that produced this head (empty for snapshot-only heads)
  entry_count: u64,            // entries in the effective set at this head
  set_hash:    Hash,           // BLAKE3 over the canonical encoding of the effective entry set
}
```

`set_hash` is the convergence oracle: two subscribers at the same head
must compute the same `set_hash` from their applied state, or the state
is wrong by definition (§9).

### 1.3 FeedDelta body

```
FeedDelta {
  feed:     FeedId,
  seq:      u64,               // the head seq this delta produces
  adds:     [Entry],           // §2; each entry is self-contained
  removes:  [Hash],            // entry ids tombstoned by this delta
}
```

A tombstone removes only entries **from the same feed**. A delta may not
tombstone, edit, or reference-for-removal another feed's entries (§6.3).

## 2. Entries

An entry is the unit a subscriber enforces. Entries are immutable;
correction is a tombstone plus, if needed, a corrected add.

```
Entry {
  id:         Hash,            // BLAKE3 over the canonical entry encoding
  subject:    Subject,         // what is listed
  action:     Action,          // what a subscriber is asked to do
  reason:     ReasonCode,      // closed vocabulary, §2.2
  evidence:   [Ref],           // hashes/URLs backing the listing; may be empty only for `flag`
  listed_at:  Timestamp,
  expires_at: Timestamp,       // per-entry expiry; §3.4
}

Subject =
  | Domain(Name)               // scam / phishing / malware domain
  | ContentHash(Hash)          // hash of known-bad content
  | Principal(AuthorKey)       // known-raider principal (an account key)

Action = Block | Quarantine | Flag    // ordered Block > Quarantine > Flag
```

### 2.1 Subject canonicalization (normative)

Matching is useless if two implementations encode the same subject
differently, so canonicalization is part of the protocol, not an
implementation detail:

- **Domain:** lowercase, IDNA-to-ASCII (punycode), no trailing dot, no
  port, no scheme. A listing matches the domain itself and, unless the
  entry sets `exact: true`, its subdomains.
- **ContentHash:** the BLAKE3 hash of the exact content bytes. Feeds do
  not carry perceptual/fuzzy hashes in v1; a fuzzy matcher is a local
  policy decision layered on top, never a feed subject.
- **Principal:** the raw 32-byte Ed25519 verifying key — the same author
  identity the writer-set machinery matches on. Display names, petnames,
  and server-local account ids are never subjects.

### 2.2 Reason codes

Closed v1 vocabulary: `scam`, `phishing`, `malware`, `spam`,
`raid`, `csam-hash` (hash subjects only; the content itself never
travels in a feed), `impersonation`, `other` (requires a non-empty
`evidence` ref). New codes are a protocol version bump (§3.2), never a
publisher invention — a subscriber that meets an unknown reason code
stores the entry and treats its action as `Flag` (the weakest) until it
upgrades. Unknown **subject** kinds are stronger medicine: the whole
entry is stored-not-applied (§1), because a subscriber cannot know what
it would be matching.

## 3. Versioning, cadence, expiry

### 3.1 Versioning

`seq` is a per-feed monotonic counter with a hash chain over heads
(`prev`). A subscriber tracks `last_applied_seq` per feed. Rules:

- A head with `seq != last_applied_seq + 1` is not applied from a delta;
  the subscriber fetches the missing range or the latest snapshot (§8.2).
- A head whose `prev` does not match the subscriber's current head id is
  a fork (§7.2), never a fast-forward.
- Snapshots are full-state checkpoints: applying snapshot at seq *n*
  sets the effective set exactly, and its `set_hash` must match the
  computed set after application.

### 3.2 Protocol version

The genesis head carries `protocol: 1`. A subscriber refuses a genesis
with a higher major protocol than it implements (it may still mirror
the objects). Entry/action/reason vocabularies only grow by protocol
bump; within v1 they are closed (§2).

### 3.3 Update cadence

The genesis head declares `cadence_seconds`: the publisher's commitment
for the maximum interval between heads **while the feed is active**,
and `retire_after_seconds`: silence longer than this means the feed is
retired, not merely late (§7.1). Publishers SHOULD issue a head at
least every `cadence_seconds` even when nothing changed (an empty delta
is a liveness proof). Recommended defaults: cadence 3600s, retire after
30 days. Subscribers poll or receive pushes on their own schedule;
transport is §8 and is deliberately not normative beyond the request
shape.

### 3.4 Expiry — two clocks

- **Entry expiry:** every entry carries `expires_at`. An expired entry
  stops being enforced at that instant; it remains in stored history.
  Publishers refresh entries they still stand behind by re-adding them
  in a delta before expiry (the re-add is a new entry id). There is no
  "extend" message — silent lifetime extension is exactly the failure
  mode a threat list must not have.
- **Head expiry:** `FeedHead.expires_at` bounds how long a head may be
  served as current. Past it, the feed enters the stale states of §7.1.

## 4. Signing and who vouches for a feed

- Every object is signed by the **feed author key** named in the
  genesis head. Verification is offline and total: signature, chain,
  seq, and window checks in §8.2 either all pass or the update is
  rejected whole (§0 rule 3).
- **Vouching is a writer-set question, not a feed question** (composes
  with #5545). A feed is *counted* by a world only when the world's
  moderation pack (§5) names the feed id in a subscription, and the
  subscription pins the expected feed author key. When writer-set view
  projection (#5545) lands, a pack may instead name a writer set and
  accept any feed whose author key is in that set at the set version the
  pack pins; until then, the pinned-key form is the protocol. A feed
  never vouches for itself: discovery (§8.3) can find feeds, but only
  the pack's pin makes one count.
- **Key rotation:** a `FeedRotate` object signed by the current author
  key names the successor key and takes effect at the next head. A
  rotation not signed by the current key is void. Rotation history is
  part of the feed's stored chain, so a subscriber can audit every key
  that ever spoke for the feed.
- **Key compromise / revocation:** a `FeedRevoke` object signed by the
  current author key retires the feed immediately at the seq it
  appears; subscribers treat all entries from the feed as unenforced
  from that head on (§7.2). A pack may additionally pin a feed as
  revoked locally — local configuration, effective immediately, and
  itself an override under §0 rule 2.

## 5. Subscription semantics (the moderation-pack side)

A subscription lives in the world's moderation pack (#5973): the pack
is world code, parameterized per world, and the subscription is one of
its parameters. A subscription names:

```
Subscription {
  feed:          FeedId,
  author_key:    AuthorKey,      // pinned; §4
  actions:       Map<Action, LocalAction>,  // per-action mapping, §5.1
  rank:          u32,            // pack-declared priority; §6.3
  mode:          Enforce | Advisory,        // §5.2
}
```

- Subscribing is a **world-side act** recorded as a pack change (signed,
  versioned, reviewable — the same discipline as writer-set changes).
  A feed cannot subscribe a world to itself, and a mirror cannot alter
  a subscription.
- Unsubscribing takes effect at once: the feed's entries stop being
  enforced, stored objects are retained for audit, and resubscribing
  resumes from the latest snapshot.
- **Local override / exemption lists** are pack-side lists in entry
  shape: an exemption names a subject that no feed may act on in this
  world; an override names a subject and the local action (including
  `Allow` and `Block`) that replaces any feed outcome. Both are
  evaluated before all feed logic (§6.1) and are not feed data — they
  are never published, synced, or expired by any feed message.

### 5.1 Action mapping

Feed actions are requests, not commands. A pack maps each feed action
to a local action per subscription (`Block → Quarantine` for a cautious
world; `Flag → Flag` typically). Unmapped actions default to `Flag`.
A mapping can weaken or strengthen, but the *feed's* stated action is
what §6 compares before mapping.

### 5.2 Advisory mode

In `Advisory` mode a feed's entries are surfaced to moderators (and
counted in reports) but never auto-enforced. New subscriptions SHOULD
start `Advisory` and be promoted to `Enforce` by an explicit pack
change — the Paladin precedent is opt-out-by-default for a *platform's
own* list; for third-party feeds this protocol makes the world's
opt-in explicit instead.

## 6. The effective decision (normative algorithm)

For a subject *S* in a world *W* at a moment *T*:

### 6.1 Local lists first

1. **Local lists.** If *S* is on *W*'s exemption list → **Allow** (stop).
   If *S* is on *W*'s override list → the override's action (stop).
   (§0 rule 2: this step cannot be reached by feed data.)
### 6.2 Collect, resolve, map

2. **Collect.** From every `Enforce`-mode subscription, take each
   unexpired entry whose subject matches *S* (§2.1) at the feed's latest
   applied head, subject to the feed's staleness state (§7.1).
3. **Resolve conflicts** among the collected entries (§6.3). The result
   is a single feed action, or none.
4. **Map.** Apply the winning subscription's action mapping (§5.1).
   If there were no collected entries → pack default (normally Allow).

### 6.3 Conflict rules (feed-vs-feed)

- A feed's entries never remove another feed's entries. The only
  cross-feed "negative" signal is §6.3 rule (c).
- (a) **Highest rank wins:** the pack's subscription `rank` orders
  feeds; the highest-ranked feed with a matching entry supplies the
  action.
- (b) **Tie → strongest action:** at equal rank, `Block > Quarantine >
  Flag`.
- (c) **Signed clear:** a feed may publish a `clear` entry (an entry
  with action `Flag` and reason `other` naming the cleared entry id in
  `evidence`) — it only affects outcomes when it comes from a feed
  ranked **strictly higher** than the listing feed, in which case the
  listing is suppressed for this world. Equal- or lower-ranked clears
  are advisory annotations shown to moderators, never suppressions.
- (d) **Final tie-break:** still tied, the earliest `listed_at` wins;
  beyond that, the lexicographically smaller entry id wins. (This makes
  rule 5 of §0 — determinism — decidable in tests, §9.)

Conflicts are **surfaced, not hidden**: a subscriber records every
§6.3 resolution where the collected actions disagreed, so moderators
can see that two trusted feeds disagreed and which rule decided.

## 7. Failure modes (stated behavior)

### 7.1 Stale feed

Let *age* = now − last applied head's `issued_at`, against the genesis
commitments (§3.3):

- *age ≤ cadence_seconds × 3*: **fresh**. Normal enforcement.
- *age > cadence × 3 and ≤ retire_after*: **stale**. Entries continue
  to enforce until their own `expires_at` (no early drop — an attacker
  who can silence a feed must not thereby unblock listed subjects), but
  the feed is marked stale in every moderator surface, and no entry
  from it may be *strengthened* by conflict rules (it can lose a §6.3
  comparison, never win one against a fresh feed).
- *age > retire_after_seconds*, or head `expires_at` passed with no
  successor for the same interval: **retired**. All entries stop being
  enforced (they lapse; they are not deleted). A later valid head
  revives the feed — retirement is a state, not a tombstone.

Entry expiry keeps working in every state: a retired or stale feed's
entries still die at their own `expires_at`.

### 7.2 Malicious or corrupt feed

- **Bad signature / broken chain / seq gap that cannot be filled /
  `set_hash` mismatch:** reject the update whole, keep last good state,
  mark the feed `suspect`, and surface to moderators. Repeated failures
  (≥ 3 consecutive) move the subscription's *effective* mode to
  Advisory until a valid head applies cleanly. The feed can degrade
  itself, never silently.
- **Fork (two validly signed heads at the same seq with different
  ids):** freeze the feed at the last common head (last good state),
  enforce nothing newer from either branch, and require a pack-side
  decision (a local override pinning the accepted head id) to unfreeze.
  Fork resolution is a human/pack act, never an automatic race.
- **Volume abuse:** a pack may set per-feed caps (max entries, max adds
  per delta). A delta breaching a cap is rejected whole under the
  corrupt-feed path above. A feed that lists everything has listed
  nothing; caps are how a world says so mechanically.
- **Revoked key:** on a valid `FeedRevoke` (§4), enforcement stops at
  that head. On evidence the author key is compromised (a fork is the
  canonical signal), moderators use the pack-side revoke (§4), which
  needs no cooperation from the compromised key.

### 7.3 Feed-vs-feed conflicts

Covered by §6.3 — stated there, deliberately not restated here so the
algorithm has exactly one normative home.

### 7.4 Malicious or mistaken local state

Out of protocol scope, by design: local lists are the world's own
sovereign data (§5). The protocol's only promise is that feed data can
never alter them (§0 rule 2), which is testable (§9).

## 8. Publisher and subscriber (implementation shape)

### 8.1 Publisher obligations

1. Hold the feed author key; sign every object; never reuse a seq.
2. Issue heads on the declared cadence (§3.3), empty deltas included.
3. Publish a fresh snapshot at least every 1024 deltas or 7 days,
   whichever first, so new subscribers converge quickly.
4. Tombstone corrections promptly; re-add entries before expiry if
   they still stand (§3.4).
5. Serve heads, deltas, and snapshots to anyone (feeds are public
   data; access control on reading a threat list only helps the
   listed). Mirrors are expected and need no permission.

### 8.2 Subscriber algorithm

1. From the pack, load subscriptions (feed id, pinned key, rank, mode).
2. For each feed: request heads with `seq > last_applied_seq` from any
   mirror (the home-server pattern: ask each known source for
   `seq > last_seen`; responses are verified per object, so the source
   is not trusted).
3. If the gap since the latest snapshot is smaller, fetch deltas;
   else fetch the snapshot, then deltas after it.
4. Verify per head, in order: signature → author key matches the pin →
   chain (`prev`, seq) → validity window → apply delta → recompute
   `set_hash` and compare. Any failure: §7.2 path, stop processing
   that feed this round.
5. Recompute effective decisions lazily from stored state per §6 (or
   incrementally, provided results are identical — §9 tests decide).

### 8.3 Discovery

Feeds are discovered out of band (a feed id + a mirror hint). A future
directory may list feeds with their genesis heads; a directory entry
is never a subscription (§4). This spec defines no directory protocol.

## 9. Testing doctrine (oracles named)

- **Oracle 1 — `set_hash` convergence (model-based).** A reference
  model applies a generated sequence of valid deltas/snapshots; the
  implementation's effective set hash must equal the model's after
  every step, including snapshot jumps.
- **Oracle 2 — the §6 decision table (model-based).** Generate local
  lists × subscriptions × entries; the model's decision (a direct
  transcription of §6/§6.3) is the expected output for every case.
- **Invariants (property-based):** (i) local lists win — no generated
  feed traffic changes a decision for a locally listed subject;
  (ii) validity gate — no object failing signature/chain/seq checks is
  ever applied; (iii) idempotence — applying the same head twice is a
  no-op; (iv) monotonicity — `last_applied_seq` never decreases except
  via snapshot application at a stated seq; (v) expiry — no entry is
  enforced at or after its `expires_at` in any feed state.
- **Golden corpus:** the Paladin-style scenarios from #6006's research
  brief (default-on platform list, per-world opt-out, raider wave) as
  fixed acceptance cases.

## 10. Implementation slices (file separately; not built here)

1. Entry/subject canonicalization + codec, with §9 Oracle-1 harness.
2. Subscriber state store + §8.2 sync/verify loop (no enforcement).
3. Pack subscription parameter + §6 decision engine, Oracle-2 tested.
4. Publisher tooling: delta/snapshot issuance, cadence heartbeats.
5. Moderator surfaces: staleness/suspect/conflict reporting (§6.3, §7).

## 11. Open questions (for the review round; do not block the spec)

- Push transport: relays already fan out signed objects for `pkgs/id`;
  whether feed heads need a dedicated push kind or ride the existing
  sync is an implementation-slice decision (§8.1/§8.2 are
  transport-agnostic on purpose).
- Cross-world reputation for feeds (which worlds enforce a feed, with
  what outcomes) is a discovery aid, not trust input — a v2 topic.
- Whether `csam-hash` entries need a separate handling profile
  (restricted evidence refs) is a policy call for Drew/the pack model
  in #5973; the protocol carries the reason code either way.
