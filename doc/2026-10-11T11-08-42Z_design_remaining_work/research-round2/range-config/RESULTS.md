# Range parameters K and T, and range sync versus LogSync

Date: 2026-10-11. Folder: `/home/user/work/research-round2/range-config/`. Harness: `harness/`, a copy of S7 with the additions listed under "Harness changes". Logs: `harness/logs/`. Labels: **verified** (I saw it in output or read it in the code), **inferred** (reasoned from verified facts), **not checked**.

## Questions (verbatim)

Owner, verbatim:

1. "n6 can these things be configurable or will that cause compat impacts?"
2. "i'm ok with range above crossover but i want to better understand what the tradeoffs are."

The owner's "these things" are K (fan-out) and T (leaf threshold), and "range above crossover" is N5 ("Range sync above about 40 chains; LogSync below (confirm the crossover in S7)"). Index: `/home/user/work/integrated/doc/2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md:93,94`.

Parent's task, as assigned:

1. Read the S7 range code. What does each range message carry (explicit bounds, a per-range mode)? Are K and T wire parameters or local split policy? Does a receiver need the sender's K and T? Can two peers with different K and T converge?
2. Copy the S7 harness. Run the real sync between two stores with (K=16, T=16) on one side and (K=4, T=8) on the other, and with both sides at (K=2, T=4) and at (K=32, T=32). Check that both stores are equal afterwards, with no duplicate inserts and no missed events. Record bytes, flights and time. Small deterministic workload, under about five minutes.
3. Compare LogSync and range on round trips, bytes, CPU, memory, code to maintain, failure modes, many chains, few new events, equal stores and large diffs, using the S1 and S7 numbers. Propose the selection rule for "range above the crossover", and say what the initiator must know (chain count, estimate of the difference, last-sync time).
4. Protocol versioning: what a change to K or T requires; what a change to the fingerprint function or the bounds encoding requires (a version bump or not); what the protocol ID string should carry.

## Short answers

**Owner 1 (configurable, compat?).** Yes, K and T can be per-node settings with no wire impact. They are local split policy: they never appear on the wire, and a node uses only its own values when it answers a message (verified: `range.rs:96,110,115`, `session.rs:119`). Pairs with different values converge exactly: K16/T16 against K4/T8, and the other pairs in Table O2, end with equal stores, no duplicate inserts, no missed events, and a wire run that matches the in-memory run byte for byte (verified, 9 wire configurations). The risk is validation, not the wire. K must be at least 2: `range.rs:97` panics otherwise (verified, K=1). T must be at least 1: with T=0, two single keys that differ never terminate (verified, panic after 10,000 flights). Changing the fingerprint, the bounds encoding, the key layout, the framing, the leaf ID width or the mode bytes is a wire change and needs a new protocol ID (Q4).

**Owner 2 (tradeoffs of range above the crossover).** Measured, the crossover is not 40 chains. S1's 40-chain figure is for 2 new events only (S1 RESULTS line 85). In control bytes, range overtakes LogSync at about 3.5 percent of the chain count: 33 new events at 1,000 chains, about 360 at 10,000 (S7 RESULTS lines 173–175). Above that, range's control grows at 1.6 to 2.5 KB per new event, while LogSync's stays at 76 bytes per chain. Range/LogSync control ratio at 1,000 chains: 0.67 at 2 percent of chains, 1.19 at 4 percent, 4.66 at 20 percent (Table O3). What range costs, beyond bytes: 7 to 9 flights against 2 for LogSync, which is 2.5 to 3.5 more round trips before data starts (inferred from alternating flights, RTT not measured). Per session it holds an in-memory index of every event: 88 bytes per event, measured RSS 88 to 92 bytes per event (verified, `mixed frame`), and an index build of 21–26 ms at 1,000 chains and 153–183 ms at 10,000 (S7 RESULTS lines 292–293). What range buys: a flat cost when the stores are equal (33 bytes, against 76 KB for LogSync), detection of interior sequence gaps (LogSync cannot see them: verified by `compare_logs`), and detection of forks (LogSync cannot see those either). Two failure modes must be handled before range is relied on. A single conflicting event aborts the batch it sits in, and the 20 unrelated events in that batch were lost (verified, E3d). A single flight above about 524,288 IDs exceeds the 16 MiB frame limit (verified in memory), so a bootstrap of a larger store fails in either direction.

