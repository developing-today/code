# Multi-transport reach — any protocol as a carrier

**Kind:** design record (programme direction)
**Recorded:** 2026-11-23 (direction from Drew, ~18:55 CST)
**Programme epic:** [#13926](https://github.com/developing-today/code/issues/13926)
**Head design:** [#13929 — the transport adapter contract](https://github.com/developing-today/code/issues/13929)
**Direction recorded on:** [#13752 (transport design), comment 7118605020](https://github.com/developing-today/code/issues/13752#issuecomment-7118605020)

## The direction

Long-term goal: the P2P / ID / fleet stack should be reachable over many alternate transports and proxied alternate systems, in the same spirit as Veilid's multi-transport design and the Spritely relay work (federated, stateless, end-to-end encrypted relays; the Noise XK netlayer, reviewed 2026-11-23).

This extends the bootstrap-channels direction recorded on #13752 earlier the same day (one signed fleet-CA-bound ticket format, many carriers — email, QR/tickets, physical media carrying _introductions_). The step recorded here: **carriers graduate from bootstrap-only to full message transport.**

## The inventory (as named by Drew)

Grouped as filed on the epic:

- **Overlay / relay transports:** Spritely-style federated stateless relays (Noise XK netlayer), Tor, Noise generally
- **Federation protocols:** ActivityPub, ATProto
- **Store-and-forward & buses:** email, MQTT
- **Request/response & document protocols:** pure HTTP/REST; Gopher and Gemini as alternatives to HTTP
- **Agent/tool protocols:** MCP, and any other common protocols / transport libraries
- **Chat systems as transports or proxied systems:** IRC relays, Zulip, Slack, Discord bots
- **Terminal access:** telnet in addition to SSH
- **Physical media:** sneakernet bundles; print/fax/carrier-pigeon-class paths are the extreme members of this group

## The level vocabulary

Each adapter declares its level **per direction**:

- **read** — the stack can receive/observe over this path
- **bidirectional** — messages flow both ways over this path
- **write-capable** — the stack can originate actions/writes over this path
- **sync-equivalent** — the path can carry full state synchronisation

Precise definitions, and the rest of the contract (adapter declarations, security per level), live in the head design, #13929.

## The asymmetry requirement

**Asymmetric paths are first-class — a hard design requirement.** Send and receive transports are chosen independently. Drew's words: _"if someone wants to send a message over carrier pigeon and receive the response over fax, I'm down to support it."_ A reply address is part of the message contract; no component may assume the return path equals the forward path.

## Programme structure

1. **Head design (#13929):** the transport adapter contract — what every adapter must declare (addressing; delivery and ordering semantics; identity binding to the fleet CA / ID identity; capability gating per level; metering), the four levels defined precisely, asymmetric routing rules, a security analysis per level, the transport-vs-proxy distinction (a proxied chat system terminates at a platform we do not control; identity is a proxy-held mapping, not a cryptographic property of the path), dependency-ordered build slices S1–S8 with acceptance criteria, and a first-adapter order grounded in what already exists: HTTP via the bridge → email → MCP via the mcpx door → MQTT → sneakernet → IRC (first proxy) → federation → overlays/Tor → Gopher/Gemini/telnet opportunistically.
2. **Adapter designs/builds** filed as children of the epic as the contract lands, each declaring its level(s) up front and proving them live before the level is recorded as held.

## First instances that already exist

- Email + HTTP as bootstrap carriers — #13752
- The MCP door, live — #13657
- The bridge, an HTTP note transport — #13895
- Sneakernet bundles (the physical-media member; also D4 of the ID-graph programme) — #13889, epic #13880

## Neighbours

- #13779 — P2P synthesis (the data plane these transports carry)
- #5674 — ID wire format & transport adapters (ID-internal scope; plugs into this contract)
- #13752 — fleet transport design (pilot stays broker-ticket-first; this programme is its recorded expansion path)
