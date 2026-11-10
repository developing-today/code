# S7: range reconciliation on real stores, over iroh

## Question

Task 04 gates on S7: does the range scheme S1 simulated work on real redb stores over iroh, with
real signed events, and how does it compare with LogSync? The brief also asks for four
"what could be missed" checks (a) to (d), and for a K and T recommendation on real stores.

## Verdicts

1. **Range sync over iroh on real stores: pass.** 1,000 chains, 2 new events: 7 flights and
   6,554 control bytes (S1 simulated 7 flights and 6,198 bytes; the difference is most likely the
   key width, see Deviations). 10,000 chains: 8 flights and 7,422 bytes. Every run in the matrices and
   sweeps is exact: both stores equal afterwards, no duplicate inserts, and each side learned the
   true difference. The wire run matches the in-memory run byte for byte, plus 4 bytes of frame
   length per message.
2. **Crossover against LogSync (chains x new events -> cheaper method).** In control bytes, range
   (K=16, T=16) is cheaper up to about N = 33 new events at 1,000 chains, and up to about N = 360 at
   10,000 chains (interpolated between measured points). Above that, LogSync is cheaper.
   LogSync is cheaper in flights everywhere (2 against 7 to 8; the LogSync count is S1's). With equal stores, range costs 33
   bytes per sync, against 76 KB (1,000 chains) or 760 KB (10,000 chains) for LogSync's Have
   exchange, which S1 measured and which does not depend on N. LogSync was not run; see "Not
   tested".
3. **K and T on real stores: keep K=16, T=16 as the default.** It is the flight-minimal setting in
   the grid (7 flights at 1,000 chains, 8 at 10,000). For bulk catch-up, where bytes matter more
   than round trips, use K=4 with T=8: control bytes fall 28 to 33 percent at N=200, for 4 more
   flights. Avoid K=2 (15 to 18 flights). K=32 costs 7 flights, the same as K=16, but 39 percent
   more bytes. At K=16, changing T from 16 to 8 gains at most 4 percent, for one more flight.
4. **(a) sequence gap: pass.** Range sync moves exactly the one missing event. A height-only
   comparison (largest seq per author) sees no difference at all: 0 of 1,000 authors differ.
5. **(b) XOR cancellation: partly.** The count check catches unequal sizes (b1, 4 flights).
   It does not catch an equal-size XOR collision (b2): both sides then skip and 4 events are
   missed. With 128-bit fingerprints this needs about 2^-128 luck per compared range, or a crafted
   4-way XOR collision (about 2^43 signed events by a generalized-birthday estimate, not computed).
   I did not construct one with real events.
6. **(c) bounds at keys with 0x00 bytes: pass.** Keys contain zero bytes in their seq field
   (every seq below 256), and 17 of 50 authors have a zero byte in the public key. In-memory and
   wire runs learn the exact difference for K/T pairs from (2,1) to (32,16).
7. **(d) interrupted session, restarted from scratch: pass.** A cut during the data phase left 96
   of 180 events committed, all in whole batches of 32. The restart sent only the remaining 84,
   with 0 duplicates. A cut during reconciliation committed nothing, and the restart completed.

## Setup

- Toolchain: rustup 1.97.0 (`cargo 1.97.0`, `rustc 1.97.0`). Shared target dir
  `/home/user/work/spikes/target`. Standalone crate with an empty `[workspace]`, `Cargo.lock`
  copied from `/home/user/work/spikes/Cargo.lock`.
- Pins, from `/home/user/work/spikes/Cargo.toml`: p2panda-core at git rev
  `3e84cac48bba05c6857cce2ec433862b97f8d53c` (0.7.1), iroh 1.3.0, redb =4.3.0, blake3 1.8.7 (lock).
  p2panda-core's `test_utils` feature is not enabled. Without it, `Header::verify` returns true in
  this build (the trap in the brief). The code never calls `verify`. Verification goes through
  `AnyHeader::decode`.
- Host: AMD Ryzen 7 7840U (16 threads), ext4 on NVMe, the same host S6 used. Each redb commit
  fsyncs.
- Events: `chain::chain_events`. One ed25519 key per device chain, derived from BLAKE3 of the chain
  index. 50 events per chain, as in S1. Header: p2panda `Header` with `ChainExt` (kind
  `group.policy`, domain `id.events.v1`, no `after`, no group). Backlink: the previous header's
  hash. Body: 256 bytes from BLAKE3 XOF over (author, seq). Headers are 228 bytes, so one record is
  486 bytes, or 490 bytes on the wire with its frame. The event ID is BLAKE3 over the header bytes,
  which is p2panda's operation hash. All 550,000 generated events pass `AnyHeader::decode` (signature
  included), a canonical re-encode, and a body hash check.
