# S7 connect: connectivity composition for the `id` design

## Question

Can one declarative profile build an iroh 1.3.0 endpoint whose relays, lookups, and address policy are each switchable, publish nothing unasked, and pass six dial tests?

**Verdict.** Tests 1–6 pass as briefed. mDNS is built but not run. The main limit is peer-side relay failover, which takes 20–26 s.

## Setup

- iroh 1.3.0, iroh-relay 1.3.0 (`server`), iroh-tickets 1.0.0.
- Added to this member: iroh-mainline-address-lookup 0.6.0 (needs iroh ^1.0 and n0-mainline ^0.7); n0-mainline 0.7.1, which holds the testnet (the brief's "mainline crate"; no `mainline` crate is in the graph); iroh-mdns-address-lookup 0.6.0 (built, not run); iroh-dns 1.3.0 and n0-future 0.3.2 (already locked).
- Workspace: `c1-connect` added to `members`. The lock gained 10 packages. A name-and-version diff shows no existing pin removed or changed.
- Toolchain: rustup 1.97.0. Build is warning-free. No p2panda crates, so the pinned rev does not apply.
- Host: one Linux machine. All endpoints run on it.

## Commands

    "$CARGO" build -p c1-connect --bin s7
    target/debug/s7 t1   # likewise t2 t3 t5 t6 policy, with the final binary
    target/debug/s7 t4   # once: it publishes two throwaway IDs to n0

Logs: `c1-connect/logs/*.txt` hold the final runs. Tests 2 and 5 were run more than once and vary, so the table gives ranges.

## Results

| Test | Verdict | Measured |
|---|---|---|
| 1. Our relay only, no direct address | PASS | Ticket 103 chars, 1 relay, 0 direct. Handshake over the local relay 13.1 ms; reply 22.7 ms. |
| 2. Local relay down; fall back to n0 | PASS (2 runs) | Server re-homed onto n0 20.5 s and 21.6 s after shutdown. Handshake over n0, timed from each dial: fresh client 656 ms and 470 ms; warm client 1,337 ms and 486 ms. Before the failure, handshake over local: 14–15 ms. |
| 3. Mainline DHT, testnet, relay-only | PASS | Record readable 168 ms after online. Record holds `relay=http://localhost:<port>/` only, with 0 IP literals, though the publisher had 2 direct addresses. Resolve and dial by ID: 1 attempt, handshake 79 ms, over relay. |
| 4. n0 defaults, dial by ID (once) | PASS | Record visible through n0 pkarr and DNS 1.0 s after both endpoints were online. Dial by ID: handshake 277 ms over `use1-1` (NA East), reply 648 ms. After 5 s a direct path had been added and selected (both endpoints on this host). |
| 5. Lookup precedence | PASS | Table below. |
| 6. Negative | PASS | Below. |
| policy (extra) | PASS | `RelayAndIp` record: 1 relay URL and 2 direct addresses (counts only). |

Test 5, three rounds, reply times. Every dial selected the correct relay.

| Case | Reply time |
|---|---|
| A. Peer in DHT only | 101–116 ms |
| B. Peer in memory only | 33–38 ms |
| C. Memory: a live relay the peer is not on. DHT: right relay | 1,035–1,042 ms |
| D. Memory: dead direct address. DHT: right relay | 1,033–1,046 ms |
| E (control). Memory only: the wrong relay | fails at 30.0 s |

Test 6:

- (b) Address-less ticket, no lookup: fails in 0.36 ms, "No address lookup configured".
- (c) Bare ID, no lookup, no ticket: fails in 0.11 ms, same error.
- (b, DHT on) The same address-less ticket resolves through the DHT in 124 ms.
- (a) Unknown ID on the testnet DHT: fails in 2.7–3.1 ms, "All address lookup services failed or produced no results". The 8-node local testnet is fast; the public DHT is not measured.

## Findings

1. **Policy is enforced twice.** The endpoint filter applies to every service, and each publisher applies its own filter after it. pkarr and the DHT default to relay-only (`pkarr.rs:168`, applied at `:351`; `iroh-mainline lib.rs:169`, applied at `:323`). mDNS defaults to no filter (`iroh-mdns lib.rs:171–173`). `connect::bind` sets the policy's filter on all three.
2. **No lookup precedence.** Each lookup's items merge into one path set (`path_state.rs:135–154`). Until a path is selected, datagrams go to every known path (`remote_state.rs:805–845`). The selector ranks direct above relay (`biased_rtt_path_selector.rs:94–107`). A failing address that arrives first adds about 1 s, consistent with QUIC's initial probe timeout (not instrumented). A failing address that is the only one holds the dial for the 30 s idle timeout (`socket.rs:127–129`). The comment at `endpoint.rs:1153` says 10 s; the measurement contradicts it.
3. **Peer-side failover is slow.** A client falls back in one handshake over n0, 0.5–1.3 s here. A peer whose home relay died moves when its periodic re-STUN timer fires, every 20–26 s (`socket.rs:2000–2005`). A second relay in a ticket helps only once the peer is reachable on it.
4. **The DHT default is public.** `DhtBuilder::default()` bootstraps from four public nodes (`n0-mainline actor/config.rs:3–7, 45`). `connect::bind` refuses an empty bootstrap list, and the testnet helper refuses non-loopback nodes.

## Deviations

- **Name clash.** The design review's "S7" is range sync on real stores (review §9). This member is the connectivity composition from the brief.
- **`localhost`, not 127.0.0.1 as in S1.** The literal would put an IP into the record, which test 3 forbids.
- **Test 2's first run failed my own check.** It read the path after the echo and saw a later direct path. The check now reads the handshake path. That log was overwritten.
- **Policy check, first attempt** accepted a record with direct addresses but no relay yet. It now waits for both. This was a test race, not a module fault.
- **Test 4** ran on the binary before rustfmt and the policy check were added. The logic is unchanged.
- **Test 3's "no internet"** holds by configuration: a local relay and the testnet only. I did not capture packets to confirm it.

## Not run, and why

- **mDNS.** `iroh-mdns-address-lookup` 0.6.0 creates its socket with `Discoverer::new_interactive` (`lib.rs:497`). That binds `0.0.0.0:5353` and joins the group on every interface (`swarm-discovery-0.6.4 socket.rs:104, 140–155`). Its builder has no interface option (`lib.rs:161–200`). Any test would reach the shared LAN, which the brief forbids.
- **Public DHT:** never used, by design.
- **Real NAT:** not tested. Both endpoints share a host, so the direct paths seen are not NAT traversal.
- **`RelayAndIp` dialing** and **clippy** were not run; only `RelayAndIp` publishing was checked.

## Recommendation

- Use `connect::bind` as the one entry point. Default: the owner's own relays, no lookups, `RelayOnly`. Keep n0 lookups off unless asked.
- Introduce peers with tickets carrying relay URLs only. Add a second relay to a ticket only with the 20–26 s re-home in mind.
- Keep stale addresses out of lookups. A stale address costs about 1 s, or 30 s when it is the only one.
- Decide the n0 publishing policy before the design relies on it. Test 4 published two throwaway records.
- Untested: a second relay connection on the peer might shorten the re-home wait.

## Reusable API (`src/connect.rs`)

```rust
pub enum AddressPolicy { RelayOnly /* default */, RelayAndIp }
pub struct Profile { pub relays: Vec<RelayUrl>, pub lookups: Lookups, pub policy: AddressPolicy, pub alpns: Vec<Vec<u8>> }
pub struct Lookups { pub n0_pkarr_publish: bool, pub n0_pkarr_resolve: bool, pub n0_dns_resolve: bool,
                     pub mainline: Option<Mainline>, pub mdns: Option<Mdns>, pub memory: bool } // all off
pub struct Mainline { pub bootstrap: Vec<String>, pub publish: bool } // bootstrap required
pub struct Mdns { pub advertise: bool }
pub async fn bind(profile: &Profile) -> anyhow::Result<Bound>; // Bound { endpoint, memory: Option<MemoryLookup> }
```

Helpers: `AddressPolicy::filter()`, `AddressPolicy::apply(&EndpointAddr)` for tickets, `n0_relays()`, `Profile::summary()`.

**To add a lookup kind:** (1) add a field to `Lookups`, off by default; (2) add an `if` in `bind()` that calls `builder.address_lookup(...)`, passing `filter.clone()` if it publishes; (3) add its line to `Profile::summary()`; (4) test that it is absent by default and publishes nothing when off.

## iroh APIs used (1.3.0 sources, file:line)

- Endpoint: `Endpoint::builder` endpoint.rs:964; `Builder::empty` :193; `Builder::bind` :227 (random key :228; lookups added :304–312; filter :306–308); `relay_mode` :568; `RelayMode` :1993 and `::custom` :2031; `alpns` :546; `address_lookup` :616; `addr_filter` :628; `connect` :1068; `connect_with_opts` :1108 (`resolve_remote` :1146); `addr` :1215; `online` :1374; `address_lookup()` :1514.
- Presets: `Minimal` presets.rs:59–79; `N0` :113–140, the n0 set for reference.
- Lookups: `AddressLookup` trait address_lookup.rs:335; `AddressLookupServices::publish` :529–543; `resolve` :565; `AddrFilter` re-export :112, defined in iroh-dns `endpoint_info.rs:243` (`relay_only` :269, `unfiltered` :264). `PkarrPublisher::n0_dns` pkarr.rs:343; `PkarrResolver::n0_dns` :525; `DnsAddressLookup::n0_dns` dns.rs:92; `MemoryLookup` memory.rs:75, `new` :105, `with_provenance` :117, `add_endpoint_info` :184 (merges), `resolve` :218.
- Paths: `resolve_remote` socket.rs:1316; `handle_address_lookup_item` remote_state.rs:906; `insert_multiple` path_state.rs:135; `BiasedRttPathSelector` biased_rtt_path_selector.rs:94.
- Record: iroh-dns `endpoint_info.rs:486–492` (`relay=`, `addr=`); `from_pkarr_signed_packet` :463.
- Relay server (iroh-relay-1.3.0): `Server::spawn` server.rs:691; `RelayConfig::new` :155; `shutdown` :878; `http_addr` :910. `defaults::prod::default_relay_map` (iroh defaults.rs:36); `RelayMap::urls` (relay_map.rs:83).
- Addresses and tickets: `EndpointAddr` (iroh-base endpoint_addr.rs) `new` :96, `from_parts` :104, `ip_addrs` :137, `relay_urls` :147; `EndpointTicket::new` (iroh-tickets endpoint.rs:82), `endpoint_addr` :87.
- DHT lookup (iroh-mainline-address-lookup-0.6.0 lib.rs): `Builder` default :161–172; `dht_builder` :176; `no_publish` :202; `addr_filter` :217; `build` :226; `into_address_lookup` :246–251; `publish` :316–340; `resolve_dht` :108.
- n0-mainline-0.7.1: `DhtBuilder::bootstrap` dht.rs:74; `Dht::builder` :147; `get_mutable_most_recent` :445; `put_mutable` :516; `Testnet::new` dht/testnet.rs:21; `DEFAULT_BOOTSTRAP_NODES` actor/config.rs:3.
- Re-home timer: `new_re_stun_timer` socket.rs:2000–2005.

## t7 and t8: Tunnel-style relay, QAD off and on

**Question.** Can a profile dial through a relay with no UDP port (the Tunnel relay) with QAD off? What does a wrong flag cost?

**Change.** Each relay is `RelayEntry { url, quic }` (`src/connect.rs:58-61`). `quic: false` builds `RelayConfig::new(url, None)`; `quic: true` builds `RelayConfig::from(url)`, the default (`:66-73`). `bind` uses these in place of `RelayMode::custom` (`:182`). The fix is `:60` (the flag) and `:70` (the `None`). A bare `RelayUrl` means `quic: true`, so existing tests keep QAD on.

**Citations.** Relay map types are in `iroh-relay-1.3.0/src/relay_map.rs`, re-exported at `iroh-1.3.0/src/lib.rs:292`. `RelayConfig { quic: Option<RelayQuicConfig> }` `:232-240`; `RelayConfig::new(url, quic)` `:250`; `From<RelayUrl>` default port 7842 (`iroh-relay-1.3.0/src/defaults.rs:7`) `:272-284`; `FromIterator<RelayConfig> for RelayMap` `:163`. `RelayMode::Custom(RelayMap)` `iroh-1.3.0/src/endpoint.rs:1993-2004`; `RelayMode::custom(urls)` `:2031-2034`. QAD probes every relay (`iroh-1.3.0/src/net_report.rs:504-505`). A `None` port fails before any send (`net_report/reportgen.rs:648-658`).

**Setup and commands.** iroh and iroh-relay 1.3.0; rustup 1.97.0. `tunnel.toml` (sha256 `4c3f8c…`) was read only. Its `enable_quic_addr_discovery = false` and lack of `[tls]` mean TCP only (`iroh-relay-1.3.0/src/main.rs:721-725, 774-775`). The harness runs the relay with `--config-path` and `RUST_LOG=info`, waits for `/ping` = 200, and kills it on drop. Commands: `c1 t7`, `c1 t8` with `RUST_LOG=info,iroh::net_report=debug`, `c1 t8` at the default level, and `c1 t1|t3|t5|t6|policy` as regressions (`logs/regress-*.txt`). New dependency: `tracing-subscriber 0.3`. The lock gained one edge, no version changes.

**Measurements.** Four runs each, plus one t8 run at the default level. The path was the relay every time.

| | t7, QAD off | t8, QAD on |
|---|---|---|
| Online | 202–210 ms | 3.00 s |
| Handshake | 17–25 ms | 16–34 ms |
| Reply | 30–41 ms | 28–58 ms |
| Client log, default level | none | none |
| Client log, `iroh::net_report=debug` | `No port available`, instant | QADv6 `No suitable relay address (AAAA)`, instant; QADv4 `probe timed out` at 3.0 s |

**Verdicts.** Blockers: none.
- **t7 PASS.** Echo over `http://127.0.0.1:3340/`; 0 direct addresses in the ticket; no lookup on the client.
- **t8 PASS, negative.** The dial works over the relay. The cost is a 3.0 s wait to come online, and the QAD errors show only at debug. The harness asserts the dial only.

**Relay log** (`RUST_LOG=info`, same in both). Two INFO lines: metrics on 127.0.0.1:9090; serving on 127.0.0.1:3340. Two WARN lines, `actor errored Stream terminated, exiting`, one per endpoint, at teardown. They mean a client closed, not a fault. Nothing else. The relay never sees QAD, since it has no UDP listener.

**Other observations.**
- Local test relays have the same mismatch: `ServerConfig::default()` leaves `quic: None` (`iroh-relay-1.3.0/src/server.rs:109-115`), so QAD waits 3 s. That is why t1 and t3 come online in ~3 s. They still pass on the new build (t1, t3, t5, t6, policy). I kept QAD on, as briefed. t2's 20–26 s re-home is not re-measured.
- t1's `online:` line awaits server, then client, so its client figure reads near zero. Not changed.
- The captive-portal check uses port 80, not the relay port; it fails harmlessly at debug.
- Not run: t2 and t4 (n0 infrastructure).

**Deviations.** Binary is `c1` (was `s7`); usage string updated. `rustfmt` on the three touched files also fixed an import order. clippy is clean after one fix. The file is now about 2,100 words, over the README's 1,500, because this section was added as asked.

**Recommendation.** Use `quic: false` for every relay without UDP. Set the local test relays to `quic: false` too; that removes the 3 s wait and should not change verdicts. Before enabling `shared_token` (the commented key is correct, `main.rs:158-200`), give `RelayEntry` a token and call `RelayConfig::with_auth_token` (`relay_map.rs:266`).

## 2026-10-11T12-20-38Z Re-run: t2 and t4

Lines 141 and 144 above are stale. t2 and t4 were re-run on the current build, and both pass. See `../../c1-rerun/RESULTS.md`.
