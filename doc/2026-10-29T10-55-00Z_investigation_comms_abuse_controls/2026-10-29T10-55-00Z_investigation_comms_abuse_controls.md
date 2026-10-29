# Comms abuse controls for ID — blocks, rate limits and per-counterparty budgets

**Date:** 2026-10-29 · **Issue:** #2032 (LIST 8) · **Synthesis consumer:** #2033
**Scope:** ID comms must carry many small packets (keypresses, live Google-Docs-style
interaction) without spam or DoS. What ships in the box: IP block, one-button common
block lists, rate limiting, `Retry-After` semantics, permanent blocks by public IP, by
ID/public key, or by topic/tag, and configurable budgets (max MB per window across the
whole app, per IP, per ID) plus message/MB caps for a **specific counterparty**, not just
any counterparty.
**Method:** read the current `pkgs/id` tree on `main` (`24901e7b`) — the nine ALPNs
`id serve` registers, the frame codec, the world session and collab paths, the existing
limiters, the config surface, and the prior design records that already flag this gap.
Deliverable requested by the issue: a control inventory with defaults proposed, and "we
don't need everything — start along this path".

---

## TL;DR verdict

**ID has no abuse-control layer, but it is closer than it looks: the parts exist as
unrelated per-feature limiters and as two designed-but-unwired gates.** There is one
allow-list (`.iroh-allowed`, `--allow-node`), five per-feature limiters, two
per-counterparty caps — and **no block list, no byte accounting, no IP-keyed decision,
and no `Retry-After`**. The `429` status does exist on two web paths; neither carries a
retry delay.

| Drew's ask | Today | Verdict |
|---|---|---|
| IP block | **nothing.** `IpAddr` appears only for bind addresses and test fixtures; no code path reads a peer's IP to decide anything | **absent** |
| One-button common block lists | no block list of any kind, no mute/ignore, no UI affordance in `web/src` | **absent** |
| Rate limiting | five limiters, each for one feature (below); none keyed on a counterparty, none configurable | **partial** |
| `Retry-After` semantics | `429` is returned on two web paths, but no `Retry-After` header and no machine-readable delay anywhere; `Reply::Retry` carries free text only | **absent** |
| Permanent block by public IP | nothing | **absent** |
| Permanent block by ID / public key | nothing — and "ID" is four different identifiers (below), so this needs a decision before it needs code | **absent** |
| Permanent block by topic / tag | nothing — and "topic" is three different things in this tree | **absent** |
| Budget: MB per window, app-wide / per IP / per ID | nothing. `WorldMetrics::add_frame_bytes` *records* frame bytes; nothing enforces a ceiling | **absent** |
| Caps for a **specific** counterparty | two exist, both narrow: `WaitingLimits::per_author = 256` held events, `MAX_PENDING_FRIEND_REQUESTS = 20` per sender | **partial** |
| Frame/message size cap | **present**: `MAX_FRAME_BYTES = 1 MiB`, enforced on read and write | **present** |
| Peer allow-list | **present**: `AccessPolicy`, `.iroh-allowed`, `--allow-node` / `--open-writes` | **present** |

**Bottom line:** the shipping path is not a WAF-shaped feature. ID's comms are QUIC
(Iroh) over nine ALPNs, so there are no HTTP headers to hang `Retry-After` on, no socket
per peer for `iptables`, and a relayed peer's IP is not visible at all. The cheap,
correct start is a **per-peer counter + token bucket + a block list keyed on the endpoint
key**, enforced at the frame boundary and at the two gates that are already designed and
unwired (`blob_gate`, `PullGate`), with the retry value carried inside ID's own frames.
IP-level blocking belongs at the listener and the relay. Byte budgets wait for counters.

---

## 1. What comms traffic actually is in ID

`id serve` registers **nine ALPNs** on one Iroh endpoint
([`pkgs/id/src/commands/serve.rs:955-990`](../../../pkgs/id/src/commands/serve.rs)):
metadata RPC (`/iroh-meta/2`), blobs, gossip, docs, event-log sync (`/id-sync/1`), world
sessions (`/id-world/1`), envelopes (`/id-envelope/1`), artifact push/pull, and the SSH
tunnel. Abuse controls have to be placed against these, not against "HTTP".

