# C1 re-run: t2 and t4 on the current build

## Question

The QUIC-flag change (a per-relay `quic` field in `RelayEntry`) landed after the last t2 and t4 runs. Do t2 (fallback when the local relay goes down) and t4 (dial by ID through n0, no ticket) still pass on the current `c1` build? How do the numbers compare with the earlier runs?

## Verdicts

- **t2: PASS** (first run). The server re-homed onto n0 **22.5 s** after the local relay shut down. A fresh client's handshake over n0 took **547 ms**. A warm client's handshake over n0 took **1.32 s**, and its own home relay was still the dead one when it dialed.
- **t4: PASS** (first run). Dial by ID with no ticket: handshake **216 ms** over `usw1-1`, reply 398 ms. The run published two throwaway IDs (the server and the dialer).

Neither test failed, so no second run was made (the brief repeats a test only on failure). Every number is a single sample.

## Method

From `/home/user/work/spikes`:

```
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
CARGO_TARGET_DIR=/home/user/work/spikes/target "$CARGO" build -p c1-connect --bin c1 > build.txt 2>&1
./target/debug/c1 t2 > t2.txt 2>&1      # 12:09:33-12:10:12 UTC, exit 0
./target/debug/c1 t4 > t4.txt 2>&1      # 12:10:31-12:10:41 UTC, exit 0
```

Stdout and stderr both go to each file.

- **Build:** exit 0, 2m 43s, 326 crates compiled (including `c1-connect`), 0 warnings. Binary sha256 `4d7367764114c7bff2bebe84db81137c67f9ca955370c4d6d7edf216578275dc`. The previous binary (05:43) was `6e9aa4e03fe8d0e19affc660d2dc3d2d4777252cf69c5b008acccfafc194114e`.
- **Sources, unchanged by the run:** `Cargo.lock` `afbc44ff13f2a8087e34ccca3e9c6bc25da33e396d21edcc5a28fbb07043a4b4`; `src/connect.rs` `5fc14ab6d0f9dfa6f6cfa5683aa87a58c7c35e15c30da386fc865a2b92cf31f0`; `src/harness.rs` `1e03a82256c9ba53496291c292a9011ea574224a378b08063944670e84e25d7d`; `src/lib.rs` `10f9d7c685b582856376c10803292848079bc17f73d74c0c561b7828eacdd686`; `src/bin/c1.rs` `401cab74d3a8f4ccf3a81337cb5e1cb65b57229bb7caeb6cf039801f94a017ae`.
- **Environment:** `RUST_LOG` unset (warn level). `IROH_FORCE_STAGING_RELAYS` unset; the binary prints `false`, so these are n0's production relays. Load average was 37.2 on 16 CPUs at the start of t4. I did not sample it before t2.
- **Side effects:** t2 and t4 start no external relay and write no files in `c1-connect`. `logs/t2.txt` and `logs/t4.txt` keep their 05:20 and 05:18 times.
- **Publishing:** t2's profiles set no lookups (`Minimal` only sets the crypto provider), so t2 published nothing. Its relay connections went to n0's `aps1-1` relay. t4 published two throwaway IDs, the server's and the dialer's.

## Results: t2

Raw output (`t2.txt`):

```
relays: local http://localhost:39519/, n0 https://aps1-1.relay.n0.iroh.link./
server online after 3.039516134s
ticket: 162 chars, 2 relay URLs, 0 direct
server home relays at start: [RelayUrl("http://localhost:39519/")]
A (both relays up): connected 81.20407ms, replied 137.224971ms, echoed true, handshake over relay http://localhost:39519/
  path after reply: * relay http://localhost:39519/
warm client home relays before: [RelayUrl("http://localhost:39519/")]
server re-homed onto n0 22.511246006s after the shutdown
B: client online 26.223181584s after the shutdown (its own wait: 3.689736429s)
B (fresh client, local relay down): connected 546.888274ms, replied 1.113102897s, echoed true, handshake over relay https://aps1-1.relay.n0.iroh.link./
  path after reply: * relay https://aps1-1.relay.n0.iroh.link./
B: handshake complete 26.770095286s after the shutdown
C: home relays at dial time: [RelayUrl("http://localhost:39519/")]
C (warm client, local relay down): connected 1.320471446s, replied 1.889272544s, echoed true, handshake over relay https://aps1-1.relay.n0.iroh.link./
  path after reply:   relay https://aps1-1.relay.n0.iroh.link./
  path after reply: * direct (address withheld)
C: handshake complete 28.656883307s after the shutdown, over Some("https://aps1-1.relay.n0.iroh.link./")
RESULT t2: PASS
```

Comparison. The earlier run 1 is `c1-connect/logs/t2.txt` (05:20). Run 2 is from `c1-connect/RESULTS.md`; its log was overwritten, so only its summary numbers survive.

| Measure | Now (12:09 UTC) | Earlier run 1 (log, 05:20) | Earlier run 2 (RESULTS.md) |
|---|---|---|---|
| A: handshake over local, both relays up | 81.2 ms (reply 137 ms) | 14.9 ms (reply 27.5 ms) | 14-15 ms (both runs) |
| Server re-home onto n0 | **22.51 s** | 20.46 s | 21.6 s |
| B: fresh client, handshake over n0 | **547 ms** (reply 1.11 s) | 656 ms (reply 1.43 s) | 470 ms |
| B: client online (own wait) | 26.2 s after shutdown (3.69 s) | 24.3 s (3.87 s) | not recorded |
| C: warm client, handshake over n0 | **1.32 s** (reply 1.89 s) | 1.34 s (reply 1.90 s) | 486 ms |
| C: warm client's home relay at dial time | still the dead local relay (dial at 26.8 s) | n0 `aps1-1` (dial at 25.0 s) | not recorded |
| Result | PASS | PASS | PASS (2 runs) |

