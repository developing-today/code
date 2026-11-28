# S1: event transport

## Question

Design doc `2026-10-11T03-44-33Z_design_groups_events_delegation`, §11 and Phase 1 S1. Four exit criteria:

1. Header adopted or borrowed.
2. Sync path chosen: p2panda-sync LogSync, or range reconciliation.
3. Sequence base: 0 or 1.
4. Ticket-only dialing confirmed.

## Setup

- Toolchain: rustup 1.97.0 (`RUSTC` and `CARGO` from `rustup which`).
- p2panda crates at git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`. They compiled, so the 0.7.1 fallback was not needed.
- iroh 1.3.0, iroh-relay 1.3.0 (feature `server`), iroh-tickets 1.0.0, iroh-base 1.3.0, noq 1.3.0.
- ed25519-dalek 2.2.0 (workspace pin `"2"`, the same version p2panda-core uses). cbor-core 0.10.1 (`serde`). blake3 1.8.7. sqlx 0.9.0, through p2panda-store, not exercised.
- Member `Cargo.toml` changes:
  - Removed: p2panda-auth, p2panda-encryption, p2panda-stream, iroh-blobs, redb, dcbor, unicode-normalization, tempfile, serde_json. p2panda-encryption's `test_utils` enables `p2panda-core/test_utils`, which turns on the signature check in `Header::verify`. Part (b) needs the non-test build.
  - Added: `postcard = "1"` (use-std; 1.1.3 already in the lock), and `iroh-relay` feature `server`.

## Commands

```
cargo check -p s1-transport --all-targets             # clean
cargo test  -p s1-transport --test header             # 9 passed (logs/test-header.txt)
cargo run --release -p s1-transport --bin s1-transport  # parts a, c, d (logs/report.txt)
target/release/dial                                    # part e (logs/dial.txt)
target/release/relay                                   # part f (logs/relay.txt)
```

## Measurements

### (a) Header size

Extension: `kind = "group.policy"`, `domain = "id.events.v1"`, `group = None`. Encoded header bytes.

| after | body | seq 1 (backlink) | seq 0 |
|---|---|---|---|
| 0 | none | 191 | 157 |
| 0 | 100 B | 226 | 192 |
| 1 | none | 225 | 191 |
| 1 | 100 B | 260 | 226 |
| 8 | none | 463 | 429 |
| 8 | 100 B | 498 | 464 |
| 32 | none | 1280 | 1246 |
| 32 | 100 B | 1315 | 1281 |

- Each `after` hash adds 34 bytes, as a 32-byte CBOR byte string. A body adds 35. The backlink adds 34. `group = Some` adds 33.
- The bare p2panda header at seq 0 (no body, no extension) is 104 bytes, measured. This sample's extension adds 53.
- Decode round trip: 16 shapes, all equal after decode and byte-identical after re-encode.

### (b) Verification

- Non-test build: a header with one signature bit flipped passes `Provenance::verify` (`operation/header.rs:185–194`). `validate_header` also passes it.
- The same bytes fail `Header::decode` with `InvalidSignature`, and fail our `check_received` on `verify_strict`.
- Changing `seq_num` without re-signing: typed `verify` still passes. Typed `hash()` returns the cached digest, while BLAKE3 over the encoded bytes changes.
- Identity public key with R = identity and S = 0: plain Ed25519 verify accepts the test message. `verify_strict` rejects it, and so do both decoders.
- **Correction to the research note.** `AnyHeader::decode` already verifies the signature (`operation/any.rs`, "Verify signature" block, `verify_strict`). Decoding from bytes is safe. The unsafe paths are typed `verify()`, `validate_header` on typed headers, and the cached `hash()`.
- p2panda does not enforce MAX_AFTER, and it ignores unknown extension keys. Our check rejects both.

### (c) Sequence base

- Base 0: the first event has no backlink. `validate_header` and `validate_backlink` both pass for seq 0 then seq 1.
- Base 1: `validate_header` on a seq-1 first event returns `BacklinkMissing`. Decode fails with `UnexpectedFieldType(IncompatibleType(Map), "backlink")`, because the extension map is read as the backlink. A phantom backlink passes header validation, but it names no held event.
- Fork gap: two different seq-1 events with the same predecessor both pass `validate_backlink`. Our store has to enforce uniqueness.

### (d) Sync cost

Setup: 1,000 or 10,000 chains of 50 events. The local side has 2 new events on 2 different chains. Control bytes only. The 2 payloads are the same under both options.

LogSync sizes come from p2panda-sync's `LogSyncMessage::Have(LogHeights)`, built from real author keys. The log ID is `u64`, one log per author. A longer log ID would cost more. CBOR is cbor-core's serde encoding, and postcard is shown alongside.

- Have: 38 B per chain (CBOR), 36 B (postcard). That is 38,009 B and 36,003 B for 1,000 chains. PreSync is 43 B (CBOR) and 4 B (postcard). Done is 5 B and 1 B.
- Heights are the last 0-based seq. Logs with no events are absent.

Range reconciliation is my own simulation. Keys are (author, seq). Bounds are shortest separating prefixes. Fingerprints are XOR of 16-byte BLAKE3 event IDs, plus a count. A leaf with at most T items sends its IDs. Both sides learn the exact symmetric difference, and this was checked in every run.

| chains | LogSync control, CBOR (postcard) | range K=2, T=16 | range K=16, T=16 |
|---|---|---|---|
| 1,000 | 76,071 B (72,012) | 4,915 B, 16 flights | 6,198 B, 7 flights |
| 10,000 | 760,071 B (720,012) | 6,046 B, 19 flights | 6,978 B, 8 flights |

- **Round trips.** LogSync needs 2 flights before operations flow (Have, then PreSync). Range reconciliation needs 7 to 19.
- **Crossover at 2 new events.** LogSync is cheaper at 35 chains (2,729 B vs 3,793 B for K=2). Range is cheaper at 40 chains (3,063 B vs 3,109 B). The crossover is about 35 to 40 chains. K=16 is not monotone in this range (3,819 B at 35, 2,784 B at 40, 4,091 B at 45). I did not isolate the cause. Bound sizes vary with where splits land.
- **Diff size moves the crossover.** At 1,000 chains, 20 new events cost 39,683 B for range (LogSync 76,071 B), and 200 cost 311,959 B (LogSync cheaper). At 10,000 chains, 200 new events cost 432,680 B (LogSync 760,071 B).
- **Leaf threshold T.** At 10,000 chains with K=16: T=8 and T=16 give 6,978 B and 8 flights. T=4 gives 7,777 B and 9 flights. T=32 gives 6,978 B. With K=2, a smaller T saves bytes but adds flights (T=4: 5,279 B, 22 flights).

### (e) Ticket dialing

`presets::Minimal` on 127.0.0.1, with IP transports only. Custom ALPN, one length-prefixed request and reply. Each failure case uses a fresh client.

- Success: 77-character ticket with 1 direct address. Reply in 1.2 ms.
- No-address ticket: fails in 27 µs with `No address lookup configured`. Minimal has no lookup service.
- Dead-address ticket (closed UDP port): times out after 30.0 s.
- Gotcha: my first run reused an address from the earlier successful dial, so the no-address case waited 30 s. Failure tests need fresh endpoints.
- LogSync over the connection: **not run** (time box). Store requirement, from source: `LogStore<AnyOperation, VerifyingKey, L, SeqNum, Hash>` for `log_heights` and `log_ranges`, and `OperationStore` for the operations. `SqliteStore` implements both. `SqliteStore::temporary()` is a constructor for a temporary sqlx SQLite store. I did not exercise it.

### (f) Relay

iroh-relay 1.3.0 `Server::spawn` in HTTP mode at `http://127.0.0.1:<port>` (no TLS, no QUIC address discovery). Both endpoints are Minimal with `RelayMode::custom([local])` and `clear_ip_transports()`.