- Store: S6's redb layout (`/home/user/work/spikes-s6/src/redb_store.rs`). Table `author_seq`:
  author (32 B) || seq (8 B, big-endian) -> event ID. Table `events`: ID -> u16 header length,
  header, body. One write transaction per batch, and the (author, seq) check runs inside it.
  p2panda's seq is u32, widened to S6's 8-byte field.
- **Key encoding:** fixed-width 40-byte keys, not tuple-encoded. The NUL-terminator issue in
  `pkgs/id/src/tuple.rs` does not apply to these keys.
- Range scheme: S1's `range_sim.rs`, ported: K-way splits, a 16-byte XOR fingerprint of the
  truncated IDs plus a count, leaves of at most T items send their IDs, shortest separating prefixes
  as bounds, and merged skip runs.
- Wire, protocol `/id-sync/1` on one bidirectional stream. Flights alternate as in S1's
  `reconcile`. Each message is a u32-length frame. The first message is the dialer's full-range
  fingerprint. A message with only Skip entries ends the loop. Data phase: after reconciliation both
  sides hold the symmetric difference, so neither needs to ask. Each side sends the events it holds
  that the peer lacks, in key order: a count frame, then one frame per record. Both sides send at
  once. The receiver verifies every event, rejects any it did not expect, and commits every `batch`
  (256) events in one transaction.
- Transport: two endpoints on localhost, `presets::Minimal`, `RelayMode::Disabled`,
  `clear_ip_transports`, dialled with `EndpointAddr::new(id).with_ip_addr(...)`. No relay, no
  address lookup.

**Measurements.** Control bytes: all reconciliation frames, both sides, including the 4-byte
length. Data bytes: the count frames and the record frames, both sides. Flights: reconciliation
messages written, both sides, which is S1's convention (a round trip is two flights). The data
phase is one concurrent step and is not counted. QUIC bytes: `udp_tx + udp_rx` on the dialer's
connection, including the handshake. Wall: from `connect` to both sessions finishing, with indexes
already built. It includes the handshake and the commits. Commit: summed redb commit time, both
sides. Index: both sides' index builds from the store, outside wall. Wall, commit, index and QUIC
bytes are medians of 5 runs. Control, data and flights are deterministic: the harness asserts they
are identical across repeats.

**Run hygiene.** Each run copies the base store to two files, adds N events to the dialer's copy,
and fsyncs both copies before the timer starts. An earlier version did not do this. A commit's fsync
then waited for the copies' writeback (about 540 MB), and the 10,000-chain medians were 59, 106 and
104 ms for N=2, 20 and 200. After the fix they are 33, 48 and 85 ms. The wire run is checked
against `range::simulate` on every run.

## Commands

```
export CARGO="$(rustup which --toolchain 1.97.0 cargo)" RUSTC="$(rustup which --toolchain 1.97.0 rustc)"
export CARGO_TARGET_DIR=/home/user/work/spikes/target
cargo test --lib                       # 5 passed
cargo build --release --bins
B=$CARGO_TARGET_DIR/release
$B/gen data 1000 ; $B/gen data 10000   # base stores, every event verified
$B/bench matrix data 1000 5 16 16 0,2,20,200   > logs/matrix-1k-k16-t16.txt
$B/bench matrix data 10000 5 16 16 0,2,20,200  > logs/matrix-10k-k16-t16.txt
$B/bench matrix data 1000 1 16 16 30,40,50,75,100 > logs/crossover-1k.txt
$B/bench matrix data 10000 1 16 16 400,800,1600   > logs/crossover-10k.txt
$B/bench sweep data 1000 2,4,8,16,32 4,8,16,32 2,20,200 > logs/sweep-1k.txt
$B/bench sweep data 10000 4,8,16 4,8,16 2,20,200 > logs/sweep-10k.txt
$B/scenarios run/scen data/base-1000.redb > logs/scenarios.txt
```

## Results

### 1. Correctness and S1 parity

Matrix at 1,000 chains (K=16, T=16, batch 256, 5 runs). All 20 runs passed every check.

| N | flights (A+B) | control B | data B | QUIC B | wall ms, median (min-max) | commit ms | index ms (both) |
|---|---|---|---|---|---|---|---|
| 0 | 2 (1+1) | 33 | 16 | 21,618 | 1.8 (1.7-2.1) | 0.0 | 24.0 |
| 2 | 7 (4+3) | 6,554 | 996 | 29,586 | 10.2 (6.7-13.0) | 6.7 | 25.4 |
| 20 | 7 (4+3) | 50,939 | 9,816 | 84,332 | 14.3 (10.5-15.3) | 4.3 | 26.2 |
| 200 | 7 (4+3) | 354,280 | 98,016 | 489,041 | 34.8 (29.9-41.1) | 15.3 | 21.1 |

