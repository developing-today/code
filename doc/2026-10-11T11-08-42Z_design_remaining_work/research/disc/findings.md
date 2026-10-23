# iroh 1.3.0 connectivity: discovery services research (read-only)

Scope: iroh 1.3.0 and its address-lookup crates, read from the crates.io tarballs under
`/tmp/research-disc/src/<crate>-<ver>/`. Nothing was published to the DHT or to any n0
service. Nothing was compiled. `/home/user/code` and the docs repo were not touched.

Tags: [read] = I read the source or spec text in this session. [inferred] = my deduction.
No claim below rests on a search snippet.

## Versions (crates.io max stable, confirmed)

| crate | version | notes |
|---|---|---|
| iroh | 1.3.0 | lock: iroh-dns 1.3.0, iroh-base 1.3.0, n0-dns-resolver 0.1.0; no `pkarr` or `hickory-resolver` crate in lock |
| iroh-mainline-address-lookup | 0.6.0 | deps `n0-mainline ^0.7` (lock 0.7.0), `iroh ^1.0.0` (lock 1.0.0), `iroh-dns ^1.0.0` [read: Cargo.toml, lock, docs.rs] |
| iroh-mdns-address-lookup | 0.6.0 | dep `swarm-discovery ^0.6` (lock 0.6.1) [read] |
| mainline | 8.0.1 | NOT the dependency. The DHT crate is `n0-mainline` [read] |
| pkarr | 8.1.0 | NOT used by iroh 1.3.0; iroh-dns reimplements the format [read] |
| iroh-dns-server | 1.3.0 | reference pkarr relay + DNS + DoH server [read] |
| iroh-tickets | 1.0.0 | EndpointTicket [read] |

Compile check: not done. The mainline crate's import paths exist in iroh 1.3.0 (`iroh::endpoint_info`
re-exports `iroh_dns::endpoint_info`, lib.rs:291; `iroh_dns::pkarr::Timestamp::from_micros` exists).
Compatibility with 1.3.0 is therefore [inferred], not built.

## (a) pkarr / DNS (N0 preset)

- N0 preset = Minimal + `PkarrPublisher::n0_dns()` + `PkarrResolver::n0_dns()` +
  `DnsAddressLookup::n0_dns()` (non-wasm) + default relay mode. `presets.rs:116-137` [read].
  Minimal only installs a crypto provider (`presets.rs:62-80`) [read]. Mainline and mDNS are not in N0 [read].
- Publisher filter: `AddrFilter::relay_only()` by default (`pkarr.rs:168`, doc 208-211) [read].
  Filtered addresses become `_iroh` TXT attrs: `relay=`, `addr=` (IP or custom), `user-data=`
  (`endpoint_info.rs:486-501`, `attrs.rs:19-20, 82-89`) [read].
- User data is never filtered: `apply_filter` copies `user_data` through (`endpoint_info.rs:194-196`) [read].
- Packet: `<32 pubkey><64 ed25519 sig><8 BE µs timestamp><DNS packet ≤1000 B>`
  (`iroh-dns/src/pkarr.rs:17-30`). Signed bytes: `3:seqi<µs>e1:v<len>:<packet>` (`pkarr.rs:291-295`).
  This is the same shape BEP 44 uses, with seq = µs timestamp [read].
- Timestamp is process-monotonic (`pkarr.rs:341-351`) [read].
- Transport: HTTP `PUT <base>/<z32>` with body = `sig|ts|packet`; `GET <base>/<z32>` verified against the
  pubkey (`pkarr.rs:620-682`). Default base `https://dns.iroh.link/pkarr` (`pkarr.rs:127`) [read].
- Cadence: on every endpoint-data change, plus every 5 min (`pkarr.rs:146`). On failure, retry after 1, 2, 3, ... s
  with no cap (`pkarr.rs:389`) [read].
- TTL: 30 s (`pkarr.rs:143`). The doc says relays ignore it (`pkarr.rs:137-141`) [read].
- DNS: `TXT _iroh.<z32>.<origin>`, origin `dns.iroh.link.` (`iroh-dns/src/dns.rs:36`) [read].
  Attributes parsed: relay, addr, user-data (`endpoint_info.rs:504-530`). The `dns.rs` doc lists only `relay=` (`address_lookup/dns.rs:37-38`) — doc is behind code [read].