**Q1 (what a range message carries; K and T).** Each message is a list of entries. Each entry carries an explicit upper bound, a key prefix with a one-byte length (zero means +infinity), and a mode: skip, fingerprint (a 16-byte XOR and a count), or an ID list (32-byte IDs, with a reply flag) (verified: `range.rs:18–30`, `range.rs:173–201`). The lower bound is not sent: it is the previous entry's upper bound (`range.rs:99,143`). K and T appear only inside `respond` (`range.rs:96–146`), never in `encode` or `decode` (`range.rs:173–261`). A receiver needs nothing from the sender beyond the message itself. Different K and T on the two peers converge (verified on real stores).

**Q2 (exactness with mixed and extreme K/T).** All nine wire configurations passed, including the calibration row. Each store ended with the other's events, there were zero duplicate inserts, each side learned exactly the symmetric difference, and the wire flights and bytes matched the in-memory run exactly (Table O2, verified). Control bytes at 1,000 chains with 20 new events on each side: K2/T4 66,001 (18 flights); K4/T8 56,761 (11); K16/T16 89,698 (7); K32/T32 125,180 (7). Mixed: K16/T16 to K4/T8 72,062 (9 flights); K4/T8 to K16/T16 79,200 (8). Among these, K4/T8 has the fewest bytes and fewer flights than K2/T4, which agrees with S7's recommendation. Wall times are indicative only, because other agents share this host.