Four channels carry the traffic the issue cares about:

1. **Metadata RPC** — postcard-encoded `MetaRequest`/`MetaResponse`, capped at
   `MAX_REQUEST_BYTES = 1 MiB` ([`protocol.rs:238`](../../../pkgs/id/src/protocol.rs)).
2. **Docs gossip** — CRDT document sync, many small updates.
3. **Collaboration WebSocket** — ProseMirror collab, MessagePack `[1, version, steps,
   clientID]` ([`web/collab.rs:6-20`](../../../pkgs/id/src/web/collab.rs)). **This is the
   live-document path and it has neither a size cap nor a rate limit**: the upgrade at
   `web/collab.rs:632` sets no `max_message_size`, and nothing counts `STEPS`/`CURSOR`.
4. **World sessions** — one JSON object per frame over a shared length-prefixed codec,
   `MAX_FRAME_BYTES = 1024 * 1024` ([`frames.rs:19-20`](../../../pkgs/id/src/frames.rs)),
   with the browser transport set to the same `max_message_size`
   ([`web/world_ws.rs:279`](../../../pkgs/id/src/web/world_ws.rs)). Keypresses and game
   input are `ClientFrame::Input`; chat is bounded at 2048 bytes
   ([`world/mod.rs:59-101`](../../../pkgs/id/src/world/mod.rs)).

Two properties drive every decision below:

- **Small packets are the normal case.** A typing session is 20-60 frames/s per
  participant. A "N per minute" limiter breaks live editing; the shape must be a
  burst-tolerant token bucket.
- **The receiver pays.** In a P2P session the victim's bandwidth and CPU are consumed by
  inbound traffic; there is no middlebox to shed load. Controls must run **before** the
  work — before decode, before commit, before render.

---

## 2. Control inventory: what exists today

All of it, with defaults and evidence. Note how little of it is reachable by an operator.

### 2.1 Size and structural caps

| Control | Value | Evidence |
|---|---|---|
| `MAX_FRAME_BYTES` (shared codec, both directions) | **1 MiB** | `frames.rs:20`, enforced `:38-41`, `:88-89`, `:124`, `:158-159`; oversized read marks the stream unusable `:110` |
| `MAX_REQUEST_BYTES` (meta RPC) | 1 MiB | `protocol.rs:238` |
| `MAX_WORLD_MODULE_BYTES` / chunk / concurrent uploads | 16 MiB / 6 KiB / 2 | `world_session/types.rs:17,20,32` |
| `MAX_SESSIONS_PER_CONNECTION` (world) | 16 | `world_net.rs:48`, semaphore `:77` |
| `HubLimits::max_sessions` / `max_open_worlds` | 1024 / 256 | `world_hub.rs:84-104` |
| SSH connections / input queue / backlog / packet | 64 / 256 / 8 MiB / 32 KiB | `world_ssh.rs:63-71,158` |
| HTTP body / upload | 10 MiB / 11 MiB | `web/routes.rs:53,56` |
| Sync session deadline / ingest batch | 60 s / 256 | `log_sync.rs:59,66` |
| Dial timeout / envelope delivery timeout | 45 s / 30 s | `connectivity.rs:54`, `envelope_net.rs:31` |

### 2.2 Rate limiters

| # | Where | What it bounds | Default | Evidence |
|---|---|---|---|---|
| 1 | World chat | Chat lines per participant, rolling | **50 / 10 s** | `world/mod.rs:53-55`, check `world/core.rs:955-975` |
| 2 | Web save | Saves of one file name | **1 / 5 s** | `web/mod.rs:86-132`, applied `web/routes.rs:1202-1214` |
| 3 | Directory auth | Code resend per address | **1 / 60 s** | `directory_auth.rs:25`, `:154-166` |
| 4 | Directory auth | Signed-request nonce table | **4096**, 5 min window | `directory_auth.rs:28-29`, `:244-260` |
| 5 | World hub | Boot quiet window for signed requests | default `0`, CLI **600 s** | `world_hub.rs:100`, `cli.rs:412` |

