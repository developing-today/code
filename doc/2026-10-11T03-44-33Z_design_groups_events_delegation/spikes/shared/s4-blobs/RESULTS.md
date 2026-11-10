# S4: payload storage with iroh-blobs and authorised fetch

**Question.** Can a p2panda body hash be the iroh-blobs hash? Can a blob be fetched by hash, without a ticket? Can a provider gate `get` and `get_many` per requester with `EventMask`? Does verified streaming reject corrupted bytes? What are the storage and release limits?

## Setup

- Member `s4-blobs`, one binary (`src/main.rs`, parts a to e). No dependency added. Every build used `--locked`.
- Toolchain: rustup `1.97.0` (rustc and cargo 1.97.0).
- p2panda-core `0.7.1` at git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`. It compiled at that rev, so no crates.io fallback was needed.
- iroh `1.3.0` (default `tls-ring`), iroh-blobs `=0.103.1` (default `fs-store`, `rpc`), bao-tree `0.16.1` (transitive), blake3 `1.8.7`.
- Provider and requester endpoints: `presets::Minimal` (no relay, no discovery), on localhost.

## Commands

```
"$CARGO" build -p s4-blobs --locked
"$CARGO" run -p s4-blobs --locked > s4-blobs/run-debug.log             # debug build
"$CARGO" run -p s4-blobs --release --locked > s4-blobs/run-release.log  # release build
curl -sS https://index.crates.io/ir/oh/iroh-blobs                      # yanked flags
```

Both final runs end with `ALL PARTS PASSED`.

## Measurements

**(a) Hash equivalence: equal at every size.** p2panda `Body::hash()` (which is `Hash::digest`, i.e. `blake3::hash`) matched the hash from `MemStore::add_slice` and from `iroh_blobs::Hash::new`.

| Body size | Equal | Hash, first 16 hex digits |
|---|---|---|
| 0 B | yes | af1349b9f5f9a1a6 (BLAKE3 of empty input) |
| 1 KiB | yes | e7286ea9aaf93be7 |
| 64 KiB | yes | d0bb951dc42dd03e |
| 1 MiB | yes | e171222f1a7ad0f9 |
| 10 MiB | yes | 8a9354f4e9164f29 |

**(b) Fetch by hash, no ticket, 10 MiB on localhost.** The requester has only the provider's `addr()` and the hash. The Downloader takes provider EndpointIds, so the requester's endpoint has a `MemoryLookup` seeded with that address. Bytes matched, and the body hash equalled the requested hash, in every run.

- Downloader, fresh endpoint per run. Release: 20.2, 17.4, 19.5 ms. Debug: 141.7, 150.7, 145.9 ms.
- No lookup (`connect(addr)`, then `GetRequest::blob(hash)`). Release: 20.9 ms. Debug: 157.5 ms.
- An earlier release run, on a busier machine: 35 to 41 ms.

**(c) Authorised fetch: every row as expected, in debug and release.** Mode: `EventMask { connected: ConnectMode::Notify, get: RequestMode::Intercept, get_many: RequestMode::Intercept, ..EventMask::DEFAULT }`. That is `Intercept`, not `InterceptLog`. Push is disabled (the default), and observe and throttle are off. `GetRequestReceived` carries no peer, so the handler maps connection ID to EndpointId from `ClientConnectedNotify`. It answers `Ok(())` for allow-listed peers and `Err(AbortReason::Permission)` for everyone else, including unknown connections. Allow list: {A}. D is not on it.

| Requester to provider | Served | Requester saw |
|---|---|---|
| A to gated, `get` | yes, bytes match | nothing |
| D to gated, `get` | no | reset code 1 (`ERR_PERMISSION`) |
| A to gated, `get_many` (wire `GetManyRequest`) | yes, bytes match | nothing |
| D to gated, `get_many` | no | reset code 1 |
| D to open provider, `get` / `get_many` | yes / yes | nothing |
| Downloader `get`: A to gated / D to gated | yes / no | nothing / error |

The provider logged six decisions. Every D decision was `Err(Permission)`.

**(d) Verified streaming: every corrupted block is rejected and never held.** A block counts as held only if a full 16 KiB block exports from the requester's store. Every held block was byte-correct.

| Case | Outcome | Held blocks, debug / release |
|---|---|---|
| Control: untouched on-disk FsStore | all bytes served | n/a |
| On-disk data bit flip, block 37 | provider's export fails (`LeafHashMismatch(592)`); requester gets reset code 3 (`ERR_INTERNAL`) | 0 / 18 (blocks 0 to 17) |
| On-disk outboard bit flip, pair for blocks 32 to 63 | provider's export fails (`ParentHashMismatch(TreeNode(767))`); reset code 3 | 0 / 10 |
| Wire bit flip in the last leaf (test provider) | requester: `leaf hash mismatch: 1008`; no reset | 63 / 63 (blocks 0 to 62); block 63 absent |

The on-disk counts depend on timing. An earlier release run, counting from the store's bitfield, held 26 blocks (data flip) and 32 (outboard flip). The wire case was the same in every run.

**(e) Storage.** Debug: MemStore add of 10 MiB 4.7 to 5.2 ms, read 8.2 to 14.6 ms. FsStore add 9.6 to 13.3 ms, read 12.8 to 12.9 ms. Release: MemStore add 3.0 to 3.2 ms, read 4.7 to 8.6 ms. FsStore add 6.3 to 9.9 ms, read 9.2 to 10.5 ms.

FsStore inlines data and outboard up to 16 KiB into `blobs.db`:

- 1 KiB and 16 KiB: no data file.
- 17 KiB: `.data` file, outboard inline.
- 64 KiB and 1 MiB: `.data` file, outboard inline. The 1 MiB outboard is 4,032 B.
- 10 MiB: `.data` of 10,485,760 B and `.obao4` of 40,896 B (639 pairs of 64 B, no header), about 0.39% overhead.

Every size survived close and reopen. `blobs.db` was 49,152 B after small inline blobs and 1,056,768 B after a 16 KiB inline blob. That is redb allocation, not payload size.

**(f) Production warning and yanks.**

- The 0.103.1 README says: "NOTE: this version of iroh-blobs is not yet considered production quality. For now, if you need production quality, use iroh-blobs 0.35." The owner accepted this (design §11.3).
- Live crates.io index, checked 2026-10-11. Yanked: 0.97.0, 0.99.0, 0.100.0, 0.101.0, 0.102.0, 0.103.0, and 0.28.0. Not yanked: 0.98.0, 0.99.1, 0.100.1, 0.101.1, 0.102.1, 0.103.1.
- The research note's "0.99.0 to 0.103.0" overstates the range. Within that range only the `.0` releases are yanked. The pin `=0.103.1` is not yanked.

## Pass or fail per exit criterion

1. **Body hash equals iroh-blobs hash: PASS** (a), at all five sizes.
2. **Fetch by hash without a ticket: PASS, with a caveat.** The requester still needs the provider's EndpointId and an address. The hash alone is not a route (research §7.1).
3. **Provider gating via EventMask: PASS** for `get` and `get_many` on the wire (c). A denial is a stream reset with code 1, not a permission message.
4. **Verified streaming: PASS on the wire** (d). On-disk corruption is caught when the provider exports the bad block.
5. **Limits noted: done** (below).

## Deviations

- Requester endpoints carry a `MemoryLookup`, an in-process address book, because the Downloader takes EndpointIds. The endpoints are still `presets::Minimal`.
- The Downloader splits `get_many` into per-hash `get`s, so `get_many` gating was tested with raw `execute_get_many`.
- The requester-side check in (d) needed a test provider that flips one bit on the wire. That is a proxy, not a network attacker.
- `Cargo.lock` changed twice during the session (141,261, then 148,038, then 148,048 bytes). I did not edit it, and my builds used `--locked`, so another member is the likely source. The pins used here are unchanged.

## Blockers and what I tried

- My first (d) draft assumed an 8-byte outboard header; the file has none. The 1 MiB outboard was inlined under default options, so I used `InlineOptions::NO_INLINE`.
- The first release run failed my (d) check, which required the requester to hold no blocks. Release runs hold verified prefixes, so the check now verifies what is held.
- `export_ranges` returns `Ok` with no data when the blob size is unknown. The scan counts a block only when a full block comes back.
- The first gate log showed per-hash gets for `get_many`, from the Downloader's split. Added raw `execute_get_many`.

## Limits

- Not production quality, per the README. Pin `=0.103.1`.
- No content routing. A requester needs an EndpointId and an address (design §11.5).
- A denied peer still connects. It sees only `stream reset by peer: error 1`. The Downloader reports `Unable to download`, so the code is lost unless the client makes a raw get.
- `observe` is not gated. Its default mode is `None`, so requests are served. This comes from reading `provider/events.rs` and was not tested. Intercept it too if existence or size must stay private.
- Verification granularity is a 16 KiB leaf. One bad byte discards its whole leaf.
- A provider with corrupt stored bytes fails the response with `ERR_INTERNAL` at the bad block. Verified blocks before it may already have arrived and are kept, and their number varies with timing. Whether a re-fetch resumes from them was not tested.
- Only sizes up to 10 MiB were tested. The research note's claim of no size cap is unverified.
- Outboard overhead is about 0.39% at 10 MiB, on disk.

## Recommendation

Adopt iroh-blobs `=0.103.1` for payloads, and use the p2panda body hash as the blob ID directly, with no mapping table.

For gating, keep the tested mask and add:

- the connection-to-peer map, which fails closed (done in the spike),
- a decision on whether `observe` must be gated,
- clients reading reset code 1 through `GetError::iroh_error_code()`, so they can tell denial from absence,
- provider addresses kept in the ticket or home-server layer,
- the blob's `size` from the event checked against limits before any fetch.