- DNS staggering: offsets `[0,200,300,600,1000,2000,3000]` ms from T0, absolute, with jitter (`stagger_call`, `dns.rs:1197-1228`); each query timeout 3 s (`dns.rs:33`). Worst case ≈6 s, matching the iroh doc (`address_lookup/dns.rs:19-21`) [read].
- Custom relay: `PkarrPublisher::builder(url)` / `PkarrResolver::builder(url)` (`pkarr.rs:290, 507`); the z32 key is appended to the URL path (`pkarr.rs:620-631`) [read].
- Custom DNS: `DnsAddressLookup::builder(origin)` + `.dns_resolver(...)` (`dns.rs:78, 62`); `DnsResolver::builder()` (`iroh-dns/src/dns.rs:762`), `.with_nameserver(addr, DnsProtocol::{Udp,Tcp,Tls,Https})` (`:420`), `.disable_fallback()` (`:494`) [read].
- Default resolver = system nameservers (`address_lookup/dns.rs:40-41`). Fallback mode default = Deferred (`n0-dns-resolver/src/builder.rs:99-103, 222`). Fallback nameservers = Cloudflare 1.1.1.1/1.0.0.1, Google 8.8.8.8/8.8.4.4, Quad9 9.9.9.9/149.112.112.112, UDP plus DoH on 443 where available (`n0-dns-resolver/src/public_resolvers.rs:42-60, 121-131`) [read].
- iroh doc says the n0 relay does not touch the Mainline DHT (`pkarr.rs:121-123`) [read, doc comment].

## (b) Mainline DHT (`iroh-mainline-address-lookup` 0.6.0 + `n0-mainline` 0.7.0)