- Both endpoints came online through the relay.
- Ticket: 103 characters, relay URL only, 0 direct addresses.
- Reply in 1.2 ms. The selected path is `Relay(http://127.0.0.1:…)`, with `is_relay = true` and `is_ip = false`.
- No n0 service is involved. Minimal adds no address lookup, and the relay map holds only the local URL.
- Blocker: none.

## Pass or fail per exit criterion

| Criterion | Result | Notes |
|---|---|---|
| Header adopted or borrowed | **Pass (adopted)** | p2panda's header format, with `ChainExt`. Decode from bytes, plus our checks for MAX_AFTER, unknown keys, and canonical re-encode. Never call typed `verify()` or `validate_header`. |
| Sync path chosen | **Pass, with a recommendation** | Range reconciliation (K=16, T=16) for events. At 2 new events, LogSync is cheaper below about 40 chains. It always wins on round trips. LogSync over iroh was not run. |
| Sequence base | **Pass: 0** | See (c). |
| Ticket-only dialing | **Pass** | Works with reachable addresses in the ticket. Fails fast with no addresses, and times out at 30 s with a dead address. |

## Deviations

- Dependency list changed as described in Setup.
- The design says `pkgs/id` pins ed25519-dalek 3.0.0. This spike uses 2.2.0, which p2panda-core uses. Keys cross as bytes. The check is not cross-tested against 3.0.0.
- The range protocol is my own simulation. It runs in memory and has no network component.
- LogSync over the iroh connection was not attempted.

## Blockers

None. The LogSync-over-iroh run was skipped for time.

## Recommendation

1. **Header.** Adopt p2panda's header with `ChainExt`, at seq base 0. Build and decode events from bytes only. Add the checks in `check_received`: MAX_AFTER, unknown keys, canonical re-encode. Do not call `Header::verify`, `validate_header`, or cached `hash()` on in-memory headers.
2. **Sync.** Use our own range reconciliation with K=16, T=16. With 2 new events it needs 7 to 8 flights at 1,000 to 10,000 chains, and about 6 to 7 KB. LogSync is cheaper in bytes in two cases: below about 40 chains, and when new events are many relative to chains (200 new events on 1,000 chains). Its advantage is 2 flights. If one protocol must be chosen, choose range reconciliation. Its cost tracks the difference, while LogSync's cost tracks the chain count whatever changed.
3. **Dialing.** Ticket-only dialing with Minimal works. Carry addresses in tickets, and treat a stale address as a timeout. Relay-only dialing works with a custom `RelayMode` and no n0 service, so invites can fall back to the home relay.
