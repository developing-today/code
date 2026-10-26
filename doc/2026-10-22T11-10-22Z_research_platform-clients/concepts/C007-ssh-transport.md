# C007: SSH as a frame transport and SSH-over-iroh

[Main reference](../main.md#c007)

## Relevant transcript excerpts

<!-- workflow:excerpt C007 190 192 -->

[Transcript lines 190-192](../transcript.txt#L190-L192)

```text
- `exec_request`/`subsystem_request` are **rejected** today
  (`world_ssh.rs:209-225`) — the hook point if a second SSH channel is ever
  wanted.
```

<!-- workflow:excerpt C007 259 260 -->

[Transcript lines 259-260](../transcript.txt#L259-L260)

```text
- **Dual channel** — two live input paths in one SSH session. Not a mode
  switch, not a screen buffer choice.
```

<!-- workflow:excerpt C007 303 310 -->

[Transcript lines 303-310](../transcript.txt#L303-L310)

```text
2. **Two channels on one SSH connection.** SSH multiplexes channels; one is
   the game stream, another a shell-like command line; the client routes by
   pane focus. **Gains:** clean separation; the command channel can be
   scripted, tab-completed, piped into without touching the game stream; one
   auth. **Losses:** needs client-side support (off-the-shelf SSH clients open
   one session channel; `exec_request`/`subsystem_request` are rejected today);
   more moving parts. **Best for:** power users with our own client, or
   automation that drives a world while watching the game stream.
```

<!-- workflow:excerpt C007 378 379 -->

[Transcript lines 378-379](../transcript.txt#L378-L379)

```text
Do not auto-detect from content. Do not build two SSH channels until someone
has a real client for it.
```

<!-- workflow:excerpt C007 467 468 -->

[Transcript lines 467-468](../transcript.txt#L467-L468)

```text
5. **Two SSH channels** (shape 2): would attach at `exec_request` /
   `subsystem_request` (`world_ssh.rs:209-225`); only with a real client.
```

<!-- workflow:excerpt C007 662 727 -->

[Transcript lines 662-727](../transcript.txt#L662-L727)

```text
## 13. Transport: SSH vs Iroh, and what wraps what

### Layer model

Three layers, and only the middle one is shared protocol:

1. **Transport envelope** — SSH channel bytes, Iroh QUIC stream frames
   (length-prefixed, `frames.rs:1-60`), WebSocket messages (`web/world_ws.rs`).
2. **Session protocol** — `ClientFrame`/`ServerFrame` JSON
   (`world_session.rs:107-362`). **Identical under every transport.** SSH
   synthesizes these frames in `world_ssh.rs:351-388`; the CLI's `WorldClient`
   (`world_net.rs:157-964`) speaks them over Iroh. Unwrapping the SSH and Iroh
   envelopes yields the same protocol.
3. **Guest ABI** — `plaza_update`/`plaza_view`/`plaza_layout` Wasm exports
   (`sandbox.rs`). A separate layer (server ↔ guest program), not part of the
   client protocol.

### How `id` connects today

- `id` is **already a real network client, not local-only**: `id world
  join/stats/logs/record/replay` connect to remote nodes over **Iroh (QUIC)**
  with `--addr`/`--no-relay` overrides and relay/discovery (`iroh = "1"`,
  `iroh-mdns-address-lookup`, optional `iroh-mainline-address-lookup`;
  tests disable relays and loop back).
- `id serve` is the server. **Server-to-server transport already exists**:
  iroh-docs replication for records (`world_records.rs`), world sync, LogSync.
- SSH today is a **presentation** adapter (a human terminal), not a transport
  for the frame protocol. **"SSH as transport"** — running the JSON frame
  protocol over an SSH exec channel (`id world join --via ssh`) — is a
  different, future feature for constrained networks and jump hosts.

### SSH (TCP) vs Iroh (QUIC)

| | SSH over TCP | Iroh (QUIC + TLS 1.3) |
|---|---|---|
| Time to first byte | ~3–5 RTT (TCP + version + kex + auth) | 1-RTT handshake; 0-RTT resumption |
| Multiplexing | all channels share one TCP connection → cross-channel head-of-line blocking | independent streams, no HOL blocking |
| Loss behavior | one retransmit stalls everything | per-stream recovery |
| NAT traversal | none — needs a reachable host, jump host, or forward | hole punching + relay fallback |
| Reachability | universal (port 22, firewalls allow it) | UDP may be blocked; relay covers that case |
| Client support | every OS ships one | requires `id`/iroh |
| Identity | host keys, known_hosts | peer IDs are public keys |

On a clean LAN both are fine — our frame-based JSON and per-event full
repaints dominate cost anyway. On lossy or high-RTT links QUIC wins. The
interactive latency story is mostly about **render batching** (§5), not the
transport.

### Tunnel combinations

Both directions are possible; they have different uses:

- **SSH over Iroh** — the useful one: gives third-party SSH clients
  reachability to NAT'd hosts (the "SSH over a tailnet" shape).
- **Iroh over SSH** — only for networks where UDP is blocked but SSH is
  allowed (outbound port 22 only).

Since `id` speaks both natively, tunneling is mostly for non-`id` tools.

### ssh3 status (checked 2026-10-18)

Research implementation (`francoismichel/ssh3`), an IETF draft
(`draft-michel-ssh3-00`), and papers — no OpenSSH adoption. Plan on classic
SSH/TCP being supported **indefinitely**. SSH servers in `id` stay rare and
opt-in (TCP reachability and performance costs).

```

<!-- workflow:excerpt C007 772 773 -->

[Transcript lines 772-773](../transcript.txt#L772-L773)

```text
- **SSH servers in `id`** — rare and opt-in (TCP reachability and performance
  costs; classic SSH supported indefinitely even if ssh3 ever matures).
```

<!-- workflow:excerpt C007 789 791 -->

[Transcript lines 789-791](../transcript.txt#L789-L791)

```text
5. **Priority of "SSH as transport"** (`id world join --via ssh` over an exec
   channel) vs Iroh-only for now? SSH-over-Iroh reachability (§13) — worth a
   Phase 12/13 task?
```

## Normalized purpose

Takes the transport layer: the envelope/session/ABI layer model, SSH versus iroh QUIC, SSH as a transport for the JSON frame protocol over an exec channel (id world join --via ssh), two SSH channels on one connection (shape 2), SSH-over-iroh and iroh-over-SSH tunnels, and ssh3 status.

## Initial thoughts

- Layer model: transport envelope (SSH, iroh QUIC, WebSocket), one session protocol identical under every transport, and a separate guest ABI (L666-L677).
- SSH today is a presentation adapter, not a frame transport; `id world join --via ssh` over an exec channel is a future feature (L688-L691). Exec and subsystem requests are rejected (world_ssh.rs:231-247).
- SSH-over-iroh gives third-party SSH clients reach to NAT'd hosts; iroh-over-SSH only helps where UDP is blocked (L714-L719).
- Two SSH channels (shape 2, L303-L310) only with a real client (L378-L379, L467-L468); #535 does not track it (D1-049 dropped).
- Plan on classic SSH indefinitely; ssh3 is research-stage (L721-L726). #536 tracks D1-050 and D1-051, and #564 places them in Phase 13.

## Inferred suggestions

- Inference: SSH as a frame transport needs a mapping from SSH authentication to capabilities; today the password is the join capability (L140).

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