Matrix at 10,000 chains (same settings, 5 runs). All 20 runs passed.

| N | flights (A+B) | control B | data B | QUIC B | wall ms, median (min-max) | commit ms | index ms (both) |
|---|---|---|---|---|---|---|---|
| 0 | 2 (1+1) | 33 | 16 | 21,557 | 3.8 (3.3-4.7) | 0.0 | 154.0 |
| 2 | 8 (4+4) | 7,422 | 996 | 30,562 | 32.5 (27.2-34.6) | 4.3 | 152.6 |
| 20 | 8 (4+4) | 63,856 | 9,816 | 97,797 | 48.0 (39.2-73.4) | 7.0 | 182.5 |
| 200 | 8 (4+4) | 462,479 | 98,016 | 599,552 | 84.7 (61.8-120.0) | 21.5 | 167.6 |

**S1 parity.** S1's simulation at K=16, T=16 gave 6,198 bytes and 7 flights at 1,000 chains, and
6,978 bytes and 8 flights at 10,000 chains. The payload here is 6,526 and 7,390 bytes. The extra
+328 and +412 bytes are consistent with the 4 extra key bytes on same-author bounds (about 82 of
them at 1,000 chains). I inferred this, I did not isolate it. Flight counts match S1 exactly.

**Data bytes.** Each event is 490 bytes on the wire, and the 16 bytes of count frames at N=0 are
constant. At N=200, control is 3.6 times the data at 1,000 chains and 4.7 times at 10,000. The
leaf ID lists are the likely cause, since each mismatched leaf sends T IDs per side. I did not
instrument the splits to confirm.

### 2. Crossover against LogSync

LogSync's control bytes come from S1's CBOR measurement of its Have, PreSync and Done messages
for the same chain counts: 76,071 bytes at 1,000 chains and 760,071 bytes at 10,000. They depend
only on the chain count, not on N. Both methods move the same N events, so only control bytes
are compared. Range numbers are control bytes from this spike, with K=16 and T=16.

| chains | N | range control B | LogSync control B (S1) | cheaper in bytes | flights, range / LogSync |
|---|---|---|---|---|---|
| 1,000 | 0 | 33 | 76,071 | range | 2 / 2 |
| 1,000 | 2 | 6,554 | 76,071 | range | 7 / 2 |
| 1,000 | 20 | 50,939 | 76,071 | range | 7 / 2 |
| 1,000 | 30 | 69,314 | 76,071 | range | 7 / 2 |
| 1,000 | 40 | 90,654 | 76,071 | LogSync | 7 / 2 |
| 1,000 | 50 | 107,390 | 76,071 | LogSync | 7 / 2 |
| 1,000 | 100 | 200,969 | 76,071 | LogSync | 7 / 2 |
| 1,000 | 200 | 354,280 | 76,071 | LogSync | 7 / 2 |
| 10,000 | 0 | 33 | 760,071 | range | 2 / 2 |
| 10,000 | 2 | 7,422 | 760,071 | range | 8 / 2 |
| 10,000 | 20 | 63,856 | 760,071 | range | 8 / 2 |
| 10,000 | 200 | 462,479 | 760,071 | range | 8 / 2 |
| 10,000 | 400 | 828,508 | 760,071 | LogSync | 8 / 2 |
| 10,000 | 800 | 1,465,422 | 760,071 | LogSync | 8 / 2 |
| 10,000 | 1,600 | 2,598,113 | 760,071 | LogSync | 8 / 2 |

Crossover N, by linear interpolation between measured points: about 33 at 1,000 chains (between 30
and 40), and about 360 at 10,000 chains (between 200 and 400). Both are about 3.5 percent of the
chain count. This rests on two points, so treat it as a rough rule.

The LogSync flight count of 2 before data is S1's figure. I did not run LogSync, so the N=0 row
for LogSync is inferred: it sends its Have messages whatever the diff. Latency is flights times
RTT. I did not measure RTT: everything here is localhost.

### 3. K and T sweeps (real stores, one run per row, bytes deterministic)

1,000 chains, control bytes and flights (both sides):

