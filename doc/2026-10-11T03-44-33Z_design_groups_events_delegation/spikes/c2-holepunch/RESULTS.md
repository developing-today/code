# C2: holepunching between two peers behind NAT, both homed on our relay

Date: 2026-10-11. Task: task-08 item 9 (C2), with connectivity design §2, §5 and §7. Sandbox: one Linux host, network namespaces in a user namespace. Nothing outside the lab topology was changed.

## Question

Two iroh peers on separate networks, both homed on our relay (a local `iroh-relay` in the lab's root namespace), with n0's relays in their maps. Does a direct path form by holepunching through NAT?

Measured for each run: time to first data over the relay, time to a direct path, whether the direct path holds, and what happens on fallback.

## Verdicts

1. **Full cone (both sides): holepunch succeeds, 3 of 3 runs.** The direct path was selected 40, 112 and 321 ms after dial start. It held for the whole 60 s window in all three runs, with no echo failures.
2. **Port-restricted (both sides): holepunch succeeds, 3 of 3 runs.** Direct at 41, 47 and 72 ms. Held for 60 s in all three.
3. **Symmetric (both sides): relay fallback, 3 of 3 runs.** No direct path in 60 s. All 234 to 236 echoes per run went over the relay, with none failed. iroh made 4 holepunch attempts, against 3 in the successful runs.
4. **Mixed pairs (extra, not in the brief):** symmetric against full cone holepunches in both orders (direct at 29 to 40 ms, 2 runs each). Full cone against port-restricted holepunches (34 ms, 1 run). Symmetric against port-restricted falls back in both orders (3 runs, plus 1 in the reverse order).
5. **QAD is what provides the public candidate.** With the home relay's QAD off and no other QAD relay, no direct path forms, even with a restricted NAT (30 s). With a QAD-only relay added to the map as a stand-in for a non-home QAD relay, a direct path forms at 39 ms and holds for 30 s. This supports the design's inference, but the stand-in is on the same host. Real n0 relays could not be reached (§6).
6. **Relay-only tickets do not stop holepunching.** Every ticket carried zero IP addresses, and direct paths still formed. The candidates are exchanged inside the QUIC connection, over the relay. A connected peer therefore learns our LAN and public addresses anyway (§5).

## Method

### Lab topology (`lab/topo.sh`)

Everything runs inside one `unshare -Urn` namespace per run. The host's network config is not touched, and all namespaces and processes are removed on exit.

    root (lab) ns: relay 10.0.0.1/24 on bridge br-wan
      gw-a: wan 10.0.0.2 | lan 192.168.1.1 (ip_forward, nftables NAT)
        host-a: 192.168.1.2, default via 192.168.1.1   (echo server)
      gw-b: wan 10.0.0.3 | lan 192.168.2.1 (ip_forward, nftables NAT)
        host-b: 192.168.2.2, default via 192.168.2.1   (dialer)

The two LANs are separate private networks. Each peer reaches the other only through its gateway's NAT. The WAN is one L2 bridge, so there is no router hop. Each gateway runs one NAT type, selected per run:

| NAT | nftables rule | What it does |
|---|---|---|
| cone | `masquerade` (port-preserving), plus `iifname wan udp dport 4444 dnat to <host>:4444` | Endpoint-independent mapping and filtering for port 4444 (full cone) |
| restricted | `masquerade` (port-preserving) | Endpoint-independent mapping, address-and-port filtering (port-restricted cone) |
| symmetric | `masquerade fully-random` | A new random external port per destination (symmetric) |

Both gateways drop unsolicited new flows aimed at the gateway itself (`input` chain). The cone DNAT covers only UDP 4444. That is exact for this test, which uses one fixed port, but it is not a general full-cone NAT.

`lab/probe-nat.sh` checked each NAT with plain UDP before any iroh code ran (`logs/probe-nat-*.txt`). The mapping and filtering behaved as designed in all three cases (table in §3.6).

### Relay (`lab/run.sh`)

- `iroh-relay` 1.3.0, the binary at `/home/user/work/spikes/tools/bin/iroh-relay` (`--version` prints 1.3.0). It was built earlier by the C1 work and not rebuilt here.
- Config: plain HTTP for relay traffic on `10.0.0.1:3340`. QAD on UDP `10.0.0.1:7842` with a TLS certificate for the IP 10.0.0.1, issued by a lab CA. Metrics on loopback. This is setup B (`vps.toml`) without the public TLS listener.
- Peers trust only the lab CA (`CaTlsConfig::custom_roots`). The relay's public TLS is not exercised.

### Peers (`src/main.rs`, binary `c2-holepunch`)

- Built on iroh 1.3.0. `Endpoint::builder(Minimal)`, IPv4 only, bound to UDP port 4444 in both namespaces.
- Relay map: the home relay (QAD on) plus n0's production relays from `c1_connect::connect::n0_relays()` (QAD on). The `c1-connect` crate is a path dependency, used read-only for `AddressPolicy`, `n0_relays` and the relay list.
- No lookup services. The ticket is built with `AddressPolicy::RelayOnly`, so it holds the home relay URL and no IP addresses. The dialer asserts `ip_addrs().count() == 0`.
- Server (host-a): waits for `online()`, sleeps 2 s so QAD can settle, writes the ticket, then echoes every stream.
- Dialer (host-b): dials the ticket, sends one echo as the first reply, then sends an echo every 250 ms for the window (60 s in the matrix). It records every `PathEvent::Selected`, and at the end it reads `holepunch_attempts` from `endpoint.metrics()`.

Definitions, all from the dial start unless noted:
- *online*: from process start to `endpoint.online()`.
- *handshake*: `connect()` returns, and the path selected at that moment.
- *first reply*: the first echo reply, and the path selected when it arrived.
- *first direct*: the first `Selected` event whose path is an IP path.
- *echoes on direct*: echoes whose sample found an IP path selected.
- *holepunch attempts*: `socket.holepunch_attempts` at window end. The first attempt happens before the server's candidates arrive, so it counts too.

### Commands

    export CARGO_TARGET_DIR=/home/user/work/spikes/target
    CARGO="$(rustup which --toolchain 1.97.0 cargo)"
    cd /home/user/work/spikes-next/c2-holepunch
    "$CARGO" build                                  # -> /home/user/work/spikes/target/debug/c2-holepunch

    lab/probe-nat.sh cone                           # likewise restricted, symmetric
    lab/run.sh cone r1 60                           # matrix: r1-r3 for cone, restricted, symmetric
    NAT_B=cone lab/run.sh symmetric mix1 30         # mixed pairs: NAT_B is gateway b's NAT
    HOME_QAD=off lab/run.sh restricted noqad-r1 30  # control: home relay without QAD
    HOME_QAD=off N0=off STANDIN=on lab/run.sh restricted standin-r1 30   # QAD-only stand-in
    N0=off lab/run.sh restricted n0off-r1 10        # control: no n0 entries in the maps
    RUST_LOG=iroh::_events::qnt=debug,iroh::_events::path=debug lab/run.sh symmetric qnt-sym 12
    python3 -I lab/summarize.py > logs/summary.md   # the table in §3.1, from the logs

Environment: `NFT` defaults to `/nix/store/66dl0246qv0avvzid7xkj85xqdlrx78a-nftables-1.1.7/bin/nft`. `nft` is not on PATH on this host (`/run/current-system/sw/bin/nft` does not exist). Nix was not run. The store binary works inside `unshare -Urn`: tables, masquerade, `fully-random`, DNAT, and `sysctl net.ipv4.ip_forward` all work.

## Results

### 3.1 Main matrix (60 s holds, default profile: home relay QAD on, n0 in the maps)

| NAT (both sides) | run | online ms | handshake ms (path) | first reply ms (path) | first direct ms | echoes on direct / ok | failed | path at end | holepunch attempts |
|---|---|---|---|---|---|---|---|---|---|
| cone | r1 | 3196 | 185 (relay) | 334 (direct) | 321 | 232 / 232 | 0 | direct | 3 |
| cone | r2 | 3067 | 46 (relay) | 86 (relay) | 112 | 233 / 234 | 0 | direct | 3 |
| cone | r3 | 3013 | 20 (relay) | 36 (relay) | 40 | 237 / 238 | 0 | direct | 3 |
| restricted | r1 | 3035 | 22 (relay) | 44 (relay) | 47 | 237 / 237 | 0 | direct | 3 |
| restricted | r2 | 3046 | 34 (relay) | 63 (relay) | 72 | 234 / 234 | 0 | direct | 3 |
| restricted | r3 | 3020 | 22 (relay) | 37 (relay) | 41 | 236 / 237 | 0 | direct | 3 |
| symmetric | r1 | 3010 | 16 (relay) | 30 (relay) | none | 0 / 236 | 0 | relay | 4 |
| symmetric | r2 | 3016 | 18 (relay) | 33 (relay) | none | 0 / 236 | 0 | relay | 4 |
| symmetric | r3 | 3054 | 55 (relay) | 105 (relay) | none | 0 / 234 | 0 | relay | 4 |

Median echo RTT in these runs, in ms. Direct: cone 3.2, 3.1, 1.4; restricted 1.6, 2.3, 1.5. Relay: symmetric 2.2, 2.4, 3.7. Relay samples are few in the direct runs, since the direct path came first. The relay is on the same host, so these are not WAN figures.

All 27 runs in this document exited 0 from the dialer: matrix 9, smoke 3, mixed 9, controls 4, traced 2. No echo failed in any of them.

### 3.2 Time to first data, time to direct, and holding

- **First data over the relay.** The handshake completed over the relay in every run, after 14 to 185 ms. The first echo reply came over the relay in 8 of the 9 matrix runs, after 30 to 105 ms. In cone r1 the direct path was already selected when the first reply arrived (334 ms), so that run has no relay-carried reply to time.
- **Time to direct.** Cone 40 to 321 ms after dial start (112 ms median). Restricted 41 to 72 ms. After the first reply, the direct path came 0 to 26 ms later (cone 26 and 4 ms; restricted 3, 9 and 4 ms). Mixed pairs: 29 to 40 ms.
- **Holding.** Once selected, the direct path stayed selected for the whole 60 s window in all six cases. There were no path closures or relay switches before teardown. Echoes ran at 1.4 to 3.2 ms median RTT.
- **Fallback (symmetric).** Relay carried everything. Holepunch attempts were 3 to 4 per run. Retry spacing is 5 s (`HOLEPUNCH_ATTEMPTS_INTERVAL`, `remote_state.rs:52`), and the trace shows attempts 5 s apart (§3.5). The count was 3 after 12 s and 4 after 60 s, so iroh does not retry at a fixed rate for the whole window. The relay path stayed in use and echoes kept running. We saw no failures or timeouts.

### 3.3 Mixed pairs (30 s holds)

Server (host-a, gateway a) listed first, dialer (host-b, gateway b) second.

| server NAT | dialer NAT | runs | handshake ms | first direct ms | echoes on direct / ok | path at end | verdict |
|---|---|---|---|---|---|---|---|
| symmetric | cone | 2 | 16, 21 | 32, 40 | 118/119 each | direct | holepunch |
| cone | symmetric | 2 | 14, 15 | 29, 29 | 118/119 each | direct | holepunch |
| cone | restricted | 1 | 18 | 34 | 118/119 | direct | holepunch |
| symmetric | restricted | 3 | 18, 16, 14 | none | 0/119 each | relay | fallback |
| restricted | symmetric | 1 | 16 | none | 0/119 | relay | fallback |

Symmetric against full cone works, in both orders. The full-cone side accepts unsolicited inbound on its port, so the symmetric side's probe creates a mapping and the reply gets through. Symmetric against port-restricted fails in both orders. Port-restricted filters on address and port, and the symmetric side's probe comes from a port that the restricted side never sent to. The reason is inferred from the NAT rules and the traces, not from a packet capture.

### 3.4 Controls

| control | home QAD | n0 in maps | stand-in | NAT | online ms | own public candidate | first direct | echoes direct / ok | attempts |
|---|---|---|---|---|---|---|---|---|---|
| no QAD anywhere | off | on (unreachable) | off | restricted | 3036 | none (LAN only) | none | 0 / 119 | 2 |
| no QAD anywhere, clean | off | off | off | restricted | 209 | none (LAN only) | none | 0 / 119 | 2 |
| QAD-only stand-in | off | off | on | restricted | 410 | 10.0.0.3:4444 | 39 ms | 119 / 119 | 3 |
| n0 removed, home QAD on | on | off | off | restricted | 220 | 10.0.0.3:4444 | 42 ms | 40 / 40 | 3 |

- **QAD is needed.** Without any QAD relay, a peer only has its LAN address to offer, and holepunching has nothing to aim at.
- **A non-home QAD relay is enough.** The stand-in serves no relay traffic, so it cannot be chosen as home. Both peers got their public address from it, and the direct path formed. This tests the mechanism, not n0 itself, since n0 was unreachable.
- **Dead n0 entries cost startup time.** With n0's entries in the map, going online took 3.0 to 3.2 s. With them removed, it took 0.21 to 0.22 s. The sandbox cannot reach n0, so every n0 entry is dead here. The 3 s is consistent with iroh's net-report probe timeout (`PROBES_TIMEOUT` = 3 s, `net_report/defaults.rs:23`). I did not trace the net report to confirm it.

### 3.5 Holepunch candidates (iroh events, `RUST_LOG`)

Symmetric (`logs/symmetric-qnt-sym-dial.txt`):

    local_candidates  = {10.0.0.3:51554, 192.168.2.2:4444}   # QAD mapping for the relay, and LAN
    remote_candidates = {10.0.0.2:21499, 192.168.1.2:4444}   # the server's
    qnt::init: one with no remote candidates at the handshake, then 17.287 s and 22.287 s
    (5 s apart). No direct path was selected.

The public candidate is a mapping for the relay destination only. Its port (51554) is not the peer-facing port, so it does not help. The server's candidate is 21499 for the same reason.

Restricted (`logs/restricted-qnt-restr-dial.txt`):

    local_candidates  = {10.0.0.3:4444, 192.168.2.2:4444}
    remote_candidates = {10.0.0.2:4444, 192.168.1.2:4444}
    path opened 192.168.2.2 -> 10.0.0.2:4444, selected at 34 ms.

Here the public port is the bound port, because the mapping is port-preserving. The direct path leaves through the LAN address and the gateway masquerades it.

### 3.6 NAT behaviour, checked before iroh (`logs/probe-nat-*.txt`)

| NAT | external port seen at root:9000 / root:9001 | unsolicited datagram from root:9002 to host-a:4444 |
|---|---|---|
| cone | 4444 / 4444 | delivered (from 10.0.0.1:9002) |
| restricted | 4444 / 4444 | dropped |
| symmetric | 54109 / 1582 | dropped |

The QAD-observed address of host-a (server side) agrees: 10.0.0.2:4444 for cone and restricted, and 10.0.0.2:59478 in the symmetric run. The dialer's observed address in symmetric runs was a different random port in each run (34161, 1779, 12408, 51554, 42806).

## Not tested, and why

- **n0's relays themselves.** The lab has no uplink, so n0's relays could not be reached. The "n0 in the maps" part of the brief is nominal here. The entries were dead, and they cost about 3 s at startup (§3.4). The QAD inference is tested only with a stand-in on the same host.
- **Real NAT devices and real networks.** The NAT behaviour is Linux netfilter emulation, checked against the probe (§3.6). Mixed-NAT pairs have one to three runs each. The lab has one NAT per host, so double NAT (CGNAT in front of a router) is not tested.
- **Latency.** Every hop is local, with sub-ms RTT. The times above measure software and scheduling, not WAN RTT. Each holepunch step costs at least one RTT, so real times will be larger. A lost first round also costs up to the 5 s retry interval.
- **Idle periods and NAT timeouts.** Each hold kept traffic flowing (an echo every 250 ms), and the kernel's default conntrack timeouts were in effect. Idle longer than 60 s, and real NAT mapping lifetimes, were not tested.
- **TLS on the relay.** Relay traffic ran over plain HTTP, and only QAD used TLS (lab CA). The public relay with a webpki certificate is untested.
- **Tunnel-style relay.** The no-QAD control is the Tunnel setup's network behaviour. The real Cloudflare Tunnel was not tested (C1 t7 covers it).
- **IPv6, mDNS, DHT, and ticket lookups.** Out of scope for C2. Tickets were the only address source.
- **Packet captures.** None. Mapping and filtering claims rest on the probe, on the QAD addresses, and on iroh's own events.
- **Path migration and relay failover during a direct path.** Not tested.

## Surprising

1. **Relay-only tickets do not stop holepunching, but they do not hide addresses either.** The candidates travel inside the QUIC connection, over the relay. The builder's `addr_filter` (`endpoint.rs:628`) is passed only to `AddressLookupServices::set_addr_filter` (`endpoint.rs:306`), so it applies to address-lookup publishing. It does not touch `local_direct_addrs` (`remote_state.rs:1105`), which feed the candidates. So a connected peer learns our LAN and public addresses, even when the ticket holds none. This is worth recording in the design, since the RelayOnly policy reads as if it hides IPs.
2. **Symmetric NAT makes the QAD address useless for holepunching.** The address QAD reports is valid only toward the relay. An endpoint behind a symmetric NAT therefore has no usable public candidate, even with QAD working. The design's "QAD gives a public address" holds only for endpoint-independent mapping.
3. **Symmetric against port-restricted falls back, in both orders.** So for pairs of phone and home router, the relay will carry the traffic. Relay capacity matters for those pairs.
4. **A dead relay entry costs about 3 s at startup** in this sandbox, with nothing else wrong.
5. **nftables is not on PATH on this NixOS host.** The store binary works fine inside the user namespace.

## Implications for the design (for the owner to decide)

- Keep a QAD-enabled relay in every client's map. A Tunnel-only home relay gives no direct paths. This supports connectivity §4's recommendation of option B (VPS, QAD) if direct paths matter. n0's own QAD could not be tested here.
- Expect relay fallback for symmetric-against-restricted and symmetric-against-symmetric pairs. Plan relay capacity for them.
- The "no IPs in tickets" rule keeps addresses out of tickets and lookups, not out of the connection. The design should say that a connected peer learns our addresses.
- Startup should not wait on every relay in the map. A dead entry added about 3 s in this sandbox.
- A lost first holepunch round costs up to 5 s. On a lossy network, expect that.

## Files

- `src/main.rs`: the peer binary (`serve`, `dial`).
- `lab/topo.sh`: the namespace topology and the NAT rulesets.
- `lab/run.sh`: one run. Sets up the topology and relay, runs both peers, cleans up.
- `lab/probe-nat.sh`, `lab/probe.py`: the NAT behaviour check.
- `lab/summarize.py`: the summary table from `logs/*-dial.txt`.
- `logs/`: small text logs (the largest is 3 KB). The table is in `logs/summary.md`. Runs: `*-smoke` (8 s), the matrix runs `r1`–`r3`, `mix*` (mixed pairs), `noqad-*`, `standin-*`, `n0off-*`, and `qnt-*` (with holepunch events).

The matrix runs and the smoke runs were executed before the `--home-qad`, `--n0`, and `--standin-url` flags, the `RUST_LOG` subscriber, and `NAT_B` were added. Those additions leave the default behaviour unchanged. The traced default runs (`qnt-restr`, `qnt-sym`) were run on the final binary and reproduce the matrix: restricted holepunches at 34 ms, symmetric falls back to the relay.
