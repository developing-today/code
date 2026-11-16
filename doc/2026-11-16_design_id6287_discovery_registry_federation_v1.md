# Discovery, registry & federation — v1 design (#6287)

Design record for issue #6287 ("id(phase-14): Discovery, registry &
federation — v1 — first-iteration design"). Core topic: #6277
(research + design). Origin: the next-horizons discussion #1240, per
Drew's 2026-11-05 decision. Sibling step: #6283 (hosted worlds v1,
`doc/2026-11-16_design_id6283_hosted_worlds_v1.md`, merged in PR
#10692) — this record composes with its decisions and does not
re-decide them.

**Entry condition — verified 2026-11-16 (task 16976, lane code-c).**
The condition had two halves, both now met and recorded on #6287:
(1) the core topic's research produced its first-pass findings —
#6277 comment 6881114669 (task 16957, 2026-11-14), covering the
announcement format, pkarr publishing, the registry trust model, and
iroh relay privacy, with the four Phase 14 tensions named; and
(2) the cross-topic predecessor in #6277's sequencing, #6283 (hosted
worlds v1 design), is CLOSED with its design landed (PR #10692).
F12 gates #1061 (CLOSED) and #1069 (CLOSED) are closed inputs;
#1067 is OPEN (fair scheduling — a dependency of hosted operation,
not of discovery semantics; see §8).

Status: v1 design. The deferred items the research pass left open
are decided in §10 with defaults; nothing here authorizes
implementation beyond the breakdown in §11.

## 1. Governing principles (#1240) — how each one holds

- **Local-first always.** The zero-infrastructure path is the invite
  link: a world with no announcement, no registry, and no relay is
  fully usable by anyone holding its link. Every other mechanism in
  this record is additive discovery on top of that floor.
- **Hostable by anyone at any scale.** A registry is a signed file
  (§5); a relay is a stock iroh relay; a pkarr resolver is a public
  good with multiple operators. No component requires our
  infrastructure, our keys, or our permission.
- **P2P behavior identical to hosted behavior.** Announcements,
  registry lists, and resolver order are the same objects and the
  same code paths whether the world is self-hosted or on our hosted
  instance. The hosted instance differs only in which registries it
  subscribes to by default (§5) and that it operates a default relay
  (§7) — both are settings, not protocol branches.
- **Our hosted instance is the public backstop, never a
  requirement.** Our curated registry and our relay are defaults a
  fresh self-host does not inherit (a fresh self-host follows no
  registry — invite links only — until its operator chooses one).

## 2. Disposition of the #6277 findings (accept / decide)

All four research findings are accepted as the shape of v1; the
items the research deferred are decided in §10:

- **Q1 (announcement format)** — accepted: the announcement is the
  primitive (the #812 feed was never built; `id world list` is
  admin-only and its "open" means loaded, not joinable). Schema in
  §3, including mandatory `seq` + `expires_at` and coarse occupancy.
- **Q2 (pkarr)** — accepted: BEP44 mutable items (~1,000-byte
  budget) carry the announcement; everything larger lives behind
  the endpoint. Custody rule is normative in §4: a registry can
  never repoint a world it lists.
