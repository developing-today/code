# N30: can an iroh 1.3.0 endpoint refuse a peer before it learns our addresses?

Date: 2026-10-11. Folder: `/home/user/work/research-round2/iroh-allowlist/`.
Pins: `iroh = "=1.3.0"` (with iroh-base 1.3.0), noq 1.3.0, noq-proto 1.3.0, tokio 1.53.2, cargo/rustc 1.97.0. Lockfile copied from `spikes-next/c2-holepunch/Cargo.lock` (locks iroh 1.3.0). Built into the shared `/home/user/work/spikes/target` (BRIEF2 rule 4).
Test host: one Linux host, user and network namespace (`unshare -Urn`), IPv4 only, `RelayMode::Disabled`, `presets::Minimal`, no address lookup.

Labels used below: **[V]** verified by output or source I read, **[I]** inferred, **[N]** not checked.

---

## Questions (verbatim)

Owner's question: "n30 document but allow tickets to have addresses or whatever else they need. if addr aren't needed then fine. i'm not sure what auth peers before connecting means."

Background: "C2 found that candidates are exchanged inside the QUIC connection, so a connected peer learns our addresses. The open question is whether an endpoint can refuse peers before they learn anything."

1. "Read the iroh 1.3.0 source (`~/.cargo/registry/src/*/iroh-1.3.0`, and iroh-relay 1.3.0). Find: the accept path (`Endpoint::accept`, `Incoming`, the connection handshake), and where the remote EndpointId becomes known; any hook or filter that can refuse a connection by remote id (endpoint hooks, connection filters, the address filter, `addr_filter`), and what each one controls; when the path and candidate exchange starts relative to the handshake, and whether the server sends its candidates before the application accepts; when the client learns the server's addresses. Give file and line for each."
2. "Test with two local endpoints (`RelayMode::Disabled`, the Minimal preset, IPv4 loopback). One endpoint refuses an unlisted id. Record what the refused side observes: the error, the timing, and whether it saw any address of the other side. Also record what the refusing side saw before refusing. Run the test in a user namespace if needed."
3. "Answer what 'authorize peers before connecting' can achieve in iroh 1.3.0. Three cases: (a) refuse during the handshake; (b) refuse right after the handshake but before any candidate or path traffic; (c) refuse at the application layer after streams open. For each, what is revealed."
4. "Recommend what we should do for N30, given that the owner accepts addresses in tickets when they are needed."

Output: "RESULTS.md with file-and-line references, the test output, and the recommendation."

---

## Short answers

**What "authorize peers before connecting" means.** It means only peers on our list get a connection. In iroh 1.3.0 that check can run before any packet for outgoing dials (`before_connect`). For incoming peers it can run only after the TLS handshake (`after_handshake`), because that is the first point where the peer's EndpointId is known. By then the server has already queued its candidate addresses. So the list limits who gets application access. It does not stop an unlisted peer from learning our addresses. [V]

**1. Source.** The remote EndpointId becomes known only when the TLS handshake finishes. It is read from the peer's TLS certificate (`connection.rs:369-413`). The only hook that sees it is `EndpointHooks::after_handshake`. That runs after the connection is registered with the socket and before the `Accepting` future resolves (`connection.rs:347-363`, `671-686`). Before the handshake the server has only an `Incoming`: the UDP source, the ALPNs from the ClientHello, and retry state (`connection.rs:130-229`; noq-proto `endpoint.rs:1326-1398`). The Router's `IncomingFilter` sees the same `Incoming` (`protocol.rs:164-215`, `573-593`). The TLS client verifier accepts any valid raw key and holds no id list (`tls/verifier.rs:125-141`; `tls.rs:63`, `116`). `before_connect` runs only for outgoing dials, before any packet (`endpoint.rs:1108-1127`; `hooks.rs:74-78`). `addr_filter` only filters what is published to address-lookup services (`endpoint.rs:628-640`; `address_lookup.rs:529-531`). Candidate exchange starts at connection registration. `handle_msg_add_connection` queues the server's direct addresses as ADD_ADDRESS (`remote_state.rs:387-409`, `1118-1137`), and this completes before the hook runs. The server sends ADD_ADDRESS as soon as it can send 1-RTT data (noq-proto `connection/mod.rs:7109-7116`, `6248-6260`). The client learns the server's addresses from the ticket and from ADD_ADDRESS (noq-proto `connection/mod.rs:5464-5490`; `remote_state.rs:288-290`). The client sends its own candidates in a REACH_OUT round at its own registration, before `connect()` returns (`remote_state.rs:944-958`; noq-proto `n0_nat_traversal.rs:456-470` and `connection/mod.rs:7165-7187`). Holepunching runs on client connections only (`remote_state.rs:517-531`). [V]

