# Comms abuse controls for ID — blocks, rate limits and per-counterparty budgets

**Date:** 2026-10-29 · **Issue:** #2032 (LIST 8) · **Synthesis consumer:** #2033
**Scope:** ID comms must carry many small packets (keypresses, live Google-Docs-style
interaction) without spam or DoS. What ships in the box: IP block, one-button common
block lists, rate limiting, `Retry-After` semantics, permanent blocks by public IP, by
ID/public key, or by topic/tag, and configurable budgets (max MB per window across the
whole app, per IP, per ID) plus message/MB caps for a **specific counterparty**, not just
any counterparty.
**Method:** read the current `pkgs/id` tree on `main` (`24901e7b`) — transport, frame
codec, world chat, web save/identity paths, server config — plus the open issues that
already record known gaps (#1766, #1713). Deliverable requested by the issue: a control
inventory with defaults proposed, and "we don't need everything — start along this path".

---

## TL;DR verdict

**ID has no abuse-control layer. It has five unrelated local limiters, each written for a
different feature, none of them reachable from configuration, none of them returning
`Retry-After`, and none of them able to name a counterparty.** Every mechanism Drew asks
for except raw frame size is **absent**:

| Drew's ask                                  | Today                                                                                                                                                            | Verdict        |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------- |
| IP block                                    | nothing anywhere; `grep -i 'block\|ban\|deny'` over `peers`, `connectivity`, `petname` returns nothing                                                            | **absent**     |
| One-button common block lists               | nothing; there is no operator surface that can even express "block this peer"                                                                                    | **absent**     |
| Rate limiting                               | five per-feature limiters (below), none configurable, none per-peer                                                                                               | **partial**    |
| `Retry-After` semantics                     | no `429`, no `Retry-After`, no `retry_after` field on any refusal; the closest thing is a human sentence in an error string                                        | **absent**     |
| Permanent block by public IP                | nothing                                                                                                                                                          | **absent**     |
| Permanent block by ID / public key          | nothing — the identity exists (`Key`), the policy does not                                                                                                        | **absent**     |
| Permanent block by topic / tag              | nothing                                                                                                                                                          | **absent**     |
| Budget: MB per window, app-wide / per IP / per ID | nothing. No byte accounting per peer exists, so no budget can be enforced even in principle                                                                  | **absent**     |
| Caps for a **specific** counterparty        | the world chat limiter is per *participant in a world* (`CHAT_RATE_BURST`), which is the only per-counterparty counter in the tree — and it is 50 lines / 10 s, not bytes | **partial**    |
| Frame/message size cap                      | **present**: `MAX_FRAME_BYTES = 1 MiB`, enforced on both read and write                                                                                           | **present**    |

**Bottom line:** the shipping path is *not* a WAF-shaped feature. ID's transport is QUIC
(Iroh), so there are no HTTP headers to hang `Retry-After` on, no socket-per-peer to
`iptables`, and an IP is often not even visible (relays and hole punching). The cheap,
correct start is a **per-peer counter + token bucket + a local block list keyed by public
key**, all enforced at the *serving and rendering* boundary, with `Retry-After`-equivalent
values carried inside ID's own refusal frames. IP-level blocking belongs at the listener
and at the relay/WAF layer, not in the app. Everything else — tag-scoped blocks, byte
budgets, shared block lists — waits until counters exist to drive them.

---

## 1. What comms traffic actually is in ID

ID is an Iroh node: QUIC transport, hole punching, optional relays, on top of which it
runs content-addressed blobs and CRDT documents
([`pkgs/id/ARCHITECTURE.md:5-33`](../../../pkgs/id/ARCHITECTURE.md)). Abuse controls have
to be placed against *these* four channels, not against "HTTP":

1. **Iroh RPC** — `MetaRequest`/`MetaResponse`, postcard-encoded, for file and tag
   operations (`ARCHITECTURE.md:44-56`). Request/response, bounded by the frame cap.
2. **Docs gossip** — CRDT document sync, many small updates. This is where a
   Google-Docs-style session spends its packets.
3. **WebSocket collaboration** — ProseMirror collab over binary MessagePack
   (`ARCHITECTURE.md:161-165`, protocol in `pkgs/id/web/README.md`). Keypresses and
   cursors are here; a single browser session emits several frames per second while
   someone types, plus cursor frames on every pointer move.
4. **World sessions** — framed binary protocol shared by the native, SSH and web world
   surfaces: `MAX_FRAME_BYTES = 1024 * 1024`, with the WebSocket transport configured to
   the same `max_message_size` ([`pkgs/id/src/frames.rs:19-20`](../../../pkgs/id/src/frames.rs)).
   A world carries chat, game input and program output
   ([`world/mod.rs:59-105`](../../../pkgs/id/src/world/mod.rs)).

Two properties of this shape drive every design decision below:

- **Small packets are the normal case.** A typing session at 10 keystrokes/s with cursor
  frames is 20-60 frames/s per participant. Any limiter with a naive "N per minute" shape
  breaks live editing; the limiter has to be a burst-tolerant token bucket, not a counter.
- **The receiver pays.** In a P2P session the victim's bandwidth and CPU are consumed by
  the attacker's *inbound* stream; there is no middlebox to shed load. Controls must
  therefore run **before** the work (before decode, before commit, before render), which
  means at the frame boundary.

---

## 2. Control inventory: what exists today

Every entry below is a real limiter in the tree, with its current default. All five were
written for a specific feature; none is exposed in configuration; none is per-peer.

| # | Where | What it bounds | Default | Evidence |
|---|-------|----------------|---------|----------|
| 1 | Frame codec | Any single frame, both directions | **1 MiB** | `frames.rs:20`, enforced at `:38-41`, `:88-89`, `:124`, `:158-159`; oversized read marks the stream unusable (`:110`) |
| 2 | World chat | Chat lines per participant, rolling window | **50 lines / 10 s** | `world/mod.rs:53-55`; check in `world/core.rs:955-975`; refusal text "chat rate exceeded; try again later" |
| 3 | World limits | Participants, chat bytes, input bytes, retained events, snapshot, tick floor | **256 / 2 KiB / 16 KiB / 1024 / 512 KiB / 1000 ms** | `world/mod.rs:59-105` (`impl Default`) |
| 4 | Web save | Saves of one file name | **1 per 5 s** | `web/mod.rs:86-132` (`SaveRateLimiter`, `DEFAULT_SAVE_COOLDOWN`) |
| 5 | Directory auth | Code resend per address; signed-request replay | **1/min**; **5 min window, 4096 nonces**; code TTL **15 min** | `directory_auth.rs:24-29`, `:154-166`, `:236-262` |

What these five do **not** give us:

- **No per-peer identity in any counter.** #2 is keyed by world participant capability,
  #4 by file name, #5 by e-mail address or request nonce. Nothing keys on the remote
  endpoint public key, which is the only identity an abuser cannot cheaply forge in ID.
- **No byte accounting at all.** There is no counter of bytes or frames per peer, per
  connection or app-wide, so "max MB per window" cannot be evaluated today. This is the
  single hardest prerequisite in the issue.
- **No `Retry-After`.** #2 and #5 refuse with prose. There is no structured retry value
  and no `429` on any web route. A client cannot back off correctly, so it retries.
- **No block list of any kind**, and no config key that could hold one: `ServerConfig`
  is `users`, `groups`, `create`, `policy` (freshness window, pull policy) and nothing
  else ([`server_config/mod.rs:58-70`](../../../pkgs/id/src/server_config/mod.rs)),
  parsed from `id-server-config.toml` (`server_config/parse.rs:750`).
- **Two known unthrottled paths already filed:** `POST /api/identity/register` has no
  rate limit and no identity eviction (#1766), and `SaveRateLimiter`'s map never evicts
  (#1713). Both are symptoms of the same gap: limiters are written per feature, so the
  next feature forgets one.

---

## 3. The control set, with proposed defaults

Proposed as four layers, in the order they should be built. Each names where it is
enforced, because in a P2P system "where" is the whole design.

### L1 — Size and rate at the frame boundary (per connection, per peer)

A token bucket per `(local node, remote endpoint key)` pair, applied at frame read before
decode, and a second one per connection for cheap fairness:

| Knob | Proposed default | Why |
|------|------------------|-----|
| `frames_per_sec` (live/interactive) | **60**, burst **180** | 10 keystrokes/s + cursor at pointer-move rate fits comfortably; 3 s of burst absorbs a paste or a scroll storm |
| `bytes_per_sec` (live) | **512 KiB/s**, burst **4 MiB** | A 1 MiB frame burst of 4 fits inside one paste; sustained media goes through blobs, not frames |
| `frames_per_sec` (unauthenticated / not-yet-authorized peer) | **10**, burst **30** | Before a capability or group membership is proven, no legitimate stream needs more |
| `connections_per_peer` | **4** concurrent | One browser tab plus a retry; stops connection fan-out |
| `connections_per_ip` (direct, listener only) | **20 / min**, **100** concurrent | Only meaningful when the peer is not relayed; see L4 |
| Refusal shape | `429`-equivalent **frame** `{code:"rate_limited", retry_after_ms}`; HTTP routes get real `429` + `Retry-After` | QUIC has no headers, so the value must live in the protocol |

`Retry-After` computation: `max(1s, ms_until_next_token)`, capped at **60 s**, and never
zero — a zero tells the client to hammer.

### L2 — Operator block list (by public key, by IP, by topic/tag)

One persisted list, three key kinds, one enforcement point each:

| Key kind | Semantics | Enforced at | Notes |
|----------|-----------|-------------|-------|
| **Public key** (`Key`, the endpoint ID) | *Permanent*: refuse to open sessions, serve blobs, accept gossip or render their objects | `connectivity.rs` accept path + every frame entry + view filter | The reliable one. Rotation is possible but costs the abuser their reputation and petname |
| **Public IP** | *Transport-level*: refuse direct connections from this address | Listener only (Iroh endpoint / host firewall); **not** enforceable when relayed | Must be documented as best-effort: a relayed peer has no visible IP |
| **Topic / tag** | *Scoped*: refuse to pull, serve or subscribe to that tag/topic | Pull policy + subscription filter | Composes with the existing `PullPolicy` in `ServerConfig` |

Storage: a `blocks` table in the existing per-node store (the same place identities and
`SaveRateLimiter` state already live), with `{kind, value, added_at, reason, expires_at}`.
`expires_at = null` means permanent, which is what the issue asks for; a temporary block is
the same row with a time.

### L3 — Budgets (app-wide, per IP, per ID) and per-counterparty caps

| Budget | Proposed default | Window |
|--------|------------------|--------|
| App-wide inbound | **32 MiB/s**, **20 GiB/hour** | rolling |
| App-wide outbound (serving) | **64 MiB/s**, **40 GiB/hour** | rolling |
| Per peer (ID) | **100 MiB/hour**, burst **8 MiB/min** | rolling |
| Per IP (direct only) | **250 MiB/hour** | rolling |
| **Specific counterparty** cap | `{peer, bytes_per_window, messages_per_window}` rows in config; default none | operator-set |

The "specific counterparty" requirement is the important one and it is *not* the same as a
per-peer budget: it lets an operator say "this one peer gets 1 MiB/day" without touching
the global budget. Model it as an override row consulted before the default bucket.

Budget enforcement needs counters that do not exist yet (§2). That is why L3 is last.

### L4 — IP blocking and "common block lists"

- **In the box:** a listener-level IP deny set (direct connections only) and the public-key
  list from L2, which is what actually works between ID nodes.
- **Not in the box, by design:** a shipped list of "bad IPs". IP block lists rot within
  hours, are trivially evaded by relayed peers, and a stale list is worse than none.
  The honest version of Drew's "one-button common block lists" is a **one-button import**
  of a list an operator already trusts (a text file or URL of CIDRs, or a peer block list),
  plus a one-click "block this peer" action in the web UI that writes the L2 row.
- **Where WAFs still earn their place:** the web surface (`id serve`) is HTTP, so
  `429`/`Retry-After`, IP allow/deny and request budgets there are ordinary web work and
  can be delegated to a reverse proxy. The P2P surface cannot.

---

## 4. How this composes with "servers accept all signed objects"

The trust model (#2030) says a server stores and relays every correctly signed object and
only *views* are scoped by who may write. Abuse controls must not quietly become an
acceptance filter, or the graph diverges between nodes. Therefore:

- **Blocks never reject a signed object.** A block changes *serving, subscribing and
  rendering*: we stop pulling from you, stop serving you, stop showing your objects. What
  is already in the graph stays there, so sync and auditability survive.
- **Rate limits and budgets do reject traffic**, and that is fine: they reject *work*, not
  *content*. A rate-limited peer is asked to come back (`retry_after_ms`), not told its
  object is invalid.
- **The one exception is the relay and the listener**, where IP-level refusal is a
  transport decision that happens before ID semantics exist.

The rule to write into #2033: *a refusal carries a reason class — `rate_limited`,
`budget_exhausted`, `blocked_by_operator`, `unauthorized` — and only the last is ever
about the object itself.*

---

## 5. Operator UX — the "quick button"

The issue's "one-button common block lists" is really an operator-flow requirement. Minimum
viable surface, in the web UI where the peer list already renders names and petnames:

1. Each peer row gets **Block** and **Mute** (block = L2 public-key row + drop live
   sessions; mute = view filter only, no refusal).
2. A **Blocks** page lists rows with `kind`, value, reason, added, expiry, and an
   **Unblock** button.
3. **Import list** takes a file/URL of peer keys or CIDRs and previews what will be added
   before committing.
4. Every refusal that a *human* can see should say which control fired and when it lifts —
   an operator cannot tune budgets they cannot observe.

---

## 6. What to build first — "we don't need everything"

Ordered by (value ÷ cost), which is also the order that keeps each step shippable alone:

- **P0 — counters and refusal shape.** Per-peer frame/byte counters, the token bucket of
  L1, and one structured refusal type carrying `reason` + `retry_after_ms`. Nothing else
  can be tuned without the counters, and the refusal shape is a wire change that is
  cheapest before more frames exist.
- **P0 — block a peer by public key.** L2's first key kind, plus the web UI button and
  the blocks page. This is the control an operator actually reaches for during an
  incident.
- **P1 — budgets** (L3) once counters exist; app-wide and per-peer first, the
  per-counterparty override second.
- **P1 — HTTP hygiene on `id serve`:** `429` + `Retry-After` on the register and save
  routes, which also closes #1766 and the intent behind #1713.
- **P2 — IP deny set at the listener**, tag/topic-scoped blocks, block-list import.
- **Not now:** shipped "common block lists", federation of block lists between nodes,
  any machine-learning or reputation scoring. None of it is needed to survive a spammer,
  and all of it presumes counters we do not have.

---

## 7. Open questions for #2033 (the design)

1. **Identity for accounting.** Endpoint public key is the right key, but a peer that
   reconnects with a new key starts with a fresh bucket. Do we key the bucket on the
   *authenticated group membership* where one exists, so an abuser cannot reset by
   rotating a key inside a group? (Cheap to do where a capability is already presented.)
2. **Relayed peers and IP.** Should the relay attach a signed statement of the peer's
   observed address, so IP blocks work through relays? That is a privacy change, not just
   a control, and needs its own decision.
3. **Where the block list lives when the node is a server for many groups.** Per-node,
   per-group, or both? A per-group block list is the natural fit for group Admin
   authority but multiplies the enforcement points.
4. **Budget units.** Bytes are right for bandwidth, but the DoS cost of tiny frames is
   CPU, not bytes. Do budgets need a second unit (frames or decoded events) or is the
   L1 frame rate enough?
5. **`Retry-After` on QUIC streams.** A refusal frame keeps the stream open; is the
   convention "refuse and keep", or "refuse and close with a code"? Keeping is friendlier
   and cheaper to implement; closing frees state.

---

## Sources

**In-repo (verified on `main` @ `24901e7b`):**

- `pkgs/id/ARCHITECTURE.md:5-33, 44-56, 161-165` — Iroh/QUIC transport, RPC protocol,
  WebSocket collaboration protocol.
- `pkgs/id/src/frames.rs:19-20, 38-41, 88-89, 110, 124, 158-159` — `MAX_FRAME_BYTES`,
  read/write enforcement, unusable-stream marking.
- `pkgs/id/src/world/mod.rs:53-55, 59-105` — chat rate constants, `WorldLimits` defaults.
- `pkgs/id/src/world/core.rs:955-975` — per-participant chat limiter and its refusal text.
- `pkgs/id/src/web/mod.rs:86-132` — `SaveRateLimiter`, `DEFAULT_SAVE_COOLDOWN`.
- `pkgs/id/src/directory_auth.rs:24-29, 154-166, 236-262` — code resend, TTL, signed-request
  window and nonce limit, `Refusal::RateLimited`.
- `pkgs/id/src/server_config/mod.rs:58-70` — `ServerConfig`/`PolicySpec`: the whole config
  surface today.
- `pkgs/id/src/server_config/parse.rs:750` — `id-server-config.toml` parse path.
- GitHub #1766 (unthrottled register handler, no identity eviction), #1713
  (`SaveRateLimiter` map never evicts), #2030 (trust/visibility model), #2033 (synthesis).

**Standards referenced for the refusal shape:** RFC 9110 §10.2.3 (`Retry-After`),
RFC 6585 §4 (`429 Too Many Requests`) — adopted as the HTTP-side spelling; the frame-side
equivalent is proposed here because QUIC streams have no headers.