**What is not limited at all:** world `input` frames (size cap only), collab `STEPS` and
`CURSOR`, `/ws/tags`, metadata RPC, blob gets, and `POST /api/identity/register`
(`web/routes.rs:2863-2885`, tracked as #1766). `SaveRateLimiter` also never evicts its
map (#1713).

### 2.3 Per-counterparty caps (count-based, never byte-based)

- `WaitingLimits::per_author = 256` held events, `max_age_ms = 7 days`
  ([`waiting.rs:33,37,64`](../../../pkgs/id/src/waiting.rs)) — the only per-author
  inbound cap.
- `MAX_PENDING_FRIEND_REQUESTS = 20` per sender per server
  ([`directory.rs:306`](../../../pkgs/id/src/directory.rs), enforced `:1250-1252`).
- World chat's 50/10 s is per *participant capability*, which is per session, not per
  person.

### 2.4 Allow-list (the inverse of a block list)

`AccessPolicy` is a flat writer allow-list for mutating operations — `put`, `delete`,
`rename`, `copy`, tag writes, blob pushes ([`access.rs:57-105`](../../../pkgs/id/src/access.rs)).
Backed by `.iroh-allowed` (`:38`, parser `:127`, loader `:139`), driven by
`--allow-node` / `--open-writes` ([`cli.rs:464,470`](../../../pkgs/id/src/cli.rs)).
Its own module doc calls it "deliberately a flat allowlist, not a capability system"
(`access.rs:1-24`). This is the closest thing to a block list in the tree, and it is
about *writing*, not about *serving to* or *receiving from*.

### 2.5 Two gates designed and not wired

- **`BlobPolicy` / `blob_gate::decide`** — fetch requires world membership; observe off by
  default ([`blob_gate.rs:53-95`](../../../pkgs/id/src/blob_gate.rs)). Its own doc says the
  reference index "has no producer yet, so `blob_events` is not wired into `id serve`"
  (`:24-25`).
- **`PullPolicy` { Open (default), Members } / `PullGate`** —
  ([`pull_policy.rs:30-80`](../../../pkgs/id/src/pull_policy.rs)); `id serve` does not read
  it and no session builds a gate from it (`:17-19`).

These two are the natural first enforcement points for a block list and for budgets: they
already exist, they already encode "who may get this from me", and they are already
documented as the place where a policy belongs.

### 2.6 Configuration surface — and the trap

- Live: CLI flags (clap, `cli.rs`), sparse env vars (`ID_WEB_TOKEN`,
  `ID_WORLD_ADMIN_TOKEN`, `ID_ROC_BIN` …), embedded `defaults.conf` (`discovery.rs:72`),
  per-world `WorldExecutionProfile` (`world_limits.rs:31-52`).
- **`ServerConfig` / `id-server-config.toml` is parsed but dead.** `ServerConfig::parse`
  and `::load` have no caller, and `id serve` has no `--config` flag
  ([`server_config/mod.rs:1-35,158-176`](../../../pkgs/id/src/server_config/mod.rs)). The
  schema (`[create]`, `[[users]]`, `[[groups]]`, `[policy]`) is documented inline, and the
  parser refuses unknown fields — so a proposed `[abuse]` table is not just a parse change,
  it is **wiring the config file up for the first time**. Any design that says "put it in
  the TOML config" must budget for that.

### 2.7 Refusals

`Refusal::RateLimited(String)` ([`directory.rs:109`](../../../pkgs/id/src/directory.rs))
maps to **HTTP 429** on the world WebSocket (`web/world_ws.rs:209`) and on save
(`web/routes.rs:1212`). So the status exists. What does not exist: any `Retry-After`
header (zero matches for `retry-after`/`retry_after` in `src/` and `web/src/`), and any
delay value inside ID's own frames — world chat refuses with the prose "chat rate
exceeded; try again later" (`world/core.rs:955-975`), and `Reply::Retry` in the envelope
protocol carries a free-text message only ([`envelope_net.rs:34-52`](../../../pkgs/id/src/envelope_net.rs)).

---

## 3. "Block by ID" and "block by topic" are not one decision each

The issue lists three block key kinds. In this tree each of them is ambiguous, and the
design cannot proceed without choosing:

| Key kind | Candidates in the code | What blocking it would mean |
|---|---|---|
| **ID / public key** | iroh `EndpointId` (transport node ID, `access.rs:97`); `AuthorKey = [u8;32]` (event graph, `waiting.rs:31`); account `id` (directory, `directory.rs:79-92`); web `client_id` (browser, `web/identity.rs:89-120`); world `participant_id: u64` (capability, *not* identity, `world_session/mod.rs:14-16`) | Blocking the `EndpointId` stops transport; blocking the `AuthorKey` stops content and rendering. **There is no join table between them**, so "block this person" is two rows, and a key rotation resets the transport row but not the author row |
| **Public IP** | no peer-IP decision path exists; `TransportAddr::Ip` appears only in tests and bind addresses | Only meaningful for direct connections; relayed peers have no visible IP |
| **Topic / tag** | gossip `TopicId` (`discovery.rs:45-60`); metadata `tags` (`tags.rs`, `tag_claims.rs`); a world's `world_name`/`world_id` | Three different objects with three different enforcement points. "Block by tag" most likely means the metadata tag (annotation on files); "block by topic" most likely the gossip channel |

Recommendation for #2033: define the block row as `{kind, value}` with `kind` naming
exactly which of these, rather than inventing a unified "ID".

---

## 4. The control set, with proposed defaults

Four layers, in build order. Each names where it is enforced, because in a P2P system
"where" is the whole design.

### L1 — Rate at the frame boundary (per peer, per connection)

A token bucket per `(local node, remote endpoint key)`, applied at frame read before
decode, plus a per-connection bucket for cheap fairness:

| Knob | Proposed default | Why |
|---|---|---|
| `frames_per_sec` (authorized, interactive) | **60**, burst **180** | 10 keystrokes/s + cursor at pointer rate fits; 3 s of burst absorbs a paste or scroll storm |
| `bytes_per_sec` (authorized) | **512 KiB/s**, burst **4 MiB** | A 4-frame burst covers one paste; sustained media belongs in blobs |
| `frames_per_sec` (not yet authorized) | **10**, burst **30** | No legitimate pre-capability stream needs more |
| `connections_per_peer` | **4** concurrent | One tab plus a retry; stops fan-out |
| Collab `STEPS`/`CURSOR` | **120 frames/s**, burst **360**, and a **1 MiB message cap** | Currently uncapped in both directions (`web/collab.rs:632`) — this is the largest single hole |
| Refusal shape | frame `{code, reason, retry_after_ms}`; HTTP routes get `429` + `Retry-After` | QUIC streams have no headers, so the value must live in the protocol |

`retry_after_ms = max(1000, ms_until_next_token)`, capped at **60 s**, never zero.

### L2 — Operator block list (by endpoint key, by author key, by IP, by tag/topic)

One persisted list, several key kinds, one enforcement point each:

| Key kind | Semantics | Enforced at |
|---|---|---|
| **Endpoint key** (`EndpointId`) | refuse new sessions, drop gossip subscriptions, stop serving blobs | `connectivity.rs` accept path, blobs router, `blob_gate` |
| **Author key** (`AuthorKey`) | filter their objects out of views and refusals | view/projection filter, `waiting.rs` queue |
| **IP** | refuse direct connections | Iroh endpoint listener / host firewall; **not** enforceable when relayed |
| **Tag / topic** | refuse to pull, serve or subscribe | `PullGate` (wire it up) and subscription filter |

Storage: a redb table in the existing event store (or a small dedicated file beside
`.identity.redb`), `{kind, value, added_at, reason, expires_at}`; `expires_at = null` is
the permanent block the issue asks for.

### L3 — Budgets (app-wide, per IP, per ID) and per-counterparty overrides

| Budget | Proposed default | Window |
|---|---|---|
| App-wide inbound | **32 MiB/s**, **20 GiB/hour** | rolling |
| App-wide outbound (serving) | **64 MiB/s**, **40 GiB/hour** | rolling |
| Per peer (endpoint key) | **100 MiB/hour**, burst **8 MiB/min** | rolling |
| Per IP (direct only) | **250 MiB/hour** | rolling |
| **Specific counterparty** | `{peer, bytes_per_window, messages_per_window}` override rows; default none | operator-set |

The per-counterparty override is the issue's key distinction and is genuinely missing
today: the two existing per-sender caps are count-based and hard-coded (`per_author = 256`,
friend requests = 20). Model it as a row consulted before the default bucket, so one peer
can be capped at 1 MiB/day without touching the global budget.

### L4 — IP blocking and "common block lists"

- **In the box:** a listener-level IP deny set (direct connections only) and the
  endpoint/author lists from L2, which are what actually work between ID nodes.
- **Not shipped:** a list of "bad IPs". IP lists rot in hours, relayed peers evade them,
  and a stale list is worse than none. The honest version of "one-button common block
  lists" is a **one-button import** of a list the operator already trusts (CIDRs or peer
  keys, with a preview) plus a one-click "block this peer" action.
- **Where WAFs still earn their place:** the `id serve` web surface is HTTP, so
  `429`/`Retry-After`, IP allow/deny and request budgets there are ordinary web work and
  can be delegated to a reverse proxy. The P2P surface cannot.

---

## 5. How this composes with "servers accept all signed objects"

The trust model (#2030) says a server stores and relays every correctly signed object and
only *views* are scoped by who may write. Abuse controls must not quietly become an
acceptance filter, or the graph diverges between nodes. Therefore:

- **Blocks never reject a signed object.** A block changes *serving, subscribing and
  rendering*: we stop pulling from you, stop serving you, stop showing your objects. What
  is already in the graph stays there, so sync and auditability survive.
- **Rate limits and budgets do reject traffic**, and that is fine: they reject *work*, not
  *content*. A rate-limited peer is asked to come back, not told its object is invalid.
- **The exception is the listener and the relay**, where IP refusal is a transport decision
  that happens before ID semantics exist.

The rule for #2033: *a refusal carries a reason class — `rate_limited`,
`budget_exhausted`, `blocked_by_operator`, `unauthorized` — and only the last is ever
about the object itself.*

---

## 6. Operator UX — the "quick button"

1. Each peer row gets **Block** (endpoint + author rows, drop live sessions) and **Mute**
   (view filter only, no refusal).
2. A **Blocks** page: `kind`, value, reason, added, expiry, **Unblock**.
3. **Import list** takes a file/URL of peer keys or CIDRs and previews what will be added.
4. Every refusal a human can see says which control fired and when it lifts. An operator
   cannot tune budgets they cannot observe.

---

## 7. What to build first — "we don't need everything"

Ordered by value ÷ cost, and each step ships alone:

- **P0 — counters and the refusal shape.** Per-peer frame/byte counters, the L1 bucket,
  and one structured refusal type carrying `reason` + `retry_after_ms`. Nothing else can be
  tuned without counters, and the refusal shape is a wire change that is cheapest now.
- **P0 — cap the collab path.** `web/collab.rs` has neither a message cap nor a rate on
  `STEPS`/`CURSOR`; a live-document session is currently the cheapest way to flood a peer.
- **P0 — block a peer.** Endpoint-key and author-key rows, the web UI button, the blocks
  page. This is the control an operator reaches for during an incident.
- **P1 — budgets** (L3) once counters exist; app-wide and per-peer first, the
  per-counterparty override second.
- **P1 — wire the two dead gates** (`blob_gate`, `PullGate`) and `ServerConfig` — the
  natural homes for policy that currently cannot be reached from `id serve`.
- **P1 — HTTP hygiene on `id serve`:** `Retry-After` on the two existing `429` sites, a
  limit on `POST /api/identity/register` (#1766).
- **P2 — IP deny set at the listener**, tag/topic-scoped blocks, block-list import.
- **Not now:** shipped "common block lists", federation of block lists, reputation
  scoring. None is needed to survive a spammer, and all presume counters we do not have.

---

## 8. Prior art in this repository (already says the same thing)

The gap is not newly discovered; the design records flagged it and nothing was built:

- `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/…design_review.md:151` —
  "anyone can publish refused events to grow it. Needs per-author caps and rate limits."
- `…/research_findings.md:22` — the pending-item queue as "a DoS vector".
- `…/design_connectivity.md:116-125` — "An open relay is free bandwidth for anyone";
  shared token / endpoint allowlist / rate limits; n0 relays "may block IPs".
- `doc/2026-10-10T02-39-03Z_design_directory_sync/…:348` — "Should a pending request be
  capped per sender, to limit spam?" (answered later by the 20-request cap).
- `doc/2026-03-22T00-00-00Z_feature_peer-discovery/…:179` — "No rate limiting: beyond
  iroh-gossip, there is no application-level rate limiting."
- `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/…:428` — LIST 8 abuse
  controls "fits" the hosted edition (Durable Object as serialization point, per-IP/ID/
  topic budgets as rows).

Related open issues that this investigation does not duplicate: #1067 (fair scheduling and
per-account quotas, worlds/storage), #1546 (`min_tick_ms` never enforced), #560 (DoS test
suite for the world sandbox), #2121 (unbounded `tuple` decode recursion), #1899 (SMTP
reader has no deadline), #1766, #1713.