| K | T | flights (same for every N) | control N=2 | control N=20 | control N=200 |
|---|---|---|---|---|---|
| 2 | 4 | 18 | 4,813 | 35,312 | 263,470 |
| 2 | 16 | 16 | 5,247 | 41,671 | 324,907 |
| 4 | 8 | 11 | 3,912 | 31,777 | 236,741 |
| 4 | 16 | 10 | 4,551 | 37,821 | 296,377 |
| 8 | 8 | 9 | 4,815 | 36,848 | 272,688 |
| 8 | 16 | 8 | 4,996 | 40,656 | 308,627 |
| 16 | 8 | 8 | 6,415 | 49,604 | 340,920 |
| **16** | **16** | **7** | **6,554** | **50,939** | **354,280** |
| 32 | 16 | 7 | 9,775 | 73,383 | 492,245 |

10,000 chains, a subset (flights are for N=2, 20, 200 in order):

| K | T | flights | control N=2 | control N=20 | control N=200 |
|---|---|---|---|---|---|
| 4 | 4 | 13 | 4,545 | 41,282 | 315,259 |
| 4 | 8 | 12 | 4,672 | 43,503 | 334,836 |
| 8 | 8 | 10 | 5,479 | 47,319 | 356,764 |
| 16 | 4 | 9 | 7,343 | 63,349 | 458,335 |
| **16** | **16** | **8** | **7,422** | **63,856** | **462,479** |

All 60 rows of the 1,000-chain sweep and all 27 rows of the 10,000-chain sweep passed their
checks.

**Recommendation.**
- **Default: K=16, T=16.** It is the lowest-flight setting in the grid, and it matches S1. Its
  bytes are 1.5 to 1.7 times the grid's byte minimum (K=4, T=8) at N of 2 to 200, in exchange for
  4 fewer flights.
- **Bulk catch-up: K=4, T=8.** It saves 28 to 33 percent of control bytes at N=200 (33 percent at
  1,000 chains, 28 percent at 10,000), for 4 more flights. Use it when bytes matter more than
  latency.
- **Avoid K=2** (15 to 18 flights for no byte saving over K=4) **and K=32** (same flights as K=16,
  more bytes).
- **T:** at K=16, T=8 gains at most 4 percent and adds a flight, so keep T=16. At K=4, a smaller T
  saves bytes and adds flights, so choose T by the bytes-or-flights trade.

### 4. What could be missed

Each check ran in `logs/scenarios.txt`, with `failures: 0`.

- **(a) Sequence gap.** On the acceptor, chain 7 lacks seq 20 but has 21 to 49. Before the sync the
  store audit reports one gap. The height view (largest seq per author) shows no difference for any
  author: 0 of 1,000. The range sync moves exactly one event, with 7 flights and 3,749 control bytes.
  Both stores are equal afterwards, and chain 7 is contiguous with valid backlinks. Range over keys
  finds interior gaps. Height-based comparison cannot.
- **(b) XOR cancellation, in memory on crafted IDs.** Real BLAKE3 IDs cannot be steered this way, so
  these runs use synthetic 32-byte IDs, and only the 16-byte truncations enter the fingerprint.
  - b1, unequal sizes: A has 3 items, B has 2, with equal fingerprints. The count check catches it.
    Both sides learn the true 5-item difference in 4 flights.
  - b2, equal sizes: A has 2 items and B has 2, with equal fingerprints. Both sides skip after 2
    flights, learn nothing, and the 4 events are missed. The count check cannot help here.
  - Real events: the chance of a match is about 2^-128 per compared range. Building one needs a
    4-way XOR collision, about 2^43 signed events by a generalized-birthday estimate. Not computed.
- **(c) Bounds at keys with 0x00 bytes.** 50 authors: 10 with zero bytes at chosen public-key
  positions (ground for them), 7 more by chance, and 33 others without a zero byte. Every key also
  has zero bytes in its seq field. In-memory runs with (K, T) of (2,1), (2,4), (4,8), (16,16) and
  (32,16) learn the exact difference. The wire sync of 20 new events is exact, with 20 of 20 inserted,
  equal stores, and valid links. Bounds here are fixed-width prefixes of 40-byte keys. Whole keys
  contain zero bytes for every author, and prefixes such as `00 00 xx` arise where the two
  `00 00` authors meet other keys. I did not count the bounds.
- **(d) Interrupted session, restarted from scratch.** Chains 0, 16, ..., 944 each got 3 new events,
  for 180 in all, with batches of 32.
  - d1: the dialer was cut after 100 records. The acceptor's commits are all whole batches: 96
    events, not 180. The dialer saw "cut before record 101", and the acceptor saw "connection lost:
    closed by peer: cut (code 1)". The store has no gaps and no bad links. The restart's
    reconciliation found exactly 84 missing events, learned the same set on both sides, inserted 84,
    and recorded 0 duplicates. Stores are equal and links are valid.
  - d2: the dialer was cut after one reconciliation message. Nothing was committed. The restart
    inserted all 180 events, with 0 duplicates.

