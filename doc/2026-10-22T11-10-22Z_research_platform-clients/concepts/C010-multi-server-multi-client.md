# C010: Multi-server and multi-client topology

[Main reference](../main.md#c010)

## Relevant transcript excerpts

<!-- workflow:excerpt C010 556 557 -->

[Transcript lines 556-557](../transcript.txt#L556-L557)

```text
binary frames; F12.12 many worlds per node; F12.13 journal replication
(tampered stream refused); F12.14 portals (both sides grant); F12.15 `id world
```

<!-- workflow:excerpt C010 681 687 -->

[Transcript lines 681-687](../transcript.txt#L681-L687)

```text
- `id` is **already a real network client, not local-only**: `id world
  join/stats/logs/record/replay` connect to remote nodes over **Iroh (QUIC)**
  with `--addr`/`--no-relay` overrides and relay/discovery (`iroh = "1"`,
  `iroh-mdns-address-lookup`, optional `iroh-mainline-address-lookup`;
  tests disable relays and loop back).
- `id serve` is the server. **Server-to-server transport already exists**:
  iroh-docs replication for records (`world_records.rs`), world sync, LogSync.
```

<!-- workflow:excerpt C010 754 756 -->

[Transcript lines 754-756](../transcript.txt#L754-L756)

```text
## 15. Beyond the terminal: native clients and client-side capabilities (long-term)

The direction set for the platform after the terminal story settles:
```

<!-- workflow:excerpt C010 768 777 -->

[Transcript lines 768-777](../transcript.txt#L768-L777)

```text
- **Multi-server, multi-client** — clients connect to several worlds/servers
  and to other clients at once; `id` as a client will often be a server
  simultaneously (configuration-dependent). **Client-only mode is always
  supported** — to keep web-app load cheap and to serve SSH-only clients.
- **SSH servers in `id`** — rare and opt-in (TCP reachability and performance
  costs; classic SSH supported indefinitely even if ssh3 ever matures).
- **Placement:** this is post-Phase-12 material. F12.7 (ABI conformance),
  F12.14 (portals) and F12.12 (many worlds per node) are the stepping stones;
  a "client shells + client capability providers" sketch belongs in a future
  Phase 13 plan.
```

## Normalized purpose

Takes the topology: one client connected to several worlds, servers and other clients at once, id as a client that is often also a server, client-only mode always supported, and the existing iroh client and server-to-server transports it builds on.

## Initial thoughts

- id is already a real network client over iroh, and server-to-server transport exists (iroh-docs replication, world sync, LogSync) (L681-L687).
- Clients connect to several worlds, servers and other clients at once; id as a client is often a server too (L768-L770).
- Client-only mode is always supported, to keep web-app load cheap and to serve SSH-only clients (L770-L771).
- Stepping stones: F12.12 many worlds per node and F12.14 portals (L556-L557, L774-L777). #536 tracks D1-056, and #564 places it in Phase 13.

## Inferred suggestions

- Inference: One client in many worlds needs per-world capability storage and identity on the client.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