---

## 9. Open questions for #2033 (the design)

1. **Which identifier is "the counterparty"?** Endpoint key, author key, account, or a
   tuple of them (§3). Key rotation resets a transport bucket; it does not reset an author
   bucket. The answer decides whether a spammer can reset by reconnecting.
2. **Relayed peers and IP.** Should the relay attach a signed statement of the peer's
   observed address so IP blocks work through relays? That is a privacy change, not just a
   control, and needs its own decision.
3. **Where the block list lives when one node serves many groups.** Per-node, per-group, or
   both? Per-group fits group Admin authority but multiplies enforcement points.
4. **Budget units.** Bytes are right for bandwidth; the DoS cost of tiny frames is CPU.
   Is the L1 frame rate enough, or do budgets need a second unit (frames or decoded events)?
5. **Refuse-and-keep or refuse-and-close on QUIC streams?** Keeping is friendlier and
   cheaper to implement; closing frees state.
6. **Does wiring `ServerConfig` come first?** If every control is a CLI flag, operators get
   no persistent policy; if the TOML file is wired, that is its own task before any
   `[abuse]` table can be written.

---

## Sources

**In-repo, verified on `main` @ `24901e7b`:**

- `pkgs/id/src/commands/serve.rs:955-990` — the nine ALPNs and router build.
- `pkgs/id/src/frames.rs:19-20, 38-41, 88-89, 110, 124, 158-159` — `MAX_FRAME_BYTES` and
  its enforcement.
