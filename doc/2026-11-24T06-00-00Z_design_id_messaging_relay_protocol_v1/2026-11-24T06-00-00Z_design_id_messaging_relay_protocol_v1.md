# ID messaging and relay protocol v1 — design

**Date:** 2026-11-24 · **Source issue:** [#2036](https://github.com/developing-today/code/issues/2036) (LIST 9 synthesis) · **Epic:** [#2056](https://github.com/developing-today/code/issues/2056) (ID platform expansion) · **Audit trail:** AUDIT-7 Lane 6, tracking [#13873](https://github.com/developing-today/code/issues/13873), umbrella [#13871](https://github.com/developing-today/code/issues/13871)
**Inputs (synthesised here, not re-investigated):**
- Ephemeral live data vs the chained event graph — [#2034](https://github.com/developing-today/code/issues/2034), doc `doc/2026-10-29T10-45-00Z_investigation_ephemeral_live_data_retention/`
- Multi-hop relay and P2P media paths — [#2035](https://github.com/developing-today/code/issues/2035), doc `doc/2026-10-29T11-05-00Z_investigation_multi_hop_relay_and_p2p_media/`
- Match audit and netcode replay — [#2037](https://github.com/developing-today/code/issues/2037); comms abuse controls — [#2038](https://github.com/developing-today/code/issues/2038) (referenced for match and dispute classes only)

**Status:** Design only. No implementation is authorised by this document. The PoCs named in §8 must not merge into permanent features before review (program rule, #2056).

Every claim is either **grounded** (cited to code or a repo doc) or **proposed** (this design's choice). The two investigations carry the file:line evidence; this doc cites them rather than repeating it.

---

## 1. Problem and context

ID currently has one message-shaped path: every durable fact is a signed `Header<ChainExt>` event in a per-author chain, stored forever in the redb event store, and synced pairwise by range reconciliation over `/id-sync/1` (**grounded**: `pkgs/id/src/chain.rs:1-16`, `pkgs/id/src/event_store/mod.rs:4-23`, `pkgs/id/src/event_sync.rs:1-26`). That path was designed for durable facts. It does not yet cover three things the platform needs:

1. **Live, high-rate data** (cursors, in-flight editor steps, game input, presence) already runs on separate unsigned paths (`pkgs/id/src/web/collab.rs:1-55`, `world/mod.rs:328-354`). There is no written contract for what may and may not enter the signed graph, so the boundary is implicit. Pushing every keystroke through the chain would cost one Ed25519 signature plus one hash-chain dependency per event, and the event store has no retention path to reclaim them (**grounded** reasoning, #2034 §1).
2. **Immediate group fan-out does not exist.** iroh-gossip is a dependency but is used only for peer-discovery announcements (`pkgs/id/src/commands/peers.rs`). Eventual transitive delivery works through periodic range sync, but nothing pushes a member's message promptly to a member reachable only through another member, and nothing records the path it took (**grounded**: #2035 §1 "Gap statement").
3. **Large payloads and media have no stated home.** The 1 MiB frame cap (`pkgs/id/src/frames.rs:20`) and voice/video/screen-share requirements are not written into one protocol. Without a rule, each feature picks its own answer.

The source issue asks for one protocol v1 that answers all three and names the attestation hand-off points the match system needs. Without it, the match, currency and chat designs each invent a partial relay and signing model.

## 2. Goals and non-goals

**Goals**
- G1. Define the two data classes (durable signed, ephemeral live) and the rule that decides which class a new message kind is in.
- G2. Define the relay rules: what a relay may do, what it cannot do, and how a relayed message is verified.
- G3. Define a durable checkpoint event so the ephemeral class leaves an auditable residue that is O(segments), not O(events).
- G4. Define group fan-out over iroh-gossip for the durable class, with a bridge role for members reachable only through another member, and hop records kept outside the signed envelope.
- G5. State the media boundary: media never rides the gossip tree and never hairpins by default.
- G6. State the large-payload rule: references by BLAKE3 hash, bytes move over iroh-blobs, and reuse the existing chunk split/join for anything that must be a single framed message.
- G7. Name the attestation hand-off points (per-match mutual attestation, two-sided checkpoints) without designing the match system.

**Non-goals**
- N1. No central server in the message path. A relay is an optional privacy proxy, not a required hop.
- N2. No per-keystroke or per-cursor signature. Live traffic is authenticated by a session-scoped key, not by signing each event.
- N3. No new flooding protocol. The fan-out substrate is iroh-gossip's existing epidemic broadcast trees.
- N4. No media codec, packetisation or call-signalling design. Only the constraints media must respect.
- N5. No key-distribution design for media groups (flagged to the media work, §8 Q5).
- N6. No money, currency or ledger design. TigerBeetle remains the settlement authority (`doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/`).
- N7. No retention enforcement in this doc. Compaction is a slice (§5), not part of the protocol contract.
- N8. No metadata privacy promise. Bridges and privacy relays learn social-graph and timing metadata; v1 says so plainly (§4.6).

## 3. Alternatives considered

| Option | Summary | Why not chosen |
| --- | --- | --- |
| **A0. Do nothing** | Keep the single durable path; live features keep their ad-hoc unsigned streams; eventual sync only. | Leaves the live/durable boundary implicit, gives no fan-out to members reachable only via a peer, and each feature invents its own relay and size rules. Acceptable only as a stopgap for the current one-hop cases. |
| **A1. Sign everything per event** | Every keystroke and cursor move becomes a signed chain event. | Ed25519 sign and verify per event is the dominant client CPU cost; sync amplification shifts the reconcile crossover (`sync_method.rs`); storage grows without bound. Rejected (#2034 §1). |
| **A2. Intermittent "initial + final" signature** | One signature at session start and one at end. | A bare final signature proves order within the segment but not timing, and lets a relay truncate the tail. Too weak (#2034 TL;DR). |
| **A3. Segment checkpoints + session MAC (chosen for live class)** | Session header signed once; live frames MAC-authenticated; signed checkpoint every N events or T seconds and on commit/blur/disconnect. | Chosen. Cost is O(segments). Limitation (no per-event third-party proof) is accepted for classes that do not need it, and §4.5 forces two-sided signing where a dispute matters. |
| **B1. Flood / custom gossip** | Write a new fan-out protocol with ID-specific dedupe and repair. | Duplicates iroh-gossip's HyParView + PlumTree, which already provide dedupe and tree repair. Rejected (#2035 verdict 2). |
| **B2. iroh-gossip fan-out with bridge members (chosen for durable class)** | Origin-signed messages gossiped on a per-group topic; members act as bridges; hop records outside the signature. | Chosen. Uses an existing dependency; ID keeps membership, signing and chain rules on top. Swarm sizing for small groups is unverified and is the PoC's first test (§8 Q1). |
| **B3. Central hairpin relay for all traffic** | Server fans out everything. | Violates N1 and makes the operator a single point of censorship and metadata capture. Kept only as opt-in privacy-relay mode (§4.4). |
| **C1. Message-level chunking protocol** | Invent wire chunking under each message. | QUIC streams already segment, retransmit and reassemble (#2035 verdict 5). Rejected for transport; the existing MLS welcome chunker is reused for the 1 MiB frame-cap case. |
| **C2. Payloads inside messages** | Send large bodies inline. | Hits the frame cap and blocks ordering. Rejected in favour of hash references over iroh-blobs. |

## 4. Chosen approach

### 4.1 Two data classes

Every message kind declares a class. The class is a property of the kind, not of the sender.

| Class | Examples | Signing | Storage | Fan-out |
| --- | --- | --- | --- | --- |
| **D — durable chained** | identity and membership ops, signed documents/revisions, durable chat, currency claims, match results | Per event, `Header<ChainExt>` (**grounded** `chain.rs:282-309`) | Event graph, forever per class policy | Gossip for immediate delivery; range sync as backstop |
| **L — live ephemeral** | cursor, editor steps, presence, typing, game input, live chat on low-retention channels | Session header signed once; frames carry a session MAC; signed `session.checkpoint` events | In memory on the session host; durable residue is checkpoints only | Direct session peers; relay only as configured |

Rule: a kind is class D unless its loss or repudiation must be provable by a third party. Class L kinds must still declare a checkpoint policy (§4.3). A kind that needs third-party dispute resolution and high rate uses class D for its result and class L with two-sided checkpoints for its live stream (§4.5).

### 4.2 Relay rules

- **R1. Origin signature is the only trust input for content.** A class D message carries the origin's signature over (content, prev-references, group, sequence). A relay can delay or withhold, but cannot forge, alter or re-attribute it (**proposed**, #2035 verdict 3).
- **R2. Hop records are audit, not trust.** Each bridge appends a hop record (relay ID, previous hop, receipt time) outside the signed envelope. Receivers never accept a message on hop-record grounds.
- **R3. Dedupe by message ID.** The message ID is BLAKE3 over signed bytes (**grounded** `chain.rs:263`). Later copies are dropped.
- **R4. Bridge only for groups you belong to, and only traffic that passed your own ingest.** Forwarding amplifies whatever ingest accepts, so a bridge re-verifies signatures and encoding before forwarding (**proposed**, #2035 §5 risk 3). Rate and budget enforcement points belong to #2038's design; until it lands, bridges apply the existing per-peer limits only.
- **R5. Class L frames are never forwarded by a bridge unless the session policy names it.** Bridges in media paths are selective forwarders of encrypted packets (§4.4); they do not read payloads.

### 4.3 Session header, MAC and segment checkpoints (class L)

Wire and record shapes are **proposed**; field names are provisional until the schema slice (§5 S1).

```
SessionHeader (signed once, class D kind session.open)
  session_id, group, participants[], policy { segment_events, segment_seconds, checkpoint_on[] },
  mac_key_commitment, prev_session_checkpoint (optional)

Frame (class L, not signed)
  session_id, sender, seq, kind, payload, mac   // mac = HMAC(session_key_for(sender), header||payload)

SegmentCheckpoint (signed, class D kind session.checkpoint)
  session_id, segment_seq, prev_checkpoint, root, event_count, t_start, t_end,
  audit_blob_ref (optional BLAKE3), signature
```

- **Cadence:** a checkpoint is emitted every `segment_events` events or `segment_seconds`, and on commit, blur, disconnect and session close. The defaults for collaborative editing are a 2-second / 256-event trial (**proposed**, #2034 follow-on 2); the production default is a per-class setting.
- **Root:** a running hash (or Merkle root) over the segment's ordered frames. Computing it is cheap; no per-event signature is needed.
- **Late joiners and reconnects:** load the latest checkpoint whose state root is known, then replay live frames since it (**proposed**, #2034 §6).
- **Key scope:** session MAC keys are derived per session from a commitment carried in the signed header. Rotation on membership change is a session event; its exact procedure is open (§8 Q3).

### 4.4 Media boundary

- Media (voice, video, screen share) never goes through the gossip tree (**grounded** constraint from #2035 verdict 4).
- Small groups (about six or fewer) use a full mesh of direct peer connections, with QUIC datagrams for live packets and streams for control.
- A member reachable only through a bridge gets a selective forwarder: it routes on a cleartext header (session, sender, packet sequence) and cannot read payloads, because the group key is held only by endpoints.
- A central hairpin exists only in privacy-relay mode, opt-in per session, with its latency and bandwidth cost stated to the user.
- v1 does not design media keys (§8 Q5).

### 4.5 Attestation hand-off points

v1 does not design the match system. It names where it plugs in:

- **H1. Match result (class D):** per-match mutual attestation, a two-party signature over the result (#2037). v1 provides the signed-event envelope and the requirement that both parties' signatures be present before a result is final.
- **H2. Audit or netcode log (class L):** segment checkpoints only, retained 30–90 days (proposed), then a checkpoint plus hash remains.
- **H3. Disputed live segment:** a `peer_signature` variant on `session.checkpoint`, countersigned by a peer at the boundary. This is required for classes that need dispute resolution; whether it is mandatory by default is open (§8 Q6).
- **H4. Currency:** signed claims enter the graph (class D); balances stay in TigerBeetle. v1 adds no currency rule.

### 4.6 Metadata statement

Bridges and privacy relays observe social-graph and timing metadata. The protocol must not be read as a privacy feature because hop records exist. Hop records exist for audit and dedupe only (**grounded** concern, #2035 §5 risk 4).

### 4.7 Large payloads

- **P1.** Messages carry a BLAKE3 reference; bytes move over iroh-blobs (verified streaming, 16 KiB groups, range requests, resume) (**grounded**, #2035 verdict 5 iii).
- **P2.** Anything that must travel as a single framed message and exceeds `MAX_FRAME_BYTES` (1 MiB, `pkgs/id/src/frames.rs:20`) reuses the MLS welcome chunker (`pkgs/id/src/encryption/welcome.rs`, `CHUNK_PAYLOAD_BYTES`). No second chunker.
- **P3.** Media packetisation is the media layer's job (codec-sized RTP-style packets, about 1200 bytes per QUIC datagram).

### 4.8 Interfaces (proposed summary)

- `Kind::class() -> {D, L}` — declared per message kind; a kind without a class fails to compile.
- `Bridge::forward(msg) -> Result<HopRecord>` — verifies (R4) then forwards on the group topic.
- `Session::checkpoint(policy) -> SegmentCheckpoint` — emits on the §4.3 triggers.
- `Verifier::walk(checkpoints) -> Result<()>` — verifies `prev_checkpoint` links and roots from a late joiner's state.

## 5. Dependency-ordered build slices

Each slice is sized for one lane or session and has its own acceptance criterion. None starts implementation before its design gate (§6) is passed.

| # | Slice | Depends on | Acceptance |
| --- | --- | --- | --- |
| S0 | Adopt this doc; add the class field to the kind registry as a doc-only table | — | Every existing kind listed with class D or L, reviewed |
| S1 | Schema: `SessionHeader`, `SegmentCheckpoint`, `session.checkpoint` kind, canonical CBOR, golden vectors, verifier walk | S0 | Golden vectors round-trip; verifier rejects a truncated tail and a broken `prev_checkpoint` |
| S2 | PoC (throwaway, not merged): collaborative editing through session header + MAC + checkpoints, 2 s / 256-event trial, late-joiner rebuild from checkpoint | S1 | Late joiner reconstructs the document from a checkpoint; PoC branch not merged |
| S3 | PoC (throwaway, not merged): five-node bridge test, four mutually visible plus one bridge-only, over iroh-gossip with hop records and dedupe | S0 | One-hop bridged latency and dedupe measured at n=5; swarm-sizing question (§8 Q1) answered |
| S4 | Feature: checkpointed live sessions with per-session MAC, key rotation, timeouts, checkpoint triggers | S2 review, S1 | Collaborative editing passes existing `collab.rs` tests plus checkpoint-trigger tests |
| S5 | Feature: group fan-out over iroh-gossip for class D with bridge role and hop records | S3 review, R1–R4 | Bridged member receives class D message without a direct connection; duplicate copies dropped |
| S6 | Feature: live-view reconstruction (late joiner, reconnect, partition heal) for `collab.rs` and `world/` | S4 | Partitioned peer reconciles at next checkpoint boundary in test |
| S7 | Feature: retention and compaction for the event store, per class, leaving checkpoint residue | S1; must preserve fork proofs and the waiting pool | No `after` resolution breaks after compaction; the store has a retention test (none exists today, `event_store/mod.rs`) |
| S8 | Investigation: two-sided checkpoints for match results and chat disputes | S1 | Written decision on H3 and mandatory/optional default, feeds #2037 and #2038 |
| S9 | Media fan-out boundary: confirm mesh and selective-forwarder constraints with the media owner | S5 | Media owner signs off that §4.4 is the constraint set (no media keys designed here) |

## 6. Rollout and rollback

- **Phase 0 (now):** this design merges as a doc. No behaviour change.
- **Phase 1:** S1 and S2 land behind a feature flag. Existing `collab.rs` behaviour is unchanged for users.
- **Phase 2:** S4 and S5 enable checkpointed live sessions and group fan-out per group, opt-in, starting with collaborative editing and one test group.
- **Phase 3:** S6 and S7 after soak. Compaction ships disabled until the retention test passes.
- **Rollback:** each phase is behind a flag. Turning a flag off returns to today's paths: unsigned `collab.rs` frames and periodic range sync. Checkpoint events already written are ordinary signed events and remain valid; they are simply not consulted. Compaction is the only irreversible step, so it ships last and only per class with its audit blob policy stated.

## 7. Verification plan

- **Schema:** golden-vector tests for `SessionHeader` and `SegmentCheckpoint`; property tests (per the testing doctrine, `doc/agents/testing-doctrine.md`) for the verifier: any prefix truncation or broken link must be rejected. The oracle is the signed-chain rule itself.
- **Class assignment:** a compile-time or CI test that every registered kind declares a class.
- **Relay:** tests that a bridge cannot alter or re-attribute a message (signature failure rejects it), that duplicates are dropped by message ID, and that hop records never affect acceptance. Oracle: R1–R3.
- **Live:** late-joiner rebuild equals the document state at the checkpoint; partition heal converges at the next boundary. Oracle: the checkpoint root.
- **Fan-out PoC:** n=5 with one bridge-only member; measure one-hop latency and duplicate suppression against the iroh-gossip defaults. Oracle: all four visible members plus the bridge-only member receive each message exactly once.
- **Live proof:** a manual two-browser session on the S4 feature, recording checkpoints and reconnect behaviour, before S4 is marked done.

## 8. Open questions, each with a named decider

| # | Question | Decider | Default until decided |
| --- | --- | --- | --- |
| Q1 | Does iroh-gossip's HyParView/PlumTree behave well at n≈5–dozens with a bridge-only member? | S3 PoC result, reviewed by the id-program lead | Use iroh-gossip; if S3 fails, fall back to explicit per-member unicast over `/id-sync/1` for small groups |
| Q2 | Group topic discovery without a central rendezvous: derive the topic from group ID and bootstrap from member addresses? | id-program lead | Derive topic from group ID; bootstrap from known member addresses plus the existing discovery topic |
| Q3 | Session MAC key: committed in the header or derived per session from a long-term key? Rotation mid-session? | id-program lead, with security review | Derived per session from a header commitment; rotate on membership change as a session event |
| Q4 | Which channels are durable vs live, and who may change a channel's policy? | Drew (product), then id-program lead | Durable by default; live only where the channel owner sets it |
| Q5 | Media group key distribution and rotation on join/leave | Media owner (not protocol v1) | Out of scope; MLS in `pkgs/id/src/encryption/` is the expected substrate |
| Q6 | Is a peer countersignature mandatory for any class by default? It requires the peer online at the boundary. | id-program lead, with #2037 and #2038 owners | Optional; required only where the class says so (H3) |
| Q7 | Checkpoint as a graph event or a parallel signed artifact (`pkgs/id/src/artifact.rs` shape)? | id-program lead | Graph event (`session.checkpoint`), so it syncs with existing machinery |
| Q8 | Per-class segment size and interval: world-configurable like `WorldLimits`? | id-program lead | Per-class default in policy; world override allowed |
| Q9 | How do checkpoints and compaction interact with fork proofs and the waiting pool (`event_store/mod.rs:12-22`, `waiting.rs`)? | id-program lead | Compaction keeps fork proofs and waiting-pool entries; S7 cannot pass without this |
| Q10 | Is a witness or notary server for high-value checkpoints in scope, given "servers accept all signed objects" (#2030)? | Drew (product) | Not in v1; revisit with S8 |

Decider note: under FDR-0013 these are decided and recorded on the issue, not parked for the owner. Where a decider is named as Drew, the default in the table applies until Drew overrides it.

## 9. Record

- This doc is the dated design record for #2036. It links back from #2036 when the PR merges.
- Tracking: AUDIT-7 Lane 6, #13873. Queue item: the `repo-health-code-pilot` slice for #2036 (slice 1). This doc satisfies that item's done check: all nine standard sections are present, slices are dependency-ordered with acceptance criteria, and each open question names a decider.
- Follow-on issues from #2034 §8 and #2035 §6 map to S1–S8 above. Filing them is a separate step, to be done with this PR's merge.