**2. Test.** Two processes in one namespace. The server refuses an unlisted id in `after_handshake`. The unlisted client's `connect()` returned `Ok` in all 60 refused runs (loopback and multi-address), and each time its return was logged before the server's hook ran. The server then closed the connection with `ApplicationClosed` code 4097, reason "not on allowlist". In the multi-address runs the client saw that close a median 19 ms after `connect()` returned (range 13.6 to 36.6 ms). In the loopback runs the median was 56 ms. With the server bound to loopback only, the client saw no address beyond its ticket hint (0 of 30 runs). With the server also on two more local addresses (10.200.0.1 and 10.200.0.2), the client received the server's ADD_ADDRESS frames in 17 of 30 runs. Each time it got all three server addresses, two of them new. Its `remote_info` listed a server address in only one of those runs. The refusing server, before it refused, saw the peer's UDP source address (every run), the peer's ALPN before the handshake, and the peer's EndpointId in the hook (every run). In traced runs it also received the peer's three candidate addresses in REACH_OUT frames, before the hook in 10 of 10 runs. Timings come from a shared machine and are indicative only. [V]

**3. The three cases.** (a) **Refuse during the handshake by EndpointId: not possible in 1.3.0.** No hook or filter sees the id before the handshake completes. Refusal before the handshake works only on UDP source, ALPN, or retry. The filter test refused 10 of 10 peers with `Incoming::refuse()`. The client's `connect()` failed after a median 4.5 ms, and no candidates were exchanged (0 of 10). That reveals nothing about us, but it cannot key on an id. (b) **Refuse right after the handshake, before candidate traffic: possible for identity, but not before candidate traffic.** This is the `after_handshake` gate, which is what the test ran. Our candidate frames are queued before the hook in 30 of 30 runs, a median 0.5 ms before it. The refused client's noq state processed them in 17 of 30 runs. The peer's candidate frames (REACH_OUT) reached us before the hook in 10 of 10 traced runs. The refused client's `connect()` succeeds first, so it believes it is connected for about 19 ms. (c) **Refuse at the application layer after streams open: reveals everything in (b) and more.** The refused peer had our full candidate set in 29 of 30 runs. Our server received its REACH_OUT candidates before refusing in 5 of 5 traced runs. [V]

**4. Recommendation.** Keep N30 as a documented exposure, and reword it so it does not promise that the gate prevents exposure. Allow addresses in tickets when a direct path needs them. Connected peers learn our candidates whether or not the ticket holds addresses, so address-free tickets protect only against third parties who read a ticket without connecting. Use the EndpointId list as an `after_handshake` gate for inbound peers, and as a `before_connect` gate for our own outbound dials. Say plainly that neither hides addresses. If the owner needs unlisted peers never to see our IPs, the options are relay-only transports for that group (inferred to send no candidates, not run) or an iroh change that defers candidate queuing until after the hook (not in 1.3.0, not tested). [V for the facts; the advice is mine.]

---

## Evidence

### E1. Source map

Paths are relative to `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/iroh-1.3.0/` unless a row names another crate (noq-proto-1.3.0, noq-1.3.0, netwatch-0.19.3).