## Deviations from S1

- **Key width.** S6's layout uses an 8-byte big-endian seq, and p2panda's header seq is u32. S1
  used 4 bytes. Each same-author bound is 4 bytes longer. That is consistent with the +328 and +412
  bytes above, but I inferred it and did not isolate it.
- **Split width.** A split uses `min(K, n)` children. When a mismatched range starts at key 0 and
  has fewer than K items, S1's code computes a bound at position 0 and panics. I read this from
  the code and did not run it. S1's runs did not reach that case, and this port avoids it.
- **Wire form.** S1's `encoded_len` is the wire size. Each message is that encoding inside a
  4-byte length frame. The harness checks this on every run.
- **No requests in the data phase.** Each side derives what it must send and expect from the
  symmetric difference, and the count frame checks that the two agree.

## Not tested, and why

- **LogSync over iroh.** Not run, and the S1 numbers are reused: 76,071 and 760,071 bytes of
  CBOR control. S1's own note says it measured only the Have sizes, from real p2panda-sync types,
  and did not run a session. The time budget did not allow the SQLite store glue plus the iroh
  adapter. In S1's model, LogSync's control is independent of N. The equal-store cost for LogSync
  is inferred from the same Have sizes, since LogSync always sends them.
- **Real network.** Everything runs on localhost with `RelayMode::Disabled`. RTT is near zero, so
  the flights-times-RTT comparison is not measured.
- **Bootstrap (an empty store receiving everything).** Not run. It is the case where control
  overhead per event matters most.
- **Scale beyond 10,000 chains and 1,600 new events.** Not run.
- **Gating, blobs, and the orderer.** Out of scope for S7. The store is set-based and accepts a gap
  on insert. Waiting for missing backlinks is task 02's job.
- **Power loss.** The interruption is a connection cut, not a process kill. Power-loss durability
  was not tested, and S6 covered only SIGKILL.
- **Adversarial XOR forgery.** The fingerprint is linear over GF(2), so a peer with enough of its own
  events could craft a matching fingerprint. A dishonest peer can always answer Skip, so this adds
  nothing beyond lying. I did not analyse it further.
- **Record bytes in the events table.** Equality was checked on (key, ID) rows. Records are fixed by
  their verified IDs and body hashes, but I did not compare their bytes directly.
- **16-byte IDs in leaves.** This would halve the leaf control cost. Not tested.
- **Index persistence.** Every session rebuilds its index from the store (medians 21 to 26 ms at
  1,000 chains, 153 to 183 ms at 10,000, both sides). A node would keep the index up to date, so this cost
  is outside the sync figures.
- **Wall time at 10,000 chains includes an O(n) scan.** At N=2, the non-commit part of the median
  wall time is about 28 ms at 10,000 chains, against about 3.5 ms at 1,000. I attribute the gap to
  the offers scan over 500,000 index rows (HashSet lookups). I did not isolate it. N=0 has no
  learned IDs and takes 3.8 ms, which fits the scan being the cost.

## Surprises

- Steady-state sync is almost free in range form: 33 bytes, against 76 KB (1,000 chains) or 760 KB
  (10,000 chains) for LogSync's Have exchange.
- The N=0 sync (handshake plus two tiny messages) moves 21.6 KB on the wire. That overhead dominates
  a small sync: N=2 moves 29.6 KB, against 6.6 KB of control and 1 KB of data.
- Control overhead is large for dense diffs: 3.6 to 4.7 times the data at N=200.
- At 10,000 chains and N=200, K=4, T=4 saves 32 percent of control bytes against K=16, T=16, for
  5 more flights. K=4, T=8 saves 28 percent, for 4 more.

## Files

- `src/chain.rs`: events, verification (`AnyHeader::decode`), records. Includes a test that flips a
  signature bit and sees `InvalidSignature`.
- `src/store.rs`: redb store with the S6 layout.
- `src/range.rs`: S1's scheme over the store index, wire encoding, in-memory simulation, and unit
  tests for NUL-heavy separators, exact learning across K and T, and the wire round trip.
- `src/session.rs`: iroh session, two-endpoint runner, cut injection.
- `src/bin/gen.rs`, `src/bin/bench.rs`, `src/bin/scenarios.rs`: base-store generation, matrix and
  sweep, and checks (a) to (d).
- `logs/`: matrix, crossover, sweep and scenario output (all under 4 KB each).
- The base stores (`data/`, about 670 MB) and the scratch `run/` directory are removed after the
  runs. `gen` rebuilds the base in about a minute.
