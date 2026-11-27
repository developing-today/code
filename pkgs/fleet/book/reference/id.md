# fleet id

`fleet id` is the Iroh-native ID transport of design #10391 (pass 2).
It gives every fleet session an ed25519 node key — its NodeId is the
session's public identity on the ID plane — and moves bytes to ID
peers under the design's mode rule. The registry summary's older
wording ("proxy, postcard, stream and serve-ws verbs land as later
slices") is historical: `session-key`, `proxy-mode` and `proxy` have
landed (slices 1–2, #15430/#15434); the initiator verbs served by
`fleet id postcard` / `fleet id stream` are the mode-3 surface the
proxy verb names, not this domain's own verbs yet.

## The session key contract (§4.1)

- **Default: one ephemeral ed25519 node key per session**, generated
  from the OS RNG at session start. Never hardcoded, checked in, or
  logged.
- **Caller override:** `--id-secret` / `FLEET_ID_SECRET` supplies the
  32-byte secret as 64 hex characters; fleet uses it verbatim and
  never persists it. Any other form is rejected
  ("secret must be 64 hex characters (32 bytes)").
- **Stamping:** the public key (NodeId) travels with the #10357
  context stamp: as `fleet_node_id` on the queue stamp and as
  `FLEET_ID_NODE_ID` in the dot-sourced session env. The secret is
  exported only as `FLEET_ID_SECRET` in that same in-session env
  rendering — it is never in the stamp, and the domain has no
  log/trip-record rendering that could carry it.
- **Storage:** process memory only, in a zeroizing buffer; the key is
  zeroized when the command exits. No disk write, no secret-store
  write.
- **Rotation/disposal:** keys are single-session; rotation is a new
  session. Nothing is published beyond ticket holders, so there is
  no revocation protocol.

The NodeId derivation oracle is ed25519-dalek's `SigningKey` — the
same key type iroh's `SecretKey` wraps, so a NodeId minted here is
the NodeId iroh presents for the same secret.

## fleet id session-key

```
fleet id session-key [--id-secret <hex>] [--stamp | --node-id]
```

Mints (or adopts, via `--id-secret` / `FLEET_ID_SECRET`) this
session's iroh key and renders its stamp:

- (default) the dot-sourced env lines: `FLEET_ID_NODE_ID` plus
  `FLEET_ID_SECRET` (in-session only);
- `--stamp` — the queue stamp JSON: `{"fleet_node_id": …}` — public
  key only, never the secret;
- `--node-id` — just the NodeId hex.

Exit codes: 0 ok · 2 usage.

## fleet id proxy-mode

```
fleet id proxy-mode [--raw] [--opaque] [--alpn <alpn>] [--mode <1|2|3|name>]
```

Reports the §4.2 mode a proxy invocation would take, under the mode
rule below. This is the selection half of `fleet id proxy`, usable on
its own to see which mode a target/flag combination resolves to.

## fleet id proxy

```
fleet id proxy [--raw | --opaque | --mode <1|2|3>] [--alpn <alpn>]
               [--relay <url>] [--expect-node-id <hex>] <target>
```

Byte-move to an ID peer under the §4.2 mode rule (first match wins):
`--mode` overrides all; `--raw` or a raw endpoint path → mode 1 (raw
proxy); `--opaque` or an ALPN fleet does not know → mode 2
(relay-aware proxy); otherwise mode 3 (initiator, served by
`fleet id postcard` / `fleet id stream`, not this verb).

- `<target>` — mode 1: TCP/Unix endpoint; mode 2: peer NodeId
  (64 hex).
- `--alpn <alpn>` — target protocol (required for mode 2; fleet
  knows `/iroh-meta/3`).
- `--relay <url>` — HTTPS relay URL for the mode-2 dial;
  `HTTP(S)_PROXY` env vars configure the CONNECT proxy.
- `--expect-node-id <hex>` — pin the peer NodeId for the mode-2
  dial; a mismatch fails closed (exit 72), never downgrades.

Mode 1 splices this process's stdin/stdout to the target, byte for
byte; fleet parses nothing. Mode 2 dials the pinned peer through the
relay and the standard HTTP CONNECT proxy when configured, then
splices opaque QUIC stream bytes.

Exit codes: 0 ok · 2 usage · 69 mode unavailable · 71 relay
unreachable · 72 identity mismatch · 73 target unreachable.