| Claim | Where |
|---|---|
| `Endpoint::accept` returns an `Accept` future that yields an `Incoming` on the first Initial packet | `src/endpoint.rs:1181-1186`; `src/endpoint/connection.rs:100-128` (`Accept::poll` builds `Incoming` at 115-118) |
| `Incoming` struct; `accept` / `refuse` / `retry` / `ignore` | `connection.rs:130-136`; `148-152`; `175-177`; `185-194` |
| `Incoming::remote_addr` (UDP source, before any handshake) | `connection.rs:204-213` |
| `Incoming::decrypt` (ClientHello bytes, via ALPN iterator) | `connection.rs:223-229`; noq-proto `src/endpoint.rs:1326-1336` (`DecryptedInitial::alpns`), `1378-1398` (`IncomingAlpns` iterator) |
| `Incoming` into a connection: `conn_from_noq_conn` | `connection.rs:232-243`; `317-367` |
| Identity from the TLS certificate (`peer_identity`) | `connection.rs:369-373` (`static_info_from_conn`), `383-413` (`remote_id_from_noq_conn`) |
| Registration with the socket, then the hook | `connection.rs:347` (`register_connection`), `352` (`fut.await?`), `358-363` (`after_handshake`, Reject closes and returns `LocallyRejected`) |
| `Accepting` yields only after registration and the hook | `connection.rs:671-686` |
| Client side (`Connecting`) goes through the same path | `connection.rs:574-587`; `541` (0-RTT path) |
| `EndpointHooks` trait; what `after_handshake` knows ("the remote's endpoint id and ALPN") | `src/endpoint/hooks.rs:68-108`, with the doc at `87-101` |
| Hook list semantics (first Reject wins) | `hooks.rs:133-171` |
| `before_connect`: outgoing only, "before any packets are sent" | `hooks.rs:74-85`; `src/endpoint.rs:1108-1127` (called at `1124-1126`) |
| `Router::incoming_filter`: sees `&Incoming`, returns Accept/Retry/Reject/Ignore | `src/protocol.rs:164-215`; `466-480` (setter, doc says it runs before the handshake completes); `573-593` (applied per Incoming) |
| TLS client verifier: no id check, only intermediates | `src/tls/verifier.rs:114-141` (comment at 137); `src/tls.rs:63`, `116` (fixed, no public setter) |
| TLS server verifier: server cert must match the dialed id | `src/tls/verifier.rs:33-75` |
| `addr_filter`: a builder option that applies at bind, to address lookup only | `src/endpoint.rs:628-640` (doc), `306-307` (applied to the `AddressLookupServices`); `src/address_lookup.rs:464-489` (struct and setter at 488), `529-531` (`publish` applies the filter) |
| Registration queues local candidates | `src/socket/remote_map/remote_state.rs:387-409` (`handle_msg_add_connection`; `update_qnt_candidates` called at 409); `src/socket.rs:1355-1366` (`register_connection`) |
| `update_qnt_candidates` calls noq `add_nat_traversal_address` for each direct address | `remote_state.rs:1105-1137` (`local_candidates` at 1105; `update_qnt_candidates` at 1118; add at 1127) |
| Candidates are local direct addresses; loopback-flagged interfaces count as local | `netwatch-0.19.3/src/interfaces/netdev_impl.rs:190-213` (addresses on loopback interfaces go to `loopback`); `iroh-1.3.0/src/socket.rs:1744-1748` (hint uses `regular` and `loopback`) |
| Holepunching (and so REACH_OUT) runs on client connections only | `remote_state.rs:517-531` (`trigger_holepunching`, filter at 527) |
| The client reacts to ADD_ADDRESS by triggering holepunching | `remote_state.rs:288-290` (addr_events; trace log at 289) |
| The initial round at registration: `do_holepunching` runs with possibly empty remote candidates | `remote_state.rs:944-958` (`initiate_nat_traversal_round` at 947; `iroh::_events::qnt::init` event at 954); failure at 967 |
| ADD_ADDRESS is queued and sent in 1-RTT packets | noq-proto `src/connection/mod.rs:7109-7116` (queue), `6248-6260` (send) |
| ADD_ADDRESS receipt (client-side state) | noq-proto `src/connection/mod.rs:5464-5490` |
| REACH_OUT receipt (server-side state) | noq-proto `src/connection/mod.rs:5507-5530` |
| REACH_OUT is produced from local addresses, even with no remote candidates | noq-proto `src/n0_nat_traversal.rs:456-470` (returns `NotEnoughAddresses` only if no local address); `src/connection/mod.rs:7165-7187` (frames appended to `pending.reach_out`); observed: see E4 |
| `handle_reach_out` logs nothing on the success path | noq-proto `src/n0_nat_traversal.rs:789-832` (trace only for stale rounds, 797 and 801) |
| Server advertises ADD_ADDRESS, client advertises via REACH_OUT | noq-proto `src/n0_nat_traversal.rs:200-217` (doc at 200-204) |
| Public remote info (paths, with usage) | `src/endpoint.rs:1644-1652` (`remote_info`); `src/socket.rs:1337-1347`; `src/socket/remote_map/remote_state.rs:370-380`; `src/socket/remote_map/remote_state/remote_info.rs:11` (`RemoteInfo`), `89` (`TransportAddrUsage`: Active / Inactive) |
| `preferred_address` (the handshake-time address carrier) is not set by iroh | grep of `iroh-1.3.0/src` and `noq-1.3.0/src`: no setter call. Only noq-proto `src/config/mod.rs:224-252`, `303-312`, `374` (defaults `None`) [V] |
| No public accessor for a remote's NAT candidate list | grep `pub fn .*nat` in `iroh-1.3.0/src`: only `src/endpoint/quic.rs:538` (a config knob) [V] |

### E2. Commands

```
# build: pinned toolchain, shared target dir (BRIEF2 rule 4)
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
CARGO_TARGET_DIR=/home/user/work/spikes/target "$CARGO" build
#   Compiling iroh-allowlist-spike v0.0.0 ...
#   Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.86s   (exit 0)

# main runs: each run is a fresh server and a fresh client, both inside `unshare -Urn`
bash lab/run.sh loopback-refuse 30
bash lab/run.sh multi-refuse 30
bash lab/run.sh multi-allow 10
bash lab/run.sh multi-app 30
bash lab/run.sh filter-refuse 10
python3 -I lab/summarize.py logs > logs/SUMMARY.txt

# debug runs: qnt::init events and noq debug output
RUST_LOG='harness=info,iroh::_events::qnt::init=debug,iroh::socket::remote_map::remote_state=trace,noq_proto=debug,iroh=warn' \
  LOG_ROOT=$PWD/logs-debug bash lab/run.sh multi-app 5      # and multi-refuse 5

# trace runs: noq frame trace (REACH_OUT), with remote_state trace kept on
RUST_LOG='harness=info,noq_proto=trace,iroh::socket::remote_map::remote_state=trace,iroh=warn' \
  LOG_ROOT=$PWD/logs-trace bash lab/run.sh multi-refuse 10  # and multi-app 5
python3 -I lab/summarize.py logs-trace multi-refuse multi-app > logs-trace/SUMMARY.txt
```

Namespace setup, inside `lab/run.sh`: `ip link set lo up`, then `ip addr add 10.200.0.1/32 dev lo` and `ip addr add 10.200.0.2/32 dev lo`. Anything bound to `0.0.0.0` then advertises these as direct addresses (E1, netdev classification).

The ticket the client gets is `EndpointAddr::new(server_id).with_ip_addr(127.0.0.1:PORT)`. Nothing else.