- `pkgs/id/src/protocol.rs:238` — `MAX_REQUEST_BYTES`.
- `pkgs/id/src/web/collab.rs:6-20, 632` — collab wire format, upgrade with no size cap.
- `pkgs/id/src/web/world_ws.rs:279, 209` — world WS `max_message_size`, 429 mapping.
- `pkgs/id/src/world/mod.rs:53-55, 59-101`; `world/core.rs:955-975` — chat rate and limits.
- `pkgs/id/src/world_net.rs:48, 77`; `world_hub.rs:84-104`; `world_ssh.rs:63-71,158` —
  connection/session caps.
- `pkgs/id/src/web/mod.rs:86-132`; `web/routes.rs:1202-1214` — `SaveRateLimiter`.
- `pkgs/id/src/directory_auth.rs:24-29, 154-166, 244-260` — resend, TTL, nonce table.
- `pkgs/id/src/access.rs:1-24, 57-105, 127-139` — `AccessPolicy`, `.iroh-allowed`.
- `pkgs/id/src/blob_gate.rs:24-25, 53-95`; `pull_policy.rs:17-19, 30-80` — designed,
  unwired gates.
- `pkgs/id/src/waiting.rs:33,37,64`; `directory.rs:306,1250-1252` — the two per-sender caps.
- `pkgs/id/src/server_config/mod.rs:1-35, 158-176` — schema and the fact it has no caller.
- `pkgs/id/src/envelope_net.rs:34-52`; `directory.rs:109` — refusal types.
- `doc/` records listed in §8.
- GitHub #2032, #2033, #2056, #2030, #2034, #2035, #2036; #1766, #1713, #1067, #1546,
  #560, #2121, #1899.

**Standards for the refusal shape:** RFC 9110 §10.2.3 (`Retry-After`), RFC 6585 §4
(`429 Too Many Requests`) — adopted as the HTTP spelling; the frame-side equivalent is
proposed here because QUIC streams carry no headers.
