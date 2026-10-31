# Investigation: BitTorrent-Style Chunk Retrieval for Graph Objects (LIST 14b)

- **Date:** 2026-10-29
- **Published to repo:** 2026-10-30 (queue task 3924, the wave-2 write for
  #2050). The investigation text is the wave-1 result posted on #2050 on
  2026-10-29; it is published here unchanged in substance so the program's
  design record lives in the repo. Follow-ups A-D in section 8 were filed
  as #2260-#2263.
- **Issue:** developing-today/code#2050 (parent epic #2056, fleet epic #2055)
- **Subject system:** `pkgs/id` in developing-today/code — P2P platform on Iroh
  (iroh 1.x, iroh-blobs 0.103, iroh-gossip 0.101, iroh-docs 0.101 per
  `pkgs/id/Cargo.toml`). Component names are taken from `pkgs/id/src/`
  (commands/get.rs, store.rs, discovery.rs, blob_gate.rs, access.rs,
  pull_policy.rs, documents.rs).
- **Question (LIST 14b):** a chat link (or any app link) must be openable even
  when the reader holds none of the data: the ID is found on someone's public
  server, and the bytes are pulled from multiple holders in chunks,
  BitTorrent-style. Investigate holder discovery for an object ID, chunk
  integrity against the object's hashes, partial-holder behaviour, and how
  this composes with Iroh's existing transfer machinery vs needing a new
  swarm layer. Deliverable: retrieval flow + gap list.

## Verdict up front

**No new swarm layer is needed for the transfer itself, and no new integrity
scheme is needed at all.** iroh-blobs already provides the three hard parts of
BitTorrent's transfer protocol, in a strictly stronger form:

1. **Range-addressable requests** — the blobs protocol requests data as
   `(hash, ChunkRanges)` and can ask several holders for disjoint ranges of
   the same blob in parallel (`GetRequest`/`GetManyRequest`,
   iroh-blobs protocol.rs).
2. **Verified streaming (bao)** — every chunk group (16 KiB) is verified
   against the blob's single BLAKE3 root hash *as it arrives*, via the
   bao-tree outboard. BitTorrent can only validate a piece after downloading
   it whole (SHA-1 per piece, piece length up to 256 KiB+); iroh-blobs
   detects bad data after at most 16 KiB and needs no piece-hash list —
   the 32-byte root hash in the ID-graph link *is* the whole integrity
   contract (Iroh, "Iroh global content discovery", 2026; iroh-blobs docs).
3. **Partial-blob state is a first-class store concept** — the FsStore keeps
   data + outboard per hash, tracks held ranges, and can resume an
   interrupted download; a provider that lacks requested ranges closes just
   that stream and reports the exact missing location (iroh-blobs
   protocol.rs), which is precisely the signal a multi-holder scheduler
   needs to re-ask someone else.

What is actually missing in `pkgs/id` today is **not** transfer machinery —
it is (a) **holder discovery for a hash** (who holds this object, complete or
partial?), (b) a **multi-provider scheduling policy** in the retrieval path
(id's `get` fetches from exactly one connection today), and (c) **wiring**:
the blob access gate that would make holder queries safe to answer
(`blob_gate.rs`) is written but not wired into `id serve`, because its
world-reference index "has no producer yet". Those are three bounded pieces
of work on top of Iroh, not a swarm protocol. Upstream agrees with the
framing: n0's own position is that the verified-streaming protocol is
superior to BitTorrent's transfer protocol and the remaining work is making
"multiprovider downloads of single blobs efficient" — scheduling and
discovery, not a new protocol (iroh.computer blog, above).

## 1. Where retrieval stands in pkgs/id today

- `id get` / `gethash` (`src/commands/get.rs`): local store lookup, else a
  single `store.remote().fetch(conn, hash)` over **one** connection — to the
  local `id serve` instance if running, or to one named peer node
  (`cmd_get_one_remote`). There is no provider set, no range splitting
  across sources, and no fallback if that one source is slow, partial, or
  leaves mid-transfer beyond what a single fetch retry would do.
- The graph→blob link already exists: a document's content hash *is* an
  iroh-blobs hash (`documents.rs`: `blob_hash()` returns
  `iroh_blobs::Hash` from the content bytes), and world artifacts/documents
  reference blobs by hash. So "dereference an ID-graph link" concretely
  means: extract hash → find holders → fetch blob → render. Only the middle
  two steps are missing.
- Peer discovery (`src/discovery.rs`) is **peer** discovery, not **content**
  discovery: servers gossip `PeerAnnouncement`s on the
  `id-peer-discovery-v1` topic (30 s announce, 120 s stale threshold) and
  build an in-memory `PeerDiscovery` table of *servers that exist*. Nothing
  in an announcement says which hashes a peer holds.
- Access control for serving blobs exists as a designed-but-unwired gate
  (`src/blob_gate.rs`, P3.4): fetch (`get`/`get_many`) is allowed only to a
  current member of a world that references the hash; `observe` (which
  answers a blob's size and **held ranges** — the partial-holder primitive)
  needs the same membership and is off by default; refusals are
  indistinguishable for unknown vs forbidden hashes; it fails closed. The
  gate is not wired into `id serve` because the `WorldAccess` reference
  index (hash → worlds referencing it) has no producer yet. A sibling gate
  for event pull (`src/pull_policy.rs`, P6.5) is likewise parsed from
  config but not yet read by `id serve`.
- Iroh address lookup is already multi-path in this codebase: mDNS
  (`iroh-mdns-address-lookup`), optional Mainline DHT address lookup
  (`iroh-mainline-address-lookup`, feature `dht` in Cargo.toml), relays.
  That solves *endpoint* reachability once you know an EndpointId; it does
  not solve *content* → EndpointId mapping.

## 2. Holder discovery for an object ID

Four channels, in the order a resolver should try them. They compose:
each returns *candidate* EndpointIds; candidacy is cheap to verify (a
size/observe query) and content itself is verified by hash, so a lying
discovery channel can waste time but can never make a reader accept wrong
bytes (the same argument n0 makes for Mainline provider records).

1. **The link's own context (first, cheapest).** An ID-graph link is almost
   never a bare hash: it arrives inside a chat message, board post, or doc
   entry in a world/doc the reader already syncs. The author of that entry,
   and every peer syncing that world, is a probable holder. iroh-docs
   already tracks exactly this for its own sync: its `ProviderNodes`
   structure implements iroh-blobs' `ContentDiscovery` trait
   (`find_providers(hash) -> stream of EndpointIds`), populated when a peer
   reports an entry `Complete` and when gossip announces `ContentReady`.
   **This is the model to copy for id's own graph**: the sync layer that
   learns "peer X has object Y complete" should feed a provider table keyed
   by blob hash. For world content this is nearly free — the information
   already flows past.
2. **Gossip holder announcements (the id-native swarm directory).** Extend
   the existing discovery plane: peers announce, per world they serve,
   either full held-hash sets (small worlds) or a compact summary (count +
   Bloom/Cuckoo filter of hashes, or just "ask me" + observe). Because
   `observe` already returns held *ranges*, discovery and partial-holder
   negotiation are the same conversation: candidate → observe → held
   ranges → schedule. Announcements must pass the blob gate's rule —
   membership in a referencing world — or holder lists become a content
   enumeration oracle (the gate's "refusals look the same" rule exists for
   exactly this). Public worlds can announce publicly; private worlds
   announce only inside their membership.
3. **The public server named by the link (the LIST 14b baseline case).**
   "The ID is found on someone's public server": a link may carry a hint —
   the publishing server's EndpointId (directly, or via a pkarr/DNS name,
   the pattern `id` already uses for node identity). That server is holder
   #1 and, more importantly, the **rendezvous**: even if it does not hold
   the bytes, a public server for a world can answer the holder query from
   its provider table (channel 1/2 state). This keeps the cold-start case —
   reader holds nothing, knows nobody — working with zero global
   infrastructure.
4. **Global discovery via Mainline DHT (optional, later).** n0's
   experimental `iroh-content-discovery` demonstrates the full loop on
   Mainline: announce `SHA-1(BLAKE3 hash)` via `announce_peer`, look up via
   `get_peers`, translate the returned host:port pairs to EndpointIds via a
   small signed-record UDP address-index service, verify candidates with a
   size query, hand off to the iroh-blobs downloader. It works today as an
   experiment. For id it is the *last* resort channel, not the first: it
   publishes "someone at this IP claims this hash" to a public DHT, which
   is right for public content (public boards, public files) and wrong for
   membership-gated worlds. Gate it per-world, default off.

**What a holder record needs:** EndpointId, hash, held-ranges summary
(complete / ranges / unknown), last-seen timestamp, and the world(s) under
which the holder may serve it (for the gate). TTL/refresh follows the
existing discovery constants' shape (announce ~30 s, stale ~120 s) for the
gossip channel; DHT announcements refresh on the DHT's own cadence.

## 3. Chunk integrity against the object's hashes

Nothing to design — only to state precisely, because it is stronger than
the BitTorrent baseline the issue names:

- The link carries the blob's BLAKE3 root hash (32 bytes). That single
  value is the complete integrity contract: bao-tree verified streaming
  checks each 16 KiB chunk group against the tree rooted at that hash as
  bytes arrive, from *any* source, in *any* order, including ranges fetched
  in parallel from different holders. There is no piece-hash manifest to
  fetch, trust, or keep in sync (contrast BitTorrent's `.torrent` info
  dict), and no chunk can be accepted from a malicious or confused holder
  unless it hashes into the tree — a bad holder can only waste ≤16 KiB of
  transfer per bad group before detection.
- The outboard (flattened hash tree, ~6% overhead at 1 KiB BLAKE3 chunks;
  iroh-blobs stores it down to 16 KiB chunk groups to shrink it) is derived
  data: the first holder serving a range includes the tree nodes needed to
  verify that range; the store persists data + outboard so verified ranges
  stay verified across restarts and can be re-served.
- Hash sequences (HashSeq — a blob whose content is an ordered list of
  hashes) cover the multi-object case: a graph object that links several
  blobs is fetched as one request with per-member hashes, each member
  independently verified. Graph objects in id are signed at the object
  layer (envelope/chain machinery) *in addition* to blob hashing: the
  signature says who asserted the link; the blob hash says the fetched
  bytes are the linked bytes. Both checks are needed; neither replaces the
  other.
- Failure handling: a chunk group that fails verification is discarded,
  the range returns to the wanted set, and the offending holder is
  deprioritised for that hash (reputation is a local scheduling input,
  never a protocol input — no global ban lists in v1).

## 4. Partial-holder behaviour

Definitions that the scheduler and the store already half-speak:

- A holder is **complete** (all ranges), **partial** (a non-empty proper
  subset of chunk-group ranges), or a **non-holder**. Held ranges are
  knowable two ways: `observe` (gated, off by default — see §1) or simply
  by asking: a provider missing requested ranges closes that stream and
  reports the exact missing location, so a scheduler can discover
  partiality by optimistic requesting + precise fallback.
- **Partial holders are useful from the first verified group.** Because
  verification is per-group against the root hash, a partial holder's
  ranges are exactly as trustworthy as a complete holder's. The scheduler
  treats the wanted set as ranges and assigns each range to any holder
  advertising (or discovered to hold) it.
- **Every downloader becomes a partial holder immediately.** FsStore
  persists verified ranges as they land, so a reader mid-download can
  already serve the ranges it holds to the next reader — the swarm
  property — subject to the blob gate (it may only serve world members,
  and only for worlds referencing the hash). This is also the LIST 15
  succession-game / LIST 14 chat-attachment scaling story: popularity
  creates capacity automatically.
- **Endgame and stalls:** when the wanted set shrinks to ranges in flight
  on slow holders, duplicate-request the tail ranges from other holders
  and keep whichever verifies first (standard endgame mode; cheap because
  duplicate groups are discarded unverified-cost ~zero beyond bandwidth).
  A range no known holder has is a **discovery miss**, not a failure:
  re-run §2 channels (the public-server rendezvous first), back off, and
  surface "0 holders for range X–Y" distinctly from "transfer failed" —
  the user-facing difference between "nobody has this (yet)" and "the
  network ate it".
- **Seeding policy is local, never protocol.** How long a node keeps and
  re-serves partial blobs is a storage/budget decision (LIST 8 budgets:
  per-world/per-peer MB caps apply to swarm serving exactly as to any
  serving). Defaults: keep what you fetched (you fetched it because your
  worlds reference it), evict under store pressure LRU — but never evict
  the *only* known copy silently in a world where you are the public
  server; that is a config-level duty, flagged not solved here.

## 5. Retrieval flow (the deliverable)

```text
reader opens link (world W, entry E, blob hash H)
  │
  1. Local store has H complete? ──yes──► render. (Fast path, unchanged.)
  │ no
  2. Build candidate holder set for H:
       a. world-W sync provider table (peers seen Complete for H)
       b. link hint: publishing server / pkarr name → EndpointId
       c. gossip holder announcements for W (membership-gated)
       d. [public worlds only, opt-in] Mainline content discovery
  3. Verify candidates cheaply: size/observe query (gate: requester must
     be a member of a world referencing H; observe needs world opt-in —
     if observe is refused/off, skip to optimistic requesting).
  4. Schedule: wanted = all chunk-group ranges of H.
       - assign ranges rarest-first (fewest advertising holders),
         prefer complete holders for the bulk, partial holders for
         the ranges only they hold;
       - cap concurrent sources (start 4) and per-source in-flight ranges;
       - iroh-blobs GetRequest with explicit ChunkRanges per source —
         one QUIC connection per holder, ranges disjoint across holders.
  5. On each verified group: persist to FsStore (data + outboard),
     remove from wanted, optionally announce self as partial holder of H
     in W. On a bad group: discard, requeue range, deprioritise holder.
     On a stream closing with missing-range report: shrink that holder's
     advertised ranges, requeue the rest elsewhere.
  6. Endgame: duplicate-request the last in-flight ranges; first verified
     copy wins. Wanted empty ──► blob Complete ──► render; announce
     complete-holder in W.
  7. Wanted non-empty and holder set exhausted ──► re-run step 2 with
     backoff; report "awaiting holders for ranges …", resumable at any
     time because partial state is persisted. Never discard partials.
```

## 6. Composition with Iroh vs a new swarm layer — explicit answer

| BitTorrent piece | Iroh / pkgs/id equivalent | New code needed? |
|---|---|---|
| `.torrent` info dict (piece hashes) | The BLAKE3 root hash in the link itself + bao outboard | No |
| Piece/block transfer protocol | iroh-blobs Get/GetMany with ChunkRanges, verified streaming | No |
| Tracker | §2 channels 1–3 (sync provider table, gossip announcements, public-server rendezvous) | **Yes — small, id-side** |
| DHT (BEP 5/44) | Optional: n0 `iroh-content-discovery` experiment over Mainline | Later, opt-in, public content only |
| Swarm scheduler (rarest-first, endgame, choking) | iroh-blobs Downloader takes a provider set; id's `get` path is single-source today | **Yes — policy in id's retrieval path** |
| Seed/leech lifecycle | FsStore partial state + serve path, under blob gate + LIST 8 budgets | Mostly wiring |
| Access control (BitTorrent has none) | `blob_gate.rs` fetch/observe membership gate | Written; **needs the reference-index producer + serve wiring** |

**Conclusion:** build a *thin* id-side layer — provider table + scheduler
policy + gate wiring — on top of iroh-blobs. Do not build a swarm protocol.
The one upstream caveat to track: n0 notes multiprovider downloads of
single blobs are not yet *efficient* upstream (connection/range scheduling
internals are still maturing), so the PoC below should measure, not assume,
the speedup curve as provider count grows.

## 7. Gap list

1. **No hash→holders index in id.** Peer discovery knows servers, not
   content. (→ Follow-up A)
2. **blob_gate unwired:** the `WorldAccess` reference index (hash → worlds
   referencing it) has no producer, so `blob_events` is not in `id serve`;
   holder queries cannot yet be answered safely for private worlds.
   (→ Follow-up A, prerequisite)
3. **Single-source retrieval:** `get`/`gethash` fetch from one connection;
   no range splitting, no fallback, no endgame. (→ Follow-up B, the PoC)
4. **No holder announcements:** gossip carries presence, not holdings; no
   Bloom/summary format chosen. (→ Follow-up A)
5. **No re-serve-while-downloading policy:** partial holders could serve
   verified ranges immediately; nothing enables or budgets it.
   (→ Follow-up C)
6. **Observe is off by default and world-narrowable only downward** — the
   right security posture, but it means the scheduler must also work in
   optimistic mode (request first, learn ranges from missing-range
   reports). The flow in §5 handles both; the PoC must test both.
   (→ Follow-up B)
7. **Global discovery undecided:** Mainline content discovery exists
   upstream as an experiment; id has no position on per-world opt-in,
   privacy review, or the address-index service's trust model.
   (→ Follow-up D, investigation-sized, gated on A)
8. **Abuse surface grows with serving:** swarm serving multiplies who can
   ask a node for ranges; LIST 8 budgets/rate limits must cover range
   serving per peer/world before public-server swarming is enabled.
   (→ Follow-up C, with LIST 8's issue as sibling)
9. **Upstream efficiency caveat:** multiprovider single-blob download
   efficiency is an acknowledged upstream work-in-progress; id should
   benchmark before committing UX promises. (→ Follow-up B measures it)

## 8. Follow-up routing

- **A — Holder discovery for blob hashes** (implementation, filed as #2260): provider
  table fed by sync + gossip announcements; produce the blob_gate
  reference index; wire `blob_events` into `id serve`. Prerequisite for
  everything else.
- **B — Multi-provider retrieval PoC** (proof of concept, review-gated, filed as #2261):
  §5 scheduler over iroh-blobs Downloader in the `get` path; benchmarks
  1 vs 2 vs 4 holders, partial-holder and observe-off modes; NOT the
  permanent feature until reviewed (program rule).
- **C — Partial-holder serving + budgets** (filed as #2262): re-serve verified ranges while
  downloading; per-world/per-peer serving budgets wired to LIST 8 controls.
  Depends on A.
- **D — Global (Mainline) content discovery evaluation** (filed as #2263): privacy/trust
  review + interop spike with `iroh-content-discovery`; public worlds
  only, opt-in. Depends on A; gates any public-swarm claim.

Gated syntheses stay gated: this investigation feeds the LIST 14 synthesis
through epic #2056; it does not itself authorise building the permanent
feature on the PoC (per the program rule and this issue's scope exclusions).

## 9. Hand-off notes for #2051 (ID chat v1 synthesis)

- Chat attachments and linked objects use the section 5 flow unchanged:
  a chat link is the link-context case of section 2 channel 1 (the message's
  world/doc gives the first holder candidates), with the publishing server
  as rendezvous for cold readers (channel 3).
- Chat v1 does not need the swarm scheduler (follow-up B, #2261) to ship:
  single-source fetch from a known holder is the degenerate case of the
  flow. What chat v1 must not do is bake in the single-source assumption —
  the retrieval call should take a holder set, even if v1 passes one holder.
- Integrity for chat-linked objects is the blob hash in the link plus the
  graph-object signature (section 3); chat needs no additional attachment
  hashing scheme.

## Sources

- Subject code: `pkgs/id/Cargo.toml`, `src/commands/get.rs`, `src/store.rs`,
  `src/discovery.rs`, `src/blob_gate.rs`, `src/pull_policy.rs`,
  `src/documents.rs` in developing-today/code (read at main 5a34fa70).
- iroh-blobs protocol documentation (GetRequest/GetManyRequest,
  ChunkRanges, missing-data stream semantics), docs.rs iroh-blobs source.
- Iroh blog, "Iroh global content discovery" (2026): Mainline provider
  announcements for BLAKE3 hashes, address-index service, pkarr names;
  "we still have some work to do to make multiprovider downloads of single
  blobs efficient". https://www.iroh.computer/blog/iroh-global-content-discovery
- Iroh docs, "Blobs" protocol page: content addressing, verified streaming,
  range requests. https://docs.iroh.computer/protocols/blobs
- Bao specification / bao-tree (BLAKE3 verified streaming, outboard trees).
- iroh-docs DeepWiki: `ProviderNodes` implementing iroh-blobs'
  `ContentDiscovery` (provider tracking via sync + gossip ContentReady).