| Scenario | Server bind and mode | Client (seed, bind, role) | Purpose |
|---|---|---|---|
| `loopback-refuse` | 127.0.0.1:0, refuse in `after_handshake` | seed 3 (unlisted), 127.0.0.1:0, `connect` | the literal spec: IPv4 loopback only |
| `multi-refuse` | 0.0.0.0:0, refuse | seed 3, 0.0.0.0:0, `connect` | candidates beyond the ticket hint |
| `multi-allow` | 0.0.0.0:0, refuse | seed 2 (allowed), 0.0.0.0:0, opens a stream | control: allowed peer |
| `multi-app` | 0.0.0.0:0, accept handshake, refuse after first stream | seed 3, 0.0.0.0:0, opens a stream | case (c) |
| `filter-refuse` | 0.0.0.0:0, `Incoming::refuse()` before handshake | seed 3, 0.0.0.0:0, `connect` | case (a) |

Keys are fixed seeds: server 1, allowed client 2, unlisted client 3. The allowlist holds seed 2's id.

### E3. Test output

Copied from `logs/SUMMARY.txt` (generated by `lab/summarize.py`, which uses only the log lines) and `logs-trace/SUMMARY.txt`.

```
== filter-refuse (10 runs) ==
server  hook ran: 0/10 (0%, 95% Wilson 0-28%); allowed flag at hook: 0 allowed of 0
server  refused in hook: 0/10 (0%, 95% Wilson 0-28%); handed to application: 0/10 (0%, 95% Wilson 0-28%); refused after stream: 0/10 (0%, 95% Wilson 0-28%); refused before handshake: 10/10 (100%, 95% Wilson 72-100%)
server  candidates queued before hook: 0/10 (0%, 95% Wilson 0-28%); queue-to-hook gap n/a
server  first Initial packet to candidates queued (server-side handshake) n/a
server  hook-to-refusal gap n/a
client  connect() returned Ok: 0/10 (0%, 95% Wilson 0-28%); connect() error: 10/10 (100%, 95% Wilson 72-100%)
client  connect() error text: {'aborted by peer: the server refused to accept a new connection': 10}
client  connect() return time n/a
client  connect() error time median 4.5 ms (min 4.0, max 11.4, n=10)
client  peer close observed time from start n/a; connect-return to close n/a
client  close message(s): none
client  saw >=1 server AddressAdded before/at close: 0/10 (0%, 95% Wilson 0-28%); of those, AddressAdded before the close event: 0
client  saw a server address NOT in the ticket (new information): 0/10 (0%, 95% Wilson 0-28%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 0/10 (0%, 95% Wilson 0-28%)

== loopback-refuse (30 runs) ==
server  hook ran: 30/30 (100%, 95% Wilson 89-100%); allowed flag at hook: 0 allowed of 30
server  refused in hook: 30/30 (100%, 95% Wilson 89-100%); handed to application: 0/30 (0%, 95% Wilson 0-11%); refused after stream: 0/30 (0%, 95% Wilson 0-11%); refused before handshake: 0/30 (0%, 95% Wilson 0-11%)
server  candidates queued before hook: 30/30 (100%, 95% Wilson 89-100%); queue-to-hook gap median 0.5 ms (min 0.3, max 7.4, n=30)
server  first Initial packet to candidates queued (server-side handshake) median 134.1 ms (min 58.5, max 248.8, n=30)
server  hook-to-refusal gap median 0.4 ms (min 0.3, max 18.0, n=30)
client  connect() returned Ok: 30/30 (100%, 95% Wilson 89-100%); connect() error: 0/30 (0%, 95% Wilson 0-11%)
client  connect() return time median 90.7 ms (min 35.5, max 184.9, n=30)
client  connect() returned before the server's hook ran: 30/30 (100%, 95% Wilson 89-100%)
client  peer close observed time from start median 147.1 ms (min 75.0, max 287.9, n=30); connect-return to close median 56.4 ms (min 21.4, max 109.2, n=30)
client  close message(s): {'closed by peer: not on allowlist (code 4097)': 30}
client  saw >=1 server AddressAdded before/at close: 5/30 (17%, 95% Wilson 7-34%); of those, AddressAdded before the close event: 5
client  saw a server address NOT in the ticket (new information): 0/30 (0%, 95% Wilson 0-11%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 0/30 (0%, 95% Wilson 0-11%)

== multi-allow (10 runs) ==
server  hook ran: 10/10 (100%, 95% Wilson 72-100%); allowed flag at hook: 10 allowed of 10
server  refused in hook: 0/10 (0%, 95% Wilson 0-28%); handed to application: 10/10 (100%, 95% Wilson 72-100%); refused after stream: 0/10 (0%, 95% Wilson 0-28%); refused before handshake: 0/10 (0%, 95% Wilson 0-28%)
server  candidates queued before hook: 10/10 (100%, 95% Wilson 72-100%); queue-to-hook gap median 0.5 ms (min 0.2, max 1.2, n=10)
server  first Initial packet to candidates queued (server-side handshake) median 32.1 ms (min 27.8, max 67.8, n=10)
server  hook-to-refusal gap n/a
client  connect() returned Ok: 10/10 (100%, 95% Wilson 72-100%); connect() error: 0/10 (0%, 95% Wilson 0-28%)
client  connect() return time median 23.5 ms (min 18.7, max 49.8, n=10)
client  connect() returned before the server's hook ran: 10/10 (100%, 95% Wilson 72-100%)
client  peer close observed time from start n/a; connect-return to close n/a
client  close message(s): none
client  stream reply(s): {'welcome': 10}
client  saw >=1 server AddressAdded before/at close: 10/10 (100%, 95% Wilson 72-100%); of those, AddressAdded before the close event: 0
client  saw a server address NOT in the ticket (new information): 10/10 (100%, 95% Wilson 72-100%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 0/10 (0%, 95% Wilson 0-28%)
client  example new addresses (run-01): ['10.200.0.1:41743', '10.200.0.2:41743']

== multi-app (30 runs) ==
server  hook ran: 30/30 (100%, 95% Wilson 89-100%); allowed flag at hook: 0 allowed of 30
server  refused in hook: 0/30 (0%, 95% Wilson 0-11%); handed to application: 30/30 (100%, 95% Wilson 89-100%); refused after stream: 30/30 (100%, 95% Wilson 89-100%); refused before handshake: 0/30 (0%, 95% Wilson 0-11%)
server  candidates queued before hook: 30/30 (100%, 95% Wilson 89-100%); queue-to-hook gap median 0.7 ms (min 0.2, max 1.6, n=30)
server  first Initial packet to candidates queued (server-side handshake) median 34.4 ms (min 28.9, max 71.9, n=30)
server  hook-to-refusal gap n/a
client  connect() returned Ok: 30/30 (100%, 95% Wilson 89-100%); connect() error: 0/30 (0%, 95% Wilson 0-11%)
client  connect() return time median 23.1 ms (min 18.5, max 50.4, n=30)
client  connect() returned before the server's hook ran: 30/30 (100%, 95% Wilson 89-100%)
client  peer close observed time from start median 43.7 ms (min 33.0, max 86.5, n=30); connect-return to close median 18.0 ms (min 14.5, max 36.1, n=30)
client  close message(s): {'closed by peer: not on allowlist (code 4098)': 30}
client  stream error(s): {'read error: connection lost': 30}
client  saw >=1 server AddressAdded before/at close: 29/30 (97%, 95% Wilson 83-99%); of those, AddressAdded before the close event: 28
client  saw a server address NOT in the ticket (new information): 29/30 (97%, 95% Wilson 83-99%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 6/30 (20%, 95% Wilson 10-37%)
client  example new addresses (run-01): ['10.200.0.1:38631', '10.200.0.2:38631']

== multi-refuse (30 runs) ==
server  hook ran: 30/30 (100%, 95% Wilson 89-100%); allowed flag at hook: 0 allowed of 30
server  refused in hook: 30/30 (100%, 95% Wilson 89-100%); handed to application: 0/30 (0%, 95% Wilson 0-11%); refused after stream: 0/30 (0%, 95% Wilson 0-11%); refused before handshake: 0/30 (0%, 95% Wilson 0-11%)
server  candidates queued before hook: 30/30 (100%, 95% Wilson 89-100%); queue-to-hook gap median 0.5 ms (min 0.3, max 4.9, n=30)
server  first Initial packet to candidates queued (server-side handshake) median 37.3 ms (min 30.9, max 69.3, n=30)
server  hook-to-refusal gap median 0.6 ms (min 0.3, max 4.8, n=30)
client  connect() returned Ok: 30/30 (100%, 95% Wilson 89-100%); connect() error: 0/30 (0%, 95% Wilson 0-11%)
client  connect() return time median 27.0 ms (min 21.7, max 55.9, n=30)
client  connect() returned before the server's hook ran: 30/30 (100%, 95% Wilson 89-100%)
client  peer close observed time from start median 47.4 ms (min 37.3, max 77.8, n=30); connect-return to close median 19.2 ms (min 13.6, max 36.6, n=30)
client  close message(s): {'closed by peer: not on allowlist (code 4097)': 30}
client  saw >=1 server AddressAdded before/at close: 17/30 (57%, 95% Wilson 39-73%); of those, AddressAdded before the close event: 14
client  saw a server address NOT in the ticket (new information): 17/30 (57%, 95% Wilson 39-73%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 6/30 (20%, 95% Wilson 10-37%)
client  example new addresses (run-02): ['10.200.0.1:38337', '10.200.0.2:38337']
```