- **Q3 (registry trust)** — accepted: curated list, operator
  registries, and mirroring are layers of one model — provenance is
  a fact (self-signed announcements), inclusion is an opinion (a
  registry's signature over a subset). Format in §5.
- **Q4 (relay privacy)** — accepted: announcements carry a relay
  hint, never a direct address or member IPs; relay-required is a
  per-client decision (§7).

## 3. The announcement (exact schema)

A world announcement is a signed object with exactly these fields:

| Field | Type | Notes |
|---|---|---|
| `world_id` | public key (ed25519) | The world's identity; also its pkarr key |
| `name` | string ≤ 64 chars | Display only; names are claims, not IDs |
| `description` | string ≤ 200 chars | One line; longer text lives behind the endpoint |
| `endpoint` | iroh node id + relay hint | Never a raw IP/port |
| `occupancy` | enum: `has_room` \| `near_full` \| `full` | Coarse by design (Q1); exact counts are a privacy leak and the runtime does not track them |
| `tags` | list of ≤ 8 short strings | Discovery hints, unsigned semantics (registries may filter on them; clients must not trust them) |
| `visibility` | enum: `unlisted` \| `listed` | `private` worlds publish nothing at all (§6) |
| `issued_at` / `expires_at` | timestamps | Lifetime ≤ 7 days; expired announcements are void, not stale |
| `seq` | uint64, monotone per world key | The BEP44 sequence; a lower-or-equal seq never replaces a higher one |

The whole object, CBOR-encoded, must fit the pkarr budget: the
field caps above keep the signed packet under ~800 bytes, leaving
headroom inside the ~1,000-byte BEP44 limit. Signing reuses the id
stack's existing envelope machinery (`pkgs/id/crates/id-sync`
envelope/artifact verification, the #1069 pattern): the signature
is by the world key, and provenance is the chain from the world key
to an operator key the verifying party already trusts. A program
(binary/module) announcement is the same object with `endpoint`
replaced by a content hash — one schema, two subjects.

## 4. Publish-to-pkarr

- The announcement is published as the BEP44 mutable item for the
  world key, via the fleet's existing pkarr path (`pkgs/pkarr-worker`
  is operational today; `pkgs/id/crates/id-sync/src/dht.rs` is the
  in-stack DHT surface).
- **Republish rule:** on any field change, and a heartbeat
  republish at half the record's effective lifetime, so a live
  world never fades from the DHT while a dead world's record does.
- **Custody (normative):** the world key is held by the world's
  operator. Our hosted instance holds keys only for worlds it
  operates. A registry holds no world keys, ever — listing is not
  custody, and no registry operation can repoint a listed world.

## 5. Registry protocol

A registry is a **signed, mirrorable list file** (the `pkgs/deploy`
registry is the worked shape lesson) containing:

- the registry's own key and inclusion policy statement;
- entries: `{ announcement (verbatim, self-signed), included_at,
  registry_seq }`;
- tombstones: `{ world_id, removed_at, reason }` — delisting
  propagates as a tombstone-with-reason (decided, §10), so a mirror
  can distinguish "delisted" from "never seen".

**Mirroring** copies announcements (which remain valid — they are
self-signed) and the mirror's *own* inclusion decisions are its own:
a mirror that imports another registry's list re-signs the subset
it chooses to include. Two registries disagreeing about a world is
the expected state, not a conflict. A client's trust question is
never "is this world real" (the announcement's signature answers
that) but "whose inclusion opinions do I follow" — an operator
setting. Defaults: our hosted instance ships subscribed to our
curated registry; a fresh self-host subscribes to none.

## 6. Visibility scopes and resolver order

- **private** — no announcement exists. Discovery is the invite
  link alone. This is the default for new worlds.
- **unlisted** — the announcement is published to pkarr (resolvers
  can find it by key) but never submitted to registries. Anyone who
  learns the `world_id` can find the endpoint; nobody can browse
  to it.
- **listed** — the announcement is published to pkarr *and*
  submitted to the registries the operator chooses.

**Resolver order (decided):** invite link (exact, zero-trust-needed)
→ pkarr by `world_id` (self-signed, registry-independent) →
subscribed registries (browse/search by name and tags). Each layer
is a fallback for the one before; no layer can override a fresher
`seq` from another.

## 7. Relay and privacy defaults

- Announcements carry a relay hint, never the host's direct
  address, and never member IPs.