- API: `DhtAddressLookup::builder()` → `Builder { dht_builder(), secret_key(), ttl(), republish_delay(), no_publish(), addr_filter(), build() }` (`lib.rs:152-243`). Also implements `AddressLookupBuilder`: the endpoint's key is used automatically (`lib.rs:245-251`) [read].
- Storage: BEP 44 mutable item. `k` = pubkey, no salt (target = SHA-1(k)), `seq` = µs timestamp, `v` = DNS packet, `sig` = pkarr signature (`lib.rs:44-52`; `n0-mainline/src/common/mutable.rs:63-70`) [read].
- Publish: `put_mutable(item, None)`, no CAS (`lib.rs:295`). Filter applied; empty result → no publish (`lib.rs:316-328`) [read].
- Republish: REPUBLISH_DELAY = 1 h (`lib.rs:34`); on a change, immediately after 50 ms debounce (`lib.rs:41, 283-291`) [read].
- Failure: on error the loop logs and sleeps the full 1 h before retrying (`lib.rs:294-311`) [read].
- Default filter `relay_only` (`lib.rs:169`). `unfiltered()` opts into IPs (`lib.rs:212-216`) [read].
- Resolve: `get_mutable_most_recent(pubkey, None)` (`lib.rs:116`); item provenance is hard-coded `"pkarr"` (`lib.rs:132`) [read].
- Bootstrap: `DhtBuilder::bootstrap(&[..])` replaces; `extra_bootstrap` appends; `no_bootstrap()` clears (`n0-mainline/src/dht.rs:74-97`) [read]. Defaults (`actor/config.rs:3-8`): `router.bittorrent.com:6881`, `dht.transmissionbt.com:6881`, `dht.libtorrent.org:25401`, `relay.pkarr.org:6881` (the last is not n0; operator [inferred] Pubky/pkarr).
- Testnet: `n0_mainline::Testnet::new(n)` (`dht/testnet.rs:21`), exported (`lib.rs:27`). Use `DhtBuilder::bootstrap(&testnet.bootstrap)` (`lib.rs:376-378` in the crate's own test). That test is `#[ignore = "flaky"]` (`lib.rs:371-372`) [read].
- Server-side checks (`core/server.rs:350-462`): v ≤ 1000 (code 205); salt ≤ 64 (207); CAS mismatch (301); seq < stored (302); bad sig (206) [read].
- Storage retention: `mutable_values` is an LRU of 1000 (`server.rs:34, 138-141`). No timer expiry in the code I read. BEP 44: items "MAY expire in 2 hours" without re-announce; "SHOULD be re-announced once an hour" [read, BEP 44 text].
- Adaptive mode: client first; after 15 min publicly reachable, becomes a server (`n0-mainline/README.md`, `config.rs` doc) [read]. Servers have no rate limiting (README) [read].
- Timing: request timeout = `estimated_rtt + 4*deviation`, RTT floor 500 ms (`actor/socket.rs:26, 506-508`) [read]. A get finishes when no request is in flight (`core/iterative_query.rs:205-214`) [read]. No latency numbers in the sources.
- Put fan-out: to closest nodes that issued tokens; bucket size K = 20 (`put_query.rs:41-66`, `routing_table.rs:10`) [read].
- Who sees what:
  - Storage nodes (K closest with tokens): publisher's UDP IP:port; record = pubkey, relay URL (+ IPs only if unfiltered), seq, sig [inferred from UDP; contents read].
  - Every node on a lookup path: requester's UDP IP:port and target = SHA-1(pubkey) [inferred].
  - Any reader: `get` needs only the target, so anyone can fetch the record for a known key [read, BEP 44 get message].
  - BEP 42: responses SHOULD include top-level `ip` = requester's external IP:port; node IDs derive from the external IP [read, BEP 42]. n0-mainline reads/writes this field (`common/messages.rs:273, 498`) [read].
  - The relay-only filter keeps IPs out of the record, not out of the UDP source address [inferred].

## (c) mDNS (`iroh-mdns-address-lookup` 0.6.0 + swarm-discovery 0.6.1)

- Groups: IPv4 224.0.0.251:5353, IPv6 ff02::fb:5353 (`swarm-discovery/src/socket.rs:12-14`). IPv4 multicast TTL set to 16, not 1 (`socket.rs:146`) [read]. ff02 is link-local for IPv6. A routed network could forward IPv4 TTL-16 multicast [inferred].
- Defaults: `advertise: true`, `filter: AddrFilter::default()`. AddrFilter::default() is `None`, which is identity (`mdns/lib.rs` builder ~171-173; `endpoint_info.rs:242-283`) [read]. So every IP is published.
- Announces: endpoint ID as base32-nopad lowercase in the instance name (`spawn_discoverer`), IP:port records, first relay in `relay=` TXT (`lib.rs:318`), `user-data=` [read].
- Cadence: iroh uses `Discoverer::new_interactive` (τ 0.7 s, φ 2.5) (`swarm lib.rs:244-250`) [read].
- Resolve: returns cached peer immediately; otherwise the stream stays open until the 10 s `LOOKUP_DURATION` timeout (`lib.rs:95`, ~401-438) [read]. So an ID-only dial that only hits mDNS waits 10 s to fail.
- Switches: omit from `address_lookup(...)`; `advertise(false)` (listen-only); `addr_filter(AddrFilter::relay_only())`; `service_name()` (`builder` 180-215) [read].

## (d) Cloudflare Worker as lookup host

- Pkarr relay: PUT/GET `/<z32>` under a base URL (iroh appends z32). Body = relay payload `sig|ts|packet`, ≤1072 B (pkarr spec `design/relays.md`) [read].
- Ed25519: Workers WebCrypto supports Ed25519 sign/verify and importKey (Web Crypto docs table) [read]. Verify `3:seqi<ts>e1:v<len>:<packet>` (the same signable).
- Storage: KV is unsuitable. Same-key writes are limited to 1/s on free and paid; changes take up to 60 s to propagate (KV docs: limits; how-kv-works) [read]. Use a Durable Object per key. SQLite-backed DO storage is "transactional and strongly consistent" (DO storage API doc) [read].
- Stale writes: iroh-dns-server returns 204 whether or not the packet was stored (`http/pkarr.rs:14-35`; `store/signed_packets.rs:149-200`) [read]. Spec says 409 (`design/relays.md`). iroh checks only `is_success()` (`pkarr.rs:675-681`), so 204 is safer for interop [inferred].
- Retention: iroh-dns-server evicts 7 days after the packet timestamp (`store/signed_packets.rs:98, 221-225`) [read].
- Rate limiting: iroh-dns-server uses governor per peer IP (`http/rate_limiting.rs:59-64`) [read]. A Worker needs its own limit (CF rate-limit rule or DO counter).
- DoH: iroh 1.3.0 can use it. iroh-dns depends on n0-dns-resolver with `transport-https` on non-wasm (`iroh-dns/Cargo.toml:124-128`) [read]. Configure `NameserverConfig::https(ip).with_tls_server_name(host)` (`iroh-dns/src/dns.rs:309-345`). The client POSTs `application/dns-message` to `https://<host>:443/dns-query`, pinned to ip (`n0-dns-resolver/src/resolver/transport.rs:278-292`) [read]. iroh-dns-server serves `/dns-query` GET and POST (`http.rs:276`, `http/doh.rs`) [read].
- CPU limits: Free 10 ms per request; Paid default 30 s (max 5 min) (Workers limits doc) [read].
- Effort [inferred]: pkarr relay on Workers + DO ≈150–200 lines JS (verify, compare-and-set on ts, GET, per-IP limit, tests). DoH TXT responder ≈150–250 more.
- Blockers: none fatal. Must use DO not KV. Use Paid plan. PkarrResolver is HTTP-only, so DoH is optional.

## (e) Tickets and MemoryLookup

- `EndpointTicket { addr: EndpointAddr }`, no expiry or version field (`iroh-tickets/src/endpoint.rs:30-31`) [read].
- `connect_with_opts` takes `impl Into<EndpointAddr>`; `From<EndpointTicket> for EndpointAddr` exists (`endpoint.rs:1108-1118`; tickets `endpoint.rs:99`) [read].
- With addresses present, `handle_msg_resolve_remote` inserts them as `Source::App` before resolving; `resolve_remote` returns Ok at once, and the lookup runs in the background (`remote_state.rs:870-880`; `path_state.rs:161-167`) [read].
- MemoryLookup: `add_endpoint_info` appends addresses and dedupes (`memory.rs:184-199`; `endpoint_info.rs:124-131`). Its doc says the relay URL is overwritten (`memory.rs:181-183`), which the code does not do; old relays persist. `set_endpoint_info` replaces. `remove_endpoint_info` retracts (`memory.rs:168-215`) [read].
- Stale: `resolve` returns the stored item with `last_updated`; nothing checks the age (`memory.rs:218-240`; `address_lookup.rs:383` TODO says last_updated is unused) [read].
- Stale IPs stay in the path set until pruning, which runs only at ≥30 non-relay paths (`path_state.rs:18, 256-295`); I found no timestamp-based expiry in `path_state.rs` [read].

## (f) Combining services and blocking

- Multiple lookups: yes. `Builder::address_lookup` pushes onto a Vec (`endpoint.rs:616-618`); `clear_address_lookup` empties it (`:596`) [read].
- Concurrency: all services are queried at once and merged (`address_lookup.rs:565-579`) [read].
- Precedence: none. Every item's addresses join one path set (`remote_state.rs:906-937`). Item `last_updated` is unused and there is no relay-choice rule (`address_lookup.rs:383` TODO) [read]. Path selection later is RTT/bias based (`biased_rtt_path_selector.rs`) [read, filename only].
- Errors: inline errors are yielded and dropped once one item arrives; all-fail → `NoResults` (`address_lookup.rs:548-558, 665-675`) [read].
- Blocking, ID-only dial (no addresses in the EndpointAddr):
  - `resolve_remote` returns at the first item (`path_state.rs:161-207`).
  - Otherwise it waits for every service to yield or end.
  - DNS: ≤≈6 s (`dns.rs` stagger above).
  - mDNS: up to 10 s (`LOOKUP_DURATION`).
  - Pkarr HTTP: no timeout set (`util.rs:21-71`, reqwest builder has only TLS + DNS resolver; `pkarr.rs:620-654`) [read]. A relay that accepts TCP and stalls blocks indefinitely [inferred from no-timeout + merge semantics].
  - DHT: until no request is in flight (`iterative_query.rs:205-214`).
  - The "connect times out after 10 s" comment (`endpoint.rs:1153-1154`) starts after `resolve_remote`. The resolve wait has no timeout in iroh (`endpoint.rs:1146`) [read].
  - Action: wrap `connect` in `tokio::time::timeout` for ID-only dials.

## Other gotchas (read)

- User data is published to every service, unfiltered (`endpoint_info.rs:194-196`). Keep it empty.
- `AddrFilter::relay_only()` also drops custom transport addresses (`endpoint_info.rs:268-271`).
- `iroh-mainline-address-lookup` builder doc says "relay publishing is disabled" by default (`lib.rs:150`). The default filter is relay_only, which does publish the relay. Doc error.
- DHT items are labelled "pkarr", so logs cannot tell DHT from the relay.

## Recommendation (g)

1. On by default: n0 pkarr publish + resolve and n0 DNS (`presets::N0`). Cost: n0 sees ID↔relay mapping, your IP, and every ID you look up; your resolver sees each ID you dial. Switch: `presets::Minimal` or `.clear_address_lookup()`; `disable_fallback()` keeps public resolvers out of DNS. Dropping only `PkarrPublisher` keeps you dial-able only by ticket while still resolving others [inferred].
2. Opt-in: (i) own pkarr relay on Workers: the host sees what n0 sees (Cloudflare terminates TLS) [inferred]; switch = relay URL. (ii) Mainline DHT: relay URL is public per key and refreshed hourly; DHT nodes see your IP; switch = `.address_lookup(DhtAddressLookup::builder())` and `.no_publish()` for lookup-only.
3. Off: mDNS: any LAN host learns endpoint ID and direct IPs, because defaults advertise and do not filter; IPv4 TTL 16. Switch = omit, or `advertise(false)`, or `relay_only`.