Trace runs (`logs-trace/SUMMARY.txt`; the trace logging slows the server, so its handshake timings are about 3 to 4 times the plain runs):

```
== multi-refuse (10 runs) ==
server  hook ran: 10/10; allowed flag at hook: 0 allowed of 10
server  refused in hook: 10/10 (100%, 95% Wilson 72-100%); handed to application: 0/10; refused before handshake: 0/10
server  candidates queued before hook: 10/10 (100%, 95% Wilson 72-100%); queue-to-hook gap median 0.7 ms (min 0.5, max 7.4, n=10)
server  first Initial packet to candidates queued (server-side handshake) median 127.6 ms (min 92.1, max 178.8, n=10)
server  hook-to-refusal gap median 0.6 ms (min 0.5, max 11.3, n=10)
server  REACH_OUT received (trace runs only): 10/10 (100%); received before hook 10/10 (100%); before refusal 10/10 (100%)
client  connect() returned Ok: 10/10 (100%); connect() error: 0/10
client  connect() return time median 91.1 ms (min 71.3, max 124.8, n=10)
client  connect() returned before the server's hook ran: 10/10 (100%)
client  peer close observed time from start median 156.5 ms (min 121.2, max 208.5, n=10); connect-return to close median 60.3 ms (min 42.7, max 99.5, n=10)
client  close message(s): {'closed by peer: not on allowlist (code 4097)': 10}
client  saw >=1 server AddressAdded before/at close: 1/10 (10%, 95% Wilson 2-40%)
client  saw a server address NOT in the ticket (new information): 1/10 (10%, 95% Wilson 2-40%)
client  example new addresses (run-02): ['10.200.0.1:43082', '10.200.0.2:43082']

== multi-app (5 runs) ==
server  hook ran: 5/5 (100%); allowed flag at hook: 0 allowed of 5
server  refused in hook: 0/5; handed to application: 5/5 (100%); refused after stream: 5/5 (100%)
server  candidates queued before hook: 5/5 (100%); queue-to-hook gap median 0.8 ms (min 0.6, max 7.7, n=5)
server  first Initial packet to candidates queued (server-side handshake) median 124.9 ms (min 78.5, max 166.3, n=5)
server  REACH_OUT received (trace runs only): 5/5 (100%); received before hook 5/5 (100%); before refusal 5/5 (100%)
client  connect() returned Ok: 5/5 (100%); connect() error: 0/5
client  connect() returned before the server's hook ran: 5/5 (100%)
client  close message(s): {'closed by peer: not on allowlist (code 4098)': 5}
client  stream error(s): {'read error: connection lost': 5}
client  saw >=1 server AddressAdded before/at close: 5/5 (100%); saw a server address NOT in the ticket: 5/5 (100%)
client  holepunch attempt failed ('connection is already closed' etc.) in: 3/5 (60%, 95% Wilson 23-88%)
client  example new addresses (run-1): ['10.200.0.1:60063', '10.200.0.2:60063']
```