**Q3 (LogSync versus range, selection rule).** Proposed rule, labelled inferred from the measured crossover:
- **Bootstrap** (no completed sync with this peer): LogSync, once a redb `LogStore` adapter exists. It needs 2 flights and about 76 bytes per chain of control. Until then, range with K16/T16, which costs 6.5 percent extra bytes and 4–5 flights, and which fails above about 524,288 events (see Q4 and Table O4).
- **Otherwise**: LogSync when the estimated number of events the two sides disagree on, D, is above about 0.035 times the local chain count C. Range (K16/T16) below that.
- **What the initiator must know**: C (its own chain count, from its index); D (an estimate: events it created since the last sync with this peer, plus events it received from this peer since then, which is a lower bound because the peer's new events are unknown before the first exchange); and T_last, the time of the last completed sync with this peer, which tells bootstrap from steady state. A margin on D is advisable.

**Q4 (versioning).** Table O5 has the full list. A change to K or T needs no protocol change, only validation (K at least 2, T at least 1). A change to the fingerprint function, the bounds encoding, the key layout, the framing, the leaf ID width, the mode bytes or the frame-size limit is wire-visible and needs a new ALPN, `/id-sync/2`. The separator choice and the batch size are local. The protocol ID should carry the wire-format version only, never K, T or batch. The message has no in-band version field: the first bytes are the entry count (`range.rs:175`), so the ALPN is the only negotiation point.

## Evidence

### E1. Environment and commands

- Toolchain: `rustup which --toolchain 1.97.0 cargo` (`/home/user/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu/bin/cargo`). Pinned p2panda rev `3e84cac48bba05c6857cce2ec433862b97f8d53c` (verified: `/home/user/work/spikes/Cargo.toml:11–16`). `CARGO_TARGET_DIR=/home/user/work/spikes/target`, as the brief says for pinned crates. This is the only write outside my folder, and it is a build cache.
- Build: `cargo build --release --bin mixed` in `harness/`. Succeeded, no warnings in the tail of the output.
- Wire runs under `unshare -Urn sh -c "ip link set lo up && …"`, because iroh binds to 127.0.0.1 (verified: loopback comes up in the namespace).
- In-memory runs: `mixed tzero`, `mixed gap`, `mixed frame 50000|524000|550000`, `mixed frame-empty 50000|550000`.
- Wire runs: `mixed pairs data 1000 3`, `mixed bootstrap data 1000`, `mixed fork data 1000`.
- Base store: `data/base-1000.redb`, built by the same generator as S7 (`chain.rs:63–79`): 1,000 chains of 50 events, 50,000 events. Deleted after the runs.
- All logs are in `harness/logs/` (`pairs-1k.txt`, `bootstrap-1k.txt`, `fork-1k.txt`, `tzero.txt`, `gap.txt`, `frame.txt`, `frame-empty.txt`). The stderr files are empty except `tzero.stderr`, which holds the expected panic messages.
- Total runtime of all runs: under two minutes.

### E2. Code reading for Q1 (S7 original, `/home/user/work/spikes-next/s7-range-real/src/`)

| Item | Where | Note |
|---|---|---|
| Modes: Skip, Fp{fp,count}, Ids{ids,reply} | `range.rs:18–23` | verified |
| Entry: upper bound, mode | `range.rs:25–30` | verified |
| Fingerprint: XOR of 16-byte truncations (implicit, by `zip`) | `range.rs:45–58`, zip at `:52` | verified |
| Counts and fingerprint over a prefix-partitioned index | `range.rs:69–79` | verified |
| Separator: shortest prefix that sorts above the previous key | `range.rs:82–86` | verified |
| `respond(msg, thresh=T, k=K)`: the only place K and T are used | `range.rs:96–146`, K assert `:97`, leaf rule `:110`, split `:113–128` | verified |
| Leaf reply: the responder replies with its own IDs for the range, unbounded by T | `range.rs:131–141`, IDs at `:135–137` | verified |
| Lower bound is the previous upper bound | `range.rs:99`, `:143` | verified |
| Wire encoding: entry count, bound (length byte, 0 = +inf), mode byte, payload | `range.rs:173–201` (bound length `:180`, modes `:185–197`) | verified |
| Decode rejects unknown mode bytes and trailing bytes | `range.rs:233–261` (`:255`, `:259`) | verified |
| Session: one frame per flight (u32 BE length), `write_frame` cap | `session.rs:67–72`, cap check `:68`, `flight` `:85–93` | verified |
| Session: the receiver calls `respond(&incoming, p.t, p.k)` with its own values | `session.rs:119` | verified |
| Session: the loop ends on an all-skip message | `session.rs:116–124` | verified |
| Protocol ID | `session.rs:24`: `b"/id-sync/1"`; `MAX_FRAME` at `:25` | verified |
| `sync_pair` takes one `Params` and gives it to both sides | `session.rs:224` (acceptor `:235`, dialer `:242`) | verified; this is why the harness needed `sync_pair_mixed` |
| Data phase: count frame, then one frame per record, both sides at once | `session.rs:127–192`, count check `:153`, expected and duplicate checks `:162–163` | verified |
| Store: key = author (32 B) ‖ seq (8 B, BE), `KEY_LEN` 40 | `store.rs:15`, `key_of` `:23–28` | verified |
| Store: a fork (same key, other ID) fails the batch | `store.rs:55–78`, bail at `:68` | verified |
| Store: `open` creates no tables; a read of an empty store fails | `store.rs:48–51` | verified (defect, E3e) |

What a message carries: explicit bounds (one per entry), and a mode per range. K and T are not carried, so they are local policy. A receiver needs nothing about the sender's K or T; it applies its own split and leaf rules to its own keys. Convergence rests on the bounds being value-space points: both sides apply the same comparison to their own keys (`range.rs:69–71`), so the partition of the key space is the same on both sides, whatever K or T each uses.

Termination (inferred from the code, not proved): a split by a party only happens when its own count exceeds T, and each child holds fewer of that party's keys. So the number of splits is bounded by the key count, provided T is at least 1. That is why T=0 is unsafe (E3b).

### E3. Test results

**(a) Calibration: the copy reproduces S7 exactly (verified).** One-sided 40 new events, K16/T16, 1,000 chains: 7 flights, 90,654 control bytes, 19,616 data bytes. S7's N=40 row: 7 flights, 90,654 (`S7 logs/crossover-1k.txt` line 6; RESULTS line 161). Data bytes are 40 × 490 + 16 (count frames), matching S7's 490-byte record (S7 RESULTS line 59).

**(b) Pairs with separate parameters on both sides (verified).** 1,000 chains; the dialer adds 20 events on chains 0, 50, …, 950; the acceptor adds 20 on chains 25, 75, …, 975 (disjoint, so no fork). Each row: 3 runs, identical bytes and flights in each run.

| Case | Dialer K,T | Acceptor K,T | Flights A+B | Control B | Data B | Checks |
|---|---|---|---|---|---|---|
| Calibration: 40 new on the dialer only | 16,16 | 16,16 | 7 (4+3) | 90,654 | 19,616 | exact, wire = in-memory |
| S7 default on both sides | 16,16 | 16,16 | 7 (4+3) | 89,698 | 19,616 | exact, wire = in-memory |
| Mixed | 16,16 | 4,8 | 9 (5+4) | 72,062 | 19,616 | exact, wire = in-memory |
| Mixed | 4,8 | 16,16 | 8 (4+4) | 79,200 | 19,616 | exact, wire = in-memory |
| Both K2 T4 | 2,4 | 2,4 | 18 (9+9) | 66,001 | 19,616 | exact, wire = in-memory |
| Both K4 T8 (S7's bulk setting) | 4,8 | 4,8 | 11 (6+5) | 56,761 | 19,616 | exact, wire = in-memory |
| Both K32 T32 | 32,32 | 32,32 | 7 (4+3) | 125,180 | 19,616 | exact, wire = in-memory |
| Mixed | 2,4 | 32,32 | 8 (4+4) | 99,603 | 19,616 | exact, wire = in-memory |
| Mixed | 32,32 | 2,4 | 9 (5+4) | 100,639 | 19,616 | exact, wire = in-memory |

Checks, in `mixed.rs:147–240`: each side learned the exact symmetric difference (computed from the stores before the sync); inserts equal the number of events the side lacked; zero duplicate inserts; the two stores are equal afterwards; the row count equals the union. All nine rows passed on every run.

Wall-time medians of the final run (ms, shared host, indicative only): 31.5, 23.2, 42.8, 24.0, 30.4, 42.2, 24.4, 26.5, 36.4. Earlier runs of the same harness, before the store fix, gave 8–15 ms for the same cases, so the variance is about 3×.

**(c) Bootstrap: an empty store (verified).** 50,000 events on the full side, one run each.

| Case | Flights A+B | Control B | Data B | Control / data |
|---|---|---|---|---|
| Full dials empty, K16/T16 | 4 (2+2) | 1,600,051 | 24,442,016 | 0.065 |
| Empty dials full, K16/T16 | 5 (3+2) | 1,601,465 | 24,442,016 | 0.066 |
| Full (K4/T8) dials empty (K16/T16) | 4 (2+2) | 1,600,051 | 24,442,016 | 0.065 |
| Empty (K16/T16) dials full (K4/T8) | 5 (3+2) | 1,600,166 | 24,442,016 | 0.065 |

The control is 32 bytes per event: the responder answers with an empty ID list, and the other side replies with every ID in the range (`range.rs:131–137`). That reply is not bounded by T. It is one flight, and the cost is nearly the same whichever side dials. Wall times (8–13 s, single runs) are dominated by 50,000 signature verifications and 196 commits (inferred).

**(d) Fork: a second, different event at one (author, seq) (verified).** Chain 3, seq 25: the acceptor holds a different event with the same key. Rows after the sync, dialer / acceptor:

| Unrelated new events on the dialer | Parameters | Failures, verbatim (abridged) | Rows after, dialer / acceptor |
|---|---|---|---|
| 0 | K16/T16 | dialer: `connection lost: closed by peer: 0`; acceptor: `fork at key f22a…0019` | 50,000 / 50,000 |
| 0 | K4/T8 | dialer: `connection lost`; acceptor: `fork at key …0019` | 50,000 / 50,000 |
| 20 | K16/T16 | both: `fork at key …0019` | 50,020 / 50,000 |
| 20 | K4/T8 | both: `fork at key …0019` | 50,020 / 50,000 |

Reading: the key is `f22a…` followed by `0000000000000019`, which is seq 25 (`store.rs:23–28`). Both sides detect the fork during the data phase. The acceptor always reports `fork at key`; the dialer reports either that or `connection lost`, depending on which error reaches it first. With 20 unrelated events in the same batch (batch size 256, `session.rs:165`), neither side commits any of them, and the sync fails without progress. The stores differ afterwards, as expected. Stores in the no-extra case are unchanged. The fork fails in every run. Later sessions should fail the same way, because the key does not move (inferred; only one session per configuration was run).

**(e) Defect in S7's store: an empty store cannot be read (verified).** Before the fix, `mixed bootstrap` failed with `Error: Table 'author_seq' does not exist`. `Store::open` (`store.rs:48–51`) never creates the tables, and a read of a missing table fails. S7 never ran an empty store, so this was not seen. In my copy, `Store::open` now creates both tables (`harness/src/store.rs:48–59`). The original is not changed.

**(f) Invalid parameters, in memory (verified, `mixed tzero`).**

| Parameters (T, K) | Result |
|---|---|
| T=16, K=16 | ok, 6 flights |
| T=1, K=2 | ok, 12 flights |
| T=0, K=16 (300 keys, 3 missing) | ok, 6 flights |
| T=16, K=1 | panic: `K must be at least 2` (`range.rs:97`) |
| One key each, differing, T=0, K=16 | panic: `reconciliation did not terminate` after 10,000 flights |
| One key each, differing, T=1, K=2 | ok, 4 flights |

T=0 does not always fail: the 300-key case converged. It fails on a single-key mismatch, because a responder with one key in a mismatched range splits into one child, which is the same range again (`range.rs:113–128` with `parts = min(k, 1) = 1`), and the loop never ends. The termination assertion is `range.rs:297` in S7 and `range.rs:327` in my copy's `simulate_mixed`. The conclusion holds: T must be at least 1.

**(g) Frame limit and index memory, in memory (verified, `mixed frame`, `mixed frame-empty`).**

| Events | Leaf reply, bytes (one flight) | Limit, bytes | Over | Index RSS, MiB | RSS bytes per event |
|---|---|---|---|---|---|
| 50,000 | 1,600,006 | 16,777,216 | no | 4.4 | 92 |
| 524,000 | 16,768,006 | 16,777,216 | no | 44.2 | 88 |
| 550,000 | 17,600,006 | 16,777,216 | **yes** | 46.4 | 88 |

The reply is 32·N + 6 bytes at all three points. The limit is therefore crossed at N = 524,288. The enforcing checks are `session.rs:68` (writer) and `session.rs:78` (reader). The empty-dialer variant does not avoid it: its flight 4 (one frame carrying every child's reply) is 1,600,185 bytes at 50,000 and 17,600,201 bytes at 550,000, over the limit. Every reply in a flight goes into one frame (`session.rs:119–121`). The wire run at 550,000 was not done (inferred that it would fail the same way, from the same check).

**(h) LogSync's completeness check, the same function it calls (verified, `mixed gap`).** LogSync decides what to send with `compare_logs(&local, &remote)` (`log_sync.rs:153`), on heights only. The function is `compare_logs`, `p2panda-core/src/logs.rs:110–162`.

| Case (heights, local / remote) | Entries sent for the chain | Why |
|---|---|---|
| Gap at seq 20 on the remote; 49 / 49 | 0 | equal heights skip (`logs.rs:131–133`) |
| Same height, different event at seq 25 (fork); 49 / 49 | 0 | equal heights skip |
| Remote lags; 49 / 30 | 1, range (30, 49] | `logs.rs:149–157` |
| Remote has no entries; 49 / none | 1, range (none, 49] | `logs.rs:121–126` |

So LogSync sends nothing for an interior gap or a fork, and reports no error. The check was on the function with heights supplied by hand, because there is no redb `LogStore` to read them from (not checked: a real LogSync run).

### E4. LogSync code, for Q3 (p2panda checkout at rev 3e84cac)

Paths are relative to `/home/user/.cargo/git/checkouts/p2panda-de6556ee3f16c1a4/3e84cac/`. The crates.io 0.7.1 in `/tmp/research/p2p-sync-wu85cK/` is a different code base (no `api.rs`, `compare` instead of `compare_logs`, a different `Logs` type), so the pins must be read against rev `3e84cac`, not 0.7.1 (verified by `diff -r`).

| Fact | Where | Verified |
|---|---|---|
| Sequence: Have (both sides at once), then PreSync or Done, then operations, then Done | `p2panda-sync/src/protocols/log_sync.rs:120–135` (Have), `158–210` (PreSync or Done, selection `188–196`), `287–411` (operations and Done) | yes |
| Flights before data: 2 (Have, PreSync) | the same | yes (S1 RESULTS line 84 agrees) |
| Heights only: `compare_logs` decides what to send | `log_sync.rs:153`; `p2panda-core/src/logs.rs:110–162` | yes |
| Have holds every log passed in the set (one `get_log_heights` per author) | `p2panda-sync/src/api.rs:573–590` (loop `:582–587`) | yes |
| Operations streamed from the store, one stream item in memory | `api.rs:596–630` (comment `:594`); close on error `:620–627` | yes |
| Dedup buffer of 1,024 hashes | `p2panda-sync/src/dedup.rs:7` | yes |
| Strict state machine: any out-of-order message aborts the session | `log_sync.rs:148–151`, `:245–248`, `:352–354`; tests `:781–884` | yes |
| A log pruned mid-sync gives a short sync, with no error | test `log_sync.rs:888–978`, asserts `:976–977` (250 promised, 249 received) | yes (test assertion; not re-run) |
| Byte and operation counters are `u32` (overflow above 4 GiB per session) | `log_sync.rs:165–166`, `:179–180`, `:474–481` | yes (read); not reached at this scale |
| Store traits: `LogStore` (6 methods) and `OperationStore` | `p2panda-store/src/logs/traits.rs:21–89`; `p2panda-store/src/operations/traits.rs:11` | yes; redb has neither |
| S1 used `SqliteStore`, which implements both | S1 RESULTS line 97 | yes |

### E5. Numbers from S1 and S7

S1 (`/home/user/work/spikes/s1-transport/RESULTS.md`): LogSync's Have is 38 bytes per chain (CBOR), 38,009 bytes at 1,000 chains; PreSync 43 bytes, Done 5 bytes (lines 74–75). Control: 76,071 bytes at 1,000 chains and 760,071 at 10,000 (S7 RESULTS line 151; `bench.rs:24–30` takes these as constants, not re-measured). Crossover at 2 new events: 35–40 chains (line 85). Range's flights: 7–19 (line 84).

S7 (`/home/user/work/spikes-next/s7-range-real/RESULTS.md`, verified):

| Chains, new events | Range control B | Range flights | LogSync control B (S1) | Ratio range/LogSync |
|---|---|---|---|---|
| 1,000, 0 | 33 | 2 | 76,071 | 0.0004 |
| 1,000, 2 | 6,554 | 7 | 76,071 | 0.086 |
| 1,000, 20 | 50,939 | 7 | 76,071 | 0.67 |
| 1,000, 30 | 69,314 | 7 | 76,071 | 0.91 |
| 1,000, 40 | 90,654 | 7 | 76,071 | 1.19 |
| 1,000, 100 | 200,969 | 7 | 76,071 | 2.64 |
| 1,000, 200 | 354,280 | 7 | 76,071 | 4.66 |
| 10,000, 2 | 7,422 | 8 | 760,071 | 0.0098 |
| 10,000, 20 | 63,856 | 8 | 760,071 | 0.084 |
| 10,000, 200 | 462,479 | 8 | 760,071 | 0.61 |
| 10,000, 400 | 828,508 | 8 | 760,071 | 1.09 |
| 10,000, 800 | 1,465,422 | 8 | 760,071 | 1.93 |
| 10,000, 1,600 | 2,598,113 | 8 | 760,071 | 3.42 |

Ratios are my arithmetic on the S7 table (`RESULTS.md` lines 155–171). Crossover by linear interpolation: 33 new events at 1,000 chains (between 30 and 40), 363 at 10,000 (between 200 and 400), i.e. 3.3 and 3.6 percent of the chain count (S7 RESULTS lines 173–175). Range's control is 3.6 times data at N=200 on 1,000 chains and 4.7 times at 10,000 (S7 RESULTS lines 144–145).

Sweeps at 1,000 chains, one-sided (S7 `logs/sweep-1k.txt`, verified): N=20: K2/T4 35,312 (18 flights), K4/T8 31,777 (11), K16/T16 50,939 (7), K32/T16 73,383 (7). N=200: K2/T4 263,470, K4/T8 236,741, K16/T16 354,280, K32/T16 492,245. The same ordering holds in my bidirectional runs, with K4/T8 cheapest in bytes.

Equal stores: range 33 bytes and 2 flights, against 76,071 bytes and 2 flights for LogSync (S7 RESULTS lines 21–22). Index build per session: 21–26 ms at 1,000 chains, 153–183 ms at 10,000, per side (S7 lines 292–293). Known O(n) scan on the data phase at 10,000 chains (S7 lines 294–297).

Interior sequence gap: range repairs it (one event, 7 flights), and a height comparison sees nothing (0 of 1,000 authors differ) (S7 RESULTS lines 226–228). LogSync's `compare_logs` behaves the same way (E3h).

### E6. Code size, for "code to maintain" (verified line counts)

Range: `range.rs` 381 lines (tests about 80 of them), `session.rs` 270, `store.rs` 138, `chain.rs` 168 (`/home/user/work/spikes-next/s7-range-real/src/`). This is our code, so we maintain it. LogSync: `log_sync.rs` 979 lines (about 530 without tests), `api.rs` 664, `dedup.rs` 116, plus an adapter to implement, which does not exist yet (not checked: its size). LogSync is upstream code at a pre-release git rev whose API differs from the published 0.7.1 (E4).

## Options

### Table O1. K and T as settings

| Option | Compat impact | Verdict |
|---|---|---|
| Per-node settings, validated (K ≥ 2, T ≥ 1), not negotiated | none on the wire (verified by the mixed runs) | recommended |
| Fixed constants | none, but no tuning for bulk catch-up | acceptable fallback |
| Negotiated per session | adds a round trip and a wire change; no benefit, since the parameters are not on the wire | rejected |

### Table O2. K/T pairs, 1,000 chains, 20 new events each side (verified)

| Pair | Control B | Flights | Use |
|---|---|---|---|
| K2/T4 both | 66,001 | 18 | none: more bytes and far more flights than K4/T8 |
| K4/T8 both | 56,761 | 11 | bulk catch-up (fewest bytes here) |
| K16/T16 both | 89,698 | 7 | default (tied with K32/T32 for the fewest flights) |
| K32/T32 both | 125,180 | 7 | none: same flights as K16, 40% more bytes |
| K16/T16 to K4/T8 | 72,062 | 9 | mixed: about half the saving of both-sided K4/T8 |
| K4/T8 to K16/T16 | 79,200 | 8 | mixed |
| K2/T4 to K32/T32 | 99,603 | 8 | mixed, extreme |
| K32/T32 to K2/T4 | 100,639 | 9 | mixed, extreme |

Reading: the bulk saving is worth most when both sides use it. In this workload, only the acceptor at K4/T8 saves 17,636 bytes, only the dialer at K4/T8 saves 10,498, and both at K4/T8 save 32,937. Each mixed row is one configuration, run three times with identical bytes.

### Table O3. Range versus LogSync, by new events as a fraction of chains (verified from S7)

| New events as % of chains | Range/LogSync control, 1,000 chains | Range/LogSync control, 10,000 chains | Flights, range / LogSync |
|---|---|---|---|
| 0.2% | 0.086 | 0.0098 (at 0.02%) / 0.084 (at 0.2%) | 7–8 / 2 |
| 2% | 0.67 | 0.61 | 7–8 / 2 |
| 3.5% (crossover) | 1.0 | 1.0 | 7–8 / 2 |
| 4% | 1.19 | 1.09 | 7–8 / 2 |
| 10% | 2.64 | 1.93 (at 8%) | 7–8 / 2 |
| 20% | 4.66 | 3.42 (at 16%) | 7–8 / 2 |

The ratio depends on the fraction, not on the chain count, which is why the rule uses D/C.

### Table O4. Failure modes

| Mode | Range | LogSync | Evidence |
|---|---|---|---|
| Interior sequence gap | repaired | not seen; no transfer, no error | range: S7 (a); LogSync: E3h |
| Fork (same key, other ID) | detected; the batch fails; session aborts; neighbouring events in the batch are lost | not seen; no transfer, no error | E3d (verified) |
| Bootstrap, large | fails when a flight exceeds 16 MiB (about 524,288 IDs), either direction | 2 flights, control ~76 B per chain, no frame problem | E3g (verified in memory; wire not run at that size) |
| T = 0 | never terminates on a single-key mismatch (the in-memory run aborts at 10,000 flights) | not applicable | E3f (verified) |
| K = 1 | panic at `range.rs:97` | not applicable | E3f (verified) |
| Empty store | fails to read until tables exist (fixed in copy) | needs a `LogStore` adapter | E3e (verified) |
| Pruned log during sync | not applicable | short sync, no error | test assertion, E4 (not re-run) |
| Out-of-order message | session aborts | session aborts | E4, E2 |
| Interrupted session | whole batches committed; restart completes | not measured | S7 (d1) |
| Byte counters (overflow) | not applicable | `u32` counters overflow above 4 GiB per session | E4 (read; not reached) |

### Table O5. What a change requires

| Change | Protocol change? | Notes |
|---|---|---|
| K (fan-out) | no | local; validate K ≥ 2 (`range.rs:97`) |
| T (leaf threshold) | no | local; validate T ≥ 1 (E3f) |
| Batch size (`Params.batch`) | no | receiver's commit batching (`session.rs:165`) |
| Separator choice (which prefix is the bound) | no | any prefix that separates the neighbouring keys works (`range.rs:82–86`, `:69–71`) (inferred) |
| Fingerprint function (hash, truncation length, XOR, count rule) | **yes**: `/id-sync/2` | both sides must compute the same fingerprint for equal ranges (`range.rs:45–79`) |
| Bounds encoding (length byte, the zero sentinel, prefix form) | **yes**: `/id-sync/2` | `range.rs:173–201` |
| Key layout and width (author ‖ BE u64 seq, 40 bytes) | **yes**: `/id-sync/2` | bounds are key prefixes, so the byte order and width are part of the wire meaning (inferred; not tested) |
| Leaf ID width (32 bytes) | **yes** | `range.rs:194–196` |
| Mode bytes or the reply flag | **yes** | the decoder rejects unknown mode bytes (`range.rs:255`) |
| Framing (u32 BE length) and the data-phase count frame | **yes** | `session.rs:67–82`, `:137` |
| `MAX_FRAME` | **yes**, and a compat issue: a reader refuses larger frames (`session.rs:78`) | raise on every node before any node relies on it |
| Event ID definition (BLAKE3 over header bytes) | **yes**, a data-model change | IDs feed the fingerprint (`chain.rs:57–59`) |

## Recommendation

1. **K and T: make them per-node settings, validated at load.** K ≥ 2 and T ≥ 1. Keep K16/T16 as the default. Use K4/T8 for bulk catch-up, enabled on both sides for the full saving (Table O2). Do not put K or T in the protocol ID, and do not negotiate them.
2. **Selection rule** (for N5, which currently says "range above about 40 chains"): range when D ≤ 0.035·C, LogSync above that and for bootstrap. The 40-chain figure should be replaced: it holds only for 2 new events. This moves the design away from "range always" (the task-04 decision that LogSync is not used). If the owner keeps range above the crossover, make the threshold a setting. At 10% of chains range costs 2.6 times LogSync's control bytes (Table O3), and every diff up to that point still uses range.
3. **Before relying on range, fix three things.** (a) Frame size: either raise `MAX_FRAME` on every node first (a compat change: a reader with the old cap refuses larger frames), or bound the flight size (a protocol change, not designed here). Until then, bootstrap above about 524,000 events fails. (b) Forks: the receiver should check each event's key before it enters the batch, skip a conflicting event, and continue. That is a local receiver change with no wire impact (not verified; the event would be retried every session). (c) Store: create tables on open (E3e).
4. **LogSync** needs a redb adapter for `LogStore` and `OperationStore` before it can be measured or used. Its gap and fork blindness is acceptable only if the store guarantees contiguous chains. Ingest must reject gaps for that, which task 02's orderer is meant to provide (S7 RESULTS line 282).
5. **What would change this.** A real RTT (the flight count matters more if RTT is high); a measured LogSync run over iroh (its CPU and memory are not measured here); evidence that chain counts above 10,000 are common (the bootstrap cap then becomes urgent); a store that cannot guarantee contiguous chains (then range's gap detection is required, and LogSync is out).

## Not verified

- A real network RTT. Everything ran on loopback, so the flights-to-latency conversion is inferred.
- A LogSync session. No redb adapter exists. LogSync's CPU, memory and bytes for operations are not measured; its control bytes are S1's CBOR numbers.
- The wire run at 550,000 events. The frame failure is shown in memory, and the empty-dialer case is in memory too.
- T = 0 on realistic sparse diffs. Only the single-key mismatch was tested.
- Fork handling across batch boundaries. The stall was shown for one batch; behaviour with several batches is inferred from `session.rs:165–171`.
- Whether a fork is retried identically on every session. Inferred from the unchanged key position; one session per configuration only.
- ALPN negotiation behaviour in iroh when two peers offer different versions. Not tested.
- Wall times: noisy on a shared host, medians of 3, indicative only.
- Chain counts other than 1,000 for the new tests. S7's 10,000-chain numbers are cited, not re-run.
- `bench`, `scenarios` and `gen` in the copy were not built after my changes. `sync_pair` keeps its signature and `Store::open` keeps its API, so they should still compile (not checked).
- The 524,288 threshold assumes the reply is 32 bytes per ID with no other content in the flight. Verified for the empty-responder case only.

## Harness changes (copy at `harness/`)

Only these differ from `/home/user/work/spikes-next/s7-range-real/src/` (verified with `diff -r`):

- `src/session.rs:224–` `sync_pair` now calls `sync_pair_mixed(dialer, acceptor, p, p, cut)`. The new `sync_pair_mixed(…, pd, pa, …)` (line 229) gives the dialer `pd` and the acceptor `pa`.
- `src/range.rs:304–330` `simulate_mixed(a, b, (T,K), (T,K))`, the in-memory run with separate parameters per side. Its termination assertion is at `range.rs:327`.
- `src/chain.rs:83–91` `event_with_body`, for the fork test.
- `src/store.rs:48–59` `Store::open` creates both tables (the defect in E3e).
- `src/bin/mixed.rs` (new, 563 lines): `pairs`, `bootstrap`, `fork`, `tzero`, `frame`, `frame-empty`, `gap`. Ground-truth checks in `sync_check` (`mixed.rs:147–240`).
- `Cargo.toml` and `Cargo.lock` are copies, with the package name unchanged (`s7-range-real`).