## Results: t4

Raw output (`t4.txt`):

```
profile: 4 relay(s) (0 with QAD off), policy RelayOnly, n0-pkarr publish, n0-pkarr resolve, n0-dns resolve
IROH_FORCE_STAGING_RELAYS set: false
server online after 3.243188175s
client online after 10.139µs
record visible through n0 lookups after 697.185698ms: ["dns: 1 relay, 0 direct", "pkarr: 1 relay, 0 direct"]
dial by ID (no ticket): connected 215.791598ms, replied 398.031311ms, echoed true, handshake over relay https://usw1-1.relay.n0.iroh.link./
  path after reply: * relay https://usw1-1.relay.n0.iroh.link./
  after 5 s: path   relay https://usw1-1.relay.n0.iroh.link./
  after 5 s: path * direct (address withheld)
RESULT t4: PASS
```

Comparison with `c1-connect/logs/t4.txt` (05:18). The earlier run was the only one recorded.

| Measure | Now (12:10 UTC) | Earlier (log, 05:18) |
|---|---|---|
| Profile line | `4 relay(s) (0 with QAD off)` | `4 relay(s)` (no QAD count) |
| Server online | 3.24 s | 3.28 s |
| Record visible via n0 lookups | 697 ms | 1.01 s |
| Record, as parsed | dns and pkarr: 1 relay, 0 direct each | same |
| Handshake, dial by ID | **216 ms** over `usw1-1` | 277 ms over `use1-1` |
| Reply | 398 ms | 648 ms |
| After 5 s | direct path selected (both endpoints on this host) | same |
| Result | PASS | PASS |

## Does the flag change these results?

Not for these two tests. `RelayMode::custom(urls)` builds its map with `RelayMap::from_iter(urls)`. Each entry becomes `url.into()`, which is `RelayConfig::from(url)` with the default QUIC config (`iroh-relay-1.3.0/src/relay_map.rs:208-213`, `:272-284`). `RelayEntry { quic: true }` builds the same `RelayConfig::from(url)` (`src/connect.rs:67-68`). Both tests use `quic: true` for every relay. The flag only changes `quic: false` entries, which neither test uses. The one visible change is in output format: the t4 profile line now carries "(0 with QAD off)".

## Not tested, and why

- **Repeats.** One run each. The earlier t2 had two runs. The re-home band comes from the code's timer, a random 20-26 s (`iroh-1.3.0/src/socket.rs:2000-2005`), so one sample cannot narrow it.
- **Real NAT.** Both endpoints share a host. The direct path that appears after 5 s is not holepunching across networks.
- **Other n0 relays.** t2 takes the fallback from `connect::n0_relays().next()` (`src/bin/c1.rs:204-207`), which is `aps1-1` here. That is the first entry, not the nearest. The n0 fallback times apply to that relay only. I did not measure the others.
- **Timing under load.** Load was 37 on 16 CPUs at t4 start. Timings are not controlled.
- **Record bytes (t4).** This run checked the parsed lookup items (1 relay, 0 direct). It did not scan the raw record for IP literals. t3 does that.
- **Other tests.** t1, t3, t5, t6, t7, t8 and `policy` were not run. They were not requested.
- **Second relay on the peer.** The open question in the c1 RESULTS (would a second relay connection shorten the re-home?) is not tested.

## Surprising

1. **t2 phase A (both relays up): handshake 81 ms, against 14-15 ms earlier, about 5x.** The config is the same as before (both relays QAD on), so I attribute the gap to load or noise. This is one sample, so the cause is not established.
2. **The warm client still listed the dead local relay as its home when it dialed (26.8 s after shutdown).** In the earlier run it had already moved to n0 by 25.0 s. The dial still worked, because the ticket names n0. The warm client re-homes on its own timer, so it need not move with the server.
3. **t4 used `usw1-1`, not `use1-1`.** The handshake went from 277 ms to 216 ms. I did not check why.
4. **The re-home at 22.5 s is above the design doc's "20-22 s".** It is inside the 20-26 s band from the code.

## Doc lines now out of date (not edited; outside this folder)

- `/home/user/work/integrated/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md`:
  - §3, t2 row: "Not re-run after the QUIC-flag change" is stale. It is re-run and passes, with a re-home of 22.5 s.
  - §3 t2 row and §7 gotcha 3: "20-22 s" should read "20-26 s (22.5 s in this run)".
  - §3 t4 row ("One run") and §1 ("in 277 ms"): there are two runs now. 277 ms over `use1-1` (05:18), 216 ms over `usw1-1` (12:10).
- `/home/user/work/spikes/c1-connect/RESULTS.md`, line 141 ("t2's 20-26 s re-home is not re-measured") and line 144 ("Not run: t2 and t4"). Both are stale.
- Throwaway IDs published to n0 by C1 in total: 4 (2 earlier, 2 in this run).

## Files

- `RESULTS.md` (this file)
- `t2.txt` (1.3 KB) and `t4.txt` (0.6 KB): raw output
- `build.txt` (10 KB): build log