Notes on these numbers:
- The "AddressAdded before the close event" counts are not a reliable order measure at this resolution. The AddressAdded lines come from iroh's remote-state actor, which can run after the application sees the close. The leak counts (at least one `AddressAdded` event) come from noq having processed an ADD_ADDRESS frame, so they do not depend on that ordering.
- An earlier trace batch used `iroh=warn` without enabling `iroh::socket::remote_map::remote_state` at trace level. That hid all `AddressAdded` lines, and it showed "0 of 10" as a result. That was a filter artifact, not a result. It was rerun with the trace enabled, and only the rerun is reported above.

### E4. Log excerpts

Leaking run, `logs/multi-refuse/run-02`, with the harness prefix trimmed. The server queues its candidates, its hook refuses, and the client receives three AddressAdded events after `connect()` has returned.

```
SERVER
17:22:39.833160Z INFO harness: first Initial packet; no EndpointId is known yet  event="incoming"  remote=Ip(127.0.0.1:42350)  alpns=["n30/allowlist/0"]
17:22:39.889465Z TRACE remote_state: updated local QNT addresses   [10.200.0.1, 10.200.0.2, 127.0.0.1 on port 38337]
17:22:39.891089Z INFO harness: hook runs  event="after_handshake"  peer=ed4928c628  allowed=false  enforce=true
17:22:39.895851Z INFO harness: no Connection was handed to the application  event="handshake_refused"  error=Connection was rejected locally  remote_info_after_refusal=["Ip(127.0.0.1:42350)/Active"]

CLIENT
17:22:39.826904Z INFO harness: dialing with one loopback hint  event="start"  ticket=[127.0.0.1:38337]
17:22:39.863568Z TRACE remote_state: updated local QNT addresses   [client's own three addresses]
17:22:39.869958Z INFO harness: connect() returned a Connection  event="connect_ok"  elapsed_ms=42.9  remote_info=["Ip(127.0.0.1:38337)/Active"]
17:22:39.892596Z TRACE remote_state: remote addrs updated, triggering holepunching  evt=Ok(AddressAdded(127.0.0.1:38337))
17:22:39.892822Z TRACE remote_state: remote addrs updated, triggering holepunching  evt=Ok(AddressAdded(10.200.0.2:38337))
17:22:39.892950Z TRACE remote_state: remote addrs updated, triggering holepunching  evt=Ok(AddressAdded(10.200.0.1:38337))
17:22:39.897418Z INFO harness: peer closed  event="closed_by_peer"  elapsed_ms=70.4  why=closed by peer: not on allowlist (code 4097)
17:22:40.098774Z INFO harness: final snapshot  remote_info=["Ip(127.0.0.1:38337)/Active"]
```

Ordering of REACH_OUT, the client's candidates, in `logs-trace/multi-app/run-1` (noq frame trace) [V]:

```
CLIENT 17:31:27.119416Z TRACE noq_proto::connection::packet_builder: frame=REACH_OUT round: 1 local_addr: 10.200.0.2:33516
CLIENT 17:31:27.119449Z ... frame=REACH_OUT round: 1 local_addr: 127.0.0.1:33516
CLIENT 17:31:27.119461Z ... frame=REACH_OUT round: 1 local_addr: 10.200.0.1:33516
SERVER 17:31:27.152097Z TRACE noq_proto::connection: got frame REACH_OUT round: 1 local_addr: 10.200.0.2:33516
SERVER 17:31:27.152144Z ... got frame REACH_OUT round: 1 local_addr: 127.0.0.1:33516
SERVER 17:31:27.152169Z ... got frame REACH_OUT round: 1 local_addr: 10.200.0.1:33516
SERVER 17:31:27.193261Z INFO harness: hook runs  event="after_handshake"  allowed=false  enforce=false
SERVER 17:31:27.194985Z INFO harness: stream from peer  event="stream_opened"  first_bytes=hello
SERVER 17:31:27.195143Z INFO harness: refusing after the stream opened  event="app_refusal"
```

The client's registration-time round, `logs-debug/multi-app/run-1` (the `qnt::init` event at DEBUG) [V]:

```
17:26:37.722638Z TRACE remote_state: handle_message{msg=AddConnection ...}: updated local QNT addresses
17:26:37.722925Z DEBUG ...do_holepunching: iroh::_events::qnt::init: remote=8a88e3dd74 local_candidates={10.200.0.1:40807, 10.200.0.2:40807, 127.0.0.1:40807} remote_candidates={}
17:26:37.725847Z INFO harness: connect() returned a Connection  event="connect_ok"  elapsed_ms=158.2
```

So the client sends its candidates (REACH_OUT) during registration, before `connect()` returns, with no remote candidates known yet. Its second round, after AddressAdded, has `remote_candidates={10.200.0.1:34455, 10.200.0.2:34455, 127.0.0.1:34455}`.

### E5. Other checks

- **When the client learns the server's addresses.** The client's first `AddressAdded` came 13.9 ms (multi-allow), 15.7 ms (multi-refuse) and 16.0 ms (multi-app) after `connect()` returned, as medians. In no run did it come before `connect()` returned. [V]
- **Handshake-time address carriers.** iroh never calls the noq-proto `preferred_address` setter (E1). The ServerHello and certificate carry no addresses. [V for iroh; the TLS content is I]
- **Public visibility of the leaked candidates.** In the refused runs, `remote_info` showed only the ticket address in 29 of the 30 `multi-refuse` runs. In one of them it listed `10.200.0.2` and `10.200.0.1` as Inactive (paths the client was probing). In one `multi-app` run it listed `10.200.0.1` as Active, which is a live path to the server's second address. [V]
- **Server-side view before the hook.** `Incoming` shows `remote=Ip(UDP source)` and `alpns=["n30/allowlist/0"]`, with no EndpointId (every run). `remote_info` on the server, read after refusal, listed only the client's UDP source. [V]
- **Filter-refuse.** `Incoming::refuse()` produces a `connect()` error after 4.5 ms (median). Its text is "aborted by peer: the server refused to accept a new connection". No candidate was exchanged. [V]

---

## Options

| Option | What it stops | What it still reveals | Achievable in iroh 1.3.0? | Verdict |
|---|---|---|---|---|
| A. Document as an accepted exposure; tickets may carry addresses when a direct path needs them | Nothing new | Our candidates to any peer that completes the handshake, unlisted ones included (17/30 runs in the lab) | Yes, it is the status quo | **Recommended** (with the wording below) |
| B. EndpointId allowlist in `after_handshake` (inbound) | Application access and group state for unlisted peers | Candidates still leak in the race (E3), and the peer's candidates reach us (E4) | Yes (`hooks.rs:102-108`; `connection.rs:358-363`) | **Do it**, but call it an access gate, not address privacy |
| C. EndpointId allowlist in `before_connect` (outbound dials) | Any packet to an unlisted server, from us | Nothing on the inbound side | Yes (`endpoint.rs:1124-1126`; `hooks.rs:74-78`). Source only; not run [N] | **Recommended** for our own dials |
| D. Refuse inbound by EndpointId before the handshake | (would stop everything) | (would reveal nothing) | **No**: no identity before the handshake (E1; `connection.rs:130-229`; `tls/verifier.rs:125-141`) | Not possible |
| E. Pre-handshake refusal by UDP source, ALPN, or retry (`IncomingFilter`) | Unsolicited floods, wrong-ALPN peers | Nothing about us (filter test: 0 of 10 exchanges) | Yes (E3 filter-refuse) | Use for hygiene only; cannot key on an id |
| F. Relay-only transport for groups that need address privacy (`clear_ip_transports`) | Candidates: no direct address means no ADD_ADDRESS and no REACH_OUT | The relay sees traffic; direct paths are lost | Config yes. Inferred to send no candidates: `initiate_nat_traversal_round` returns `NotEnoughAddresses` with no local address (`n0_nat_traversal.rs:457-459`). Not run [N] | Option for specific groups; costs direct paths |
| G. Patch iroh (or fork): queue local candidates only after `after_handshake` accepts (move `update_qnt_candidates` after the hook, `remote_state.rs:387-409`, `connection.rs:358-363`) | Candidates to refused peers | Fork or upstream change to maintain; the server still sees the peer's UDP source | Not in 1.3.0. Not built or tested [N] | Not now; record as the only route to a strict guarantee |
| H. `addr_filter` set to relay-only | Nothing on connections | Filters only lookup publishing (`address_lookup.rs:529-531`) | Yes, but irrelevant to connected peers | Use it for public DNS/DHT policy only |