- **Relay-required is per-client (decided):** the joiner's privacy
  is the joiner's choice. A client in relay-required mode connects
  only via relay (peers see the relay's address); direct
  (hole-punched) connections expose the joiner's IP to the peer and
  the client must say so in its UI, honestly, per Q4's limits.
- Relay choice is a trust choice (the relay operator observes
  metadata): self-hosters may run their own; ours is the public
  default backstop. Usage limits on our relay are an operations
  decision for the hosted-instance step, not this protocol.

## 8. Dependencies — named, not assumed (fresh states, 2026-11-16)

| Dependency | State | Bearing on v1 |
|---|---|---|
| #812 world directory feed | CLOSED (not built) | The announcement is the primitive (§2 Q1) |
| #1061 F12.4 provider registry | CLOSED | Manifest/capability pattern is a closed input |
| #1067 F12.10 fair scheduling | OPEN | Hosted-operation concern; discovery semantics do not depend on it. Named, not assumed |
| #1068 F12.14 portals | OPEN | Portals model cross-world trust as mutual grant; a registry's inclusion opinion must never override a portal grant (§5). Federation *messaging* is #1068's step, not this one |
| #1069 F12.17 trust & provenance | CLOSED | Signature-vs-admin-trust-list pattern generalized per-registry (§3, §5) |
| #6283 hosted worlds v1 | CLOSED (PR #10692) | Predecessor step; worlds this design discovers are the worlds that step hosts |
| `pkgs/pkarr-worker` | operational | The publishing path §4 builds on |

## 9. What breaks first — and the v1 mitigation for each

1. **Dead announcements linger.** A world dies without
   unpublishing. Mitigation: `expires_at` ≤ 7 days + heartbeat
   rule (§4) — silence is self-cleaning.
2. **Registry spam / squatting.** Anyone can announce anything.
   Mitigation: inclusion is the registry's opinion (§5); the
   curated default is curated precisely so spam has no default
   audience. Names are claims, never IDs (§3).
3. **Key loss = identity loss** (research tension 2). v1 accepts
   this honestly: no recovery mechanism is specified, because every
   candidate weakens the provenance guarantee. A future step may
   add operator-key rotation with a signed handover; v1 worlds
   that lose their key are re-created under a new `world_id`.
4. **Griefing of listed worlds** (tension 1). Mitigation is at the
   world layer, not discovery: coarse occupancy, and join gates
   belong to the world/hosting step (#6283), not to the registry.
5. **Relay abuse** (tension 3). A free public relay invites
   bandwidth abuse. v1's mitigation is architectural honesty:
   relay-required is per-client, self-hosted relays are first-class,
   and our relay's limits are an ops decision (§7), not a protocol
   property.

## 10. Open choices — decided here with defaults

| Choice (deferred by research) | v1 decision |
|---|---|
| Exact announcement schema | §3 (field caps sized to the pkarr budget) |
| Registry list format | Signed list file with entries + tombstones (§5) |
| Delisting propagation | Tombstone-with-reason (§5) |
| Heartbeat interval | Republish at half the record's effective lifetime (§4) |
| Resolver fallback order | Invite link → pkarr → subscribed registries (§6) |
| Program announcements | Same schema, content hash in place of endpoint (§3) |
| Relay-required default | Per-client setting; not a per-world or global default (§7) |
| Curated-list default | Default only on our hosted instance; self-hosts default to no registry (§5) |

## 11. Implementation breakdown

Filed from this record (each is its own issue, linked from #6287):

1. Announcement schema + signing in id-sync (the §3 object, CBOR
   encoding, envelope verification, seq/expiry rules) with
   known-answer vectors.
2. Publish-to-pkarr for announcements (BEP44 put via the existing
   pkarr path; heartbeat republish; custody rule enforced by
   construction — no registry-side key handling).
3. Registry list format + a reference registry publisher (signed
   list, entries, tombstones) and mirror tooling.
4. Resolver: invite-link, pkarr, and registry layers in §6 order,
   with seq-freshness arbitration.
5. Client registry-subscription setting + visibility-scope
   handling (private/unlisted/listed) in the world join flow.

## 12. Out of scope (explicit)

- Federation *messaging* between worlds (portals, #1068's step).
- Hosted-instance operations (relay limits, abuse response) —
  the hosted step's record, not this protocol's.
- Key recovery / identity handover (named in §9.3 as accepted v1
  limitation).
- Search ranking, recommendations, or any central index: discovery
  is lists and keys, not a platform.