---

## Recommendation

**Reasons.**

1. The EndpointId check does what the owner wants for access. When it runs in `after_handshake`, an unlisted peer never gets a connection to the application (`loopback-refuse` and `multi-refuse`, 60 of 60). When the check runs after the first stream instead (`multi-app`), the unlisted peer can open one stream and send one message before it is refused (30 of 30). The hook placement is the one to use. The check is cheap and should stay.
2. It cannot do what "before they learn anything" would need. Candidate queuing precedes the hook by design (E1), and nothing in 1.3.0 identifies a peer before its handshake. So "authorize before connecting" should not be presented as address privacy.
3. Tickets are not where the leak is. A connected peer learns our candidates whether or not the ticket holds addresses (the test ticket held only a loopback hint, and the candidates still went out: to the accepted peer in 10 of 10 runs and to the refused peer in 17 of 30). So "allow addresses in tickets when needed" adds exposure only to whoever holds the ticket, who can read it without connecting. That is a real but separate exposure, and it is acceptable when the ticket goes only to invited peers. Address-free tickets limit third-party lookup exposure. They do not protect against connected peers.
4. The outbound side can genuinely be "authorized before connecting": `before_connect` stops any packet to an unlisted server (Option C). That is the honest meaning of the phrase for our own dials.

**Proposed N30 wording** (for the design index row and the main design's N30 note):

> **N30. Address exposure to connected peers.** A peer that completes the TLS handshake with us learns our direct addresses (every local IP we bind), and so does an unlisted peer. The candidates are queued right after the handshake, before the EndpointId check (`after_handshake`) runs. In the lab, an unlisted peer got them in 17 of 30 refused attempts. The peer's candidates reach us in the same window. Tickets may carry addresses when a direct path needs them. Those are unsigned hints (N24), and they expose addresses to whoever holds the ticket. For address privacy from a group, use relay-only transports for it, at the cost of direct paths.

**Replace in the index row for N30:** the option "Authorize peers before connecting (invite first), so only trusted peers connect" should read "Gate by EndpointId after the handshake (access only). Address privacy needs relay-only transports or a patched iroh (not in 1.3.0)."

**What would change this recommendation.**

- If the owner needs unlisted peers never to see our IPs, the answer becomes Option F (relay-only for that group) or G (a patched iroh). Neither is built or tested here.
- If real networks show a much lower or higher leak rate, the wording "in about half of refusals" needs updating. The outcome depends on whether the queued candidate frame leaves before the close is queued. In the lab that gap is about 1 ms (queue-to-hook median 0.5 ms, hook-to-refusal median 0.6 ms). That is local scheduling rather than network round-trip time, so the rate may differ on a real network. This is inferred [I].
- If iroh adds a pre-candidate or per-connection candidate control, Option G becomes an iroh option rather than a fork.
- If the owner accepts that any direct peer sees our IP, the N30 question narrows to the candidate list, which is what the wording above covers.

---

## Not verified

- **[N] Real networks.** This is one host, loopback and two local addresses. No NAT, IPv6, multi-NIC host, or relay (`RelayMode::Disabled`). The leak rate is a local race and was measured on a shared machine. Trace runs add overhead.
- **[N] iroh-relay 1.3.0 source.** I listed its files but did not read the relay code. The test does not use a relay. Relay-side identity or auth would live there, and it is outside this test.
- **[N] Outbound `before_connect` refusal** (Option C): source only. Not run.
- **[N] `IncomingFilterOutcome::Ignore` and `Retry`**: source only. Not run.
- **[N] ClientHello contents beyond ALPN** (SNI, key shares, other extensions). `decrypt()` exposes only the ALPN iterator (noq-proto `endpoint.rs:1334-1398`), so I did not inspect the bytes.
- **[I] TLS 1.3 placement of the client's Certificate** (after the server's flight, under handshake keys). This is standard TLS 1.3 behaviour (RFC 8446 §2, §4.4.2). I did not fetch the RFC in this run.
- **[N] Server-side REACH_OUT handling after receipt.** I observed receipt, not what the server did next (probe scheduling, path opens). One plain `multi-app` run showed an Active client path to the server's 10.200.0.1.
- **[N] One `multi-app` run with no AddressAdded** (`logs/multi-app/run-28`). I did not investigate why.
- **[N] Effect of close timing or error code on the race.** Not varied.
- **[I] Relay-only candidate absence.** Inferred from `clear_ip_transports` and `initiate_nat_traversal_round` (E1), not run.
- **[N] Timing values as absolutes.** The first batch (`loopback-refuse`) ran while other agents were building, and its connect medians (about 91 ms) are three to four times the later batches (23 to 27 ms). Use the ratios and the counts, not the absolute times.
