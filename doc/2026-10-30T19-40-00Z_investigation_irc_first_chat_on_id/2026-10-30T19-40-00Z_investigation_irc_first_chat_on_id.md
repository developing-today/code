# IRC-first chat on ID — channels, graph-linked objects, presence

**Date:** 2026-10-30 · **Issue:** #2049 (LIST 14a) · **Synthesis consumer:** #2051
**Scope:** Drew: "I mostly just want IRC", with room to grow toward Discord/Slack/Teams.
What an IRC-shaped client needs from ID: channels/topics, identity = the Iroh ID
(no registration), messages that link to games, pictures, and other objects stored in
people's graphs, and presence/roster semantics over P2P links.
Out of scope: object retrieval internals (#2050 owns them) and the chat v1 synthesis
itself (#2051 consumes this).
**Method:** read the current `pkgs/id` tree on `origin/main` (`c90fcbc7`) — the world
chat path, the envelope transport, the group/invite/membership primitives, the serving
router's ALPNs, and the adjacent `pkgs/chat` demo — plus the bridge-semantics
investigation (#2058, which left chat an explicit open question for #2051 with input
from this issue).

---

## TL;DR verdict

**ID already owns roughly two-thirds of an IRC core wearing a game-world costume, and
the missing third is well-bounded.** World chat gives us addressed rooms, a bounded
sanitized message event, capability-scoped send/receive, and catch-up history; the
envelope transport gives us signed 1:1 delivery with explicit receipt semantics; and
identity is already registration-free (Iroh node ID, pasted tickets, private petnames).
What does not exist: channels outside worlds, a message link format for graph objects,
and any presence/roster protocol. None of those needs new trust machinery — they are
addressing and convention on top of the signed-event layer, with retrieval delegated
to #2050. The sketch below keeps the IRC surface minimal and names exactly where the
Discord-shaped growth (threads, reactions, servers) attaches later without breaking it.

| Drew's ask                          | Today                                                                                            | Verdict                                             |
| ----------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| Channels/topics                     | worlds are addressed rooms with join + scoped chat, but there is no channel outside a game world | **partial — the room exists, the channel does not** |
| Identity = Iroh ID, no registration | endpoint ticket or bare node ID everywhere; petnames are private and never replicated            | **present**                                         |
| Messages linking graph objects      | no link format; blobs/artifacts move by hash + ticket but nothing embeds a reference in chat     | **absent**                                          |
| Presence/roster over P2P            | no presence protocol; discovery + direct connections exist, nothing publishes liveness           | **absent**                                          |
| Minimal protocol sketch             | envelope + world frames are transports, not a chat protocol                                      | **absent — this doc**                               |
| Growth path w/o breaking core       | unscoped                                                                                         | **absent — this doc**                               |

---

## 1. What the tree already gives chat

### 1.1 World chat: a room with scoped send, bounded messages, catch-up

`WorldEventKind::Chat(String)` ([`pkgs/id/src/world/mod.rs:330`](../../../pkgs/id/src/world/mod.rs))
is a first-class journaled event, committed through `WorldCore`
([`pkgs/id/src/world/core.rs:578,989`](../../../pkgs/id/src/world/core.rs)), sanitized on
the way in ([`sanitize_chat`](../../../pkgs/id/src/world/util.rs), `world/util.rs:78`),
and bounded by `WorldLimits::chat_bytes` (default 2048, `world/mod.rs` defaults) with a
per-participant rate limit (`CHAT_RATE_BURST = 50` per 10 s, `world/mod.rs:53-54`).
Capability scopes gate it: join/receive vs. send are separate grants, and a reader
without chat scope never sees chat events (`world/mod.rs:973`). Reconnect catch-up comes
free via `retained_events` (default 1024) and snapshot rebasing. Sanitized display names
(`sanitize_display_name`) already solve the "who said it" rendering problem for one room.

The limitation is shape, not substance: a chat exists only **inside a world** (a game
session), addressed by world, with no topic subdivision, no DMs, and no history beyond
the retention window. An IRC-shaped client needs the same event with a channel address
that is not a game.

### 1.2 Envelopes: signed 1:1 delivery with receipt semantics

`/id-envelope/1` ([`pkgs/id/src/envelope_net.rs:29`](../../../pkgs/id/src/envelope_net.rs))
is one bidirectional stream per payload: the sender writes a frame holding the payload's
JSON, the receiver answers with a [`Reply`](../../../pkgs/id/src/envelope_net.rs)
(`Applied` / `Duplicate` / `Refused` / `Retry`). The module's own contract states the
trust rule: _"The payload's signature is what makes it trusted, not the iroh node that
delivered it."_ That is exactly the property an IRC-style PRIVMSG needs: delivery over
whatever path, authenticity from the bytes.

Recipients are addressed as _"an endpoint ticket or a bare node ID"_
([`envelope_outbox.rs:75`](../../../pkgs/id/src/envelope_outbox.rs)), with tickets
parsed or bare IDs resolved through discovery (`envelope_outbox.rs:388-395`). No
registration step exists anywhere in this path — the node ID **is** the account.

### 1.3 Groups, invites, membership: the roster and the door

- [`groups/`](../../../pkgs/id/src/groups/mod.rs) (Phase 2, P2.4): members and roles,
  a signed policy record, templates, and **friendship as a two-member group**. State is a
  `fold` over events, order-independent; Admin role, member cap, dissolve.
- [`invites/event.rs`](../../../pkgs/id/src/invites/event.rs): fixed-layout signed
  invite events (`inviter || ticket_digest || group || role || expires_ms` + Ed25519 under
  domain tag `id-invite/v1/event`), bound to one ticket digest so a string-form mixup
  cannot redirect an invite.
- [`membership/`](../../../pkgs/id/src/membership/mod.rs): p2panda-auth-backed
  membership with gate/layer/frontier.

Together these are the roster (who may speak in a channel) and the door (how they get
in) — but they currently serve groups/worlds, not chat channels. A channel's member set
and operator set map onto this machinery with no new cryptography.

### 1.4 Transports: nine ALPNs, none of them chat

`id serve` registers metadata RPC, blobs, gossip, docs, event-log sync, world sessions,
envelopes, and artifact push/pull on one Iroh endpoint
([`serve.rs:956-984`](../../../pkgs/id/src/commands/serve.rs)), plus the SSH tunnel.
Chat traffic today rides **inside** world sessions (game frames) or envelopes (1:1) —
there is no channel-multicast transport. Gossip (`iroh_gossip`) is the natural
presence/channel fan-out carrier and is already wired; docs sync already demonstrates
multi-writer CRDT convergence over it.

### 1.5 What is _not_ ID chat (verified, to avoid confusion)

- `pkgs/chat/` is a Go SSH-room demo (charmbracelet wish/bubbletea, `localhost:23234`,
  `pkgs/chat/main.go`) — a toy room, unrelated to ID identity or protocol.
- `directory_mail.rs` is world confirmation email (SMTP sink), not messaging.
- `petname.rs`: private local aliases, never replicated, never sent — the anti-namespace:
  display names are local-only by design, which is why the protocol sketch below uses
  node IDs on the wire and leaves rendering to the client.

---

## 2. Minimal protocol sketch (what an IRC-shaped client needs)

Design rule, from the intake review: keep the IRC surface minimal; growth room stays as
notes, not requirements. Every item below reuses a primitive from §1.

### 2.1 Channel addressing

A channel is a named scope with a member set, independent of any world:

- Address form: `<scope>/<name>` where scope is one of `world:<id>` (existing world
  chat, unchanged), `group:<hash>` (a `groups/` group — roster and ops come from the
  signed policy record), or `dm:<node-id>` (a two-member implicit group; friendship
  already models this).
- Creating a channel = creating (or naming) the backing group; joining = the existing
  invite flow (`invites/event.rs`) or open-join where the policy record allows it
  (mirrors `grant_on_use` in world policy).
- No new namespace registry: names resolve inside the client's petname + roster view,
  exactly as account names already do. Globally unique channel names are a
  Discord-shaped problem and explicitly deferred.

### 2.2 Message format

One JSON frame on the existing envelope/world transports (no new ALPN in v1):

```json
{
  "v": 1,
  "channel": "group:<hash>",
  "author": "<node-id>",
  "seq": 42,
  "after": ["<hash>", "..."],
  "text": "gg, clip at @3:12",
  "links": [{ "kind": "blob", "hash": "<hex>", "author": "<node-id>", "hint": "<ticket?>" }],
  "sig": "<ed25519 over the above under domain tag id-chat/v1/message>"
}
```

- `seq` + `after` mirror the chain discipline (`chain.rs`: per-author sequence + backlink
  - causal frontier, domain-tagged) so channel history is a per-author-chain subset with
    the same fork semantics — and the same fork-proof exchange requirement the bridge
    investigation already imposed (LogSync compares heights, not heads).
- `text` keeps the world-chat bounds: 2048 bytes, `sanitize_chat` on ingest, 50/10 s
  burst cap per sender. Same numbers, no new tuning debate.
- Channel messages that need durability settle into signed events (the journal);
  everything else stays ephemeral — the journal/ephemeral boundary question #2058 left
  for #2051, with this issue's input: **chat text settles per-channel like world chat
  does; presence never settles** (§2.4).

### 2.3 Graph-object links (reference + resolution; retrieval is #2050)

A link is a reference triple the recipient can resolve without trusting the sender's
node: `{kind, hash, author}` plus an optional `hint` (an endpoint ticket or node ID
believed to hold a copy). Resolution order:

1. Local store: do I already have this hash (blob store / event store)?
2. Author: open the author's node (ticket or bare ID through discovery) and fetch —
   authenticity comes from the hash, so _any_ holder is a valid source.
3. Swarm: the #2050 chunk-retrieval machinery (BitTorrent-style) for objects whose
   author is offline.

Kinds in v1: `blob` (pictures, clips, files — content-addressed bytes) and `event`
(a signed event by hash, e.g. "that game result", "that picture's caption revision").
`game` and other rich kinds are **not** new link kinds — they are events the recipient's
client renders (a game invite is an event link + client affordance). Access control on
fetch is the existing gate story (`blob_gate`, `PullGate`, per the abuse-controls
investigation); the link itself carries no capability.

### 2.4 Presence and roster

Presence is liveness, not history: it is **never journaled, never synced, never
replayed** (the #2058 rule: "if it isn't signed, it isn't synced"). Semantics:

- Each node publishes `{node-id, channels[], status, ts}` heartbeats to channel peers
  over direct connections, fanned out via the already-wired gossip ALPN; `ts` + a short
  TTL (suggested 60–90 s) is the entire protocol. Stale = gone. No leave messages are
  required; rejoins are just fresh heartbeats (this is what makes netsplits boring).
- Roster = channel member set (from the backing group's fold) annotated with last-seen
  liveness. Roster intersection, not a new data type.
- Typing/keypress indicators ride the same heartbeat path with shorter TTLs — the
  small-packet lesson from the abuse-controls doc (20–60 frames/s is normal traffic)
  applies: presence is a trickle the token-bucket work already anticipates, not a new
  limiter debate.

### 2.5 IRC command mapping (the client contract)

| IRC idea             | ID realization (this sketch)                                                        |
| -------------------- | ----------------------------------------------------------------------------------- |
| NICK / registration  | none — node ID is the nick; client renders petname locally                          |
| JOIN `#ch`           | resolve channel → invite/open-join on the backing group                             |
| PRIVMSG `#ch` / user | channel frame (§2.2) via gossip/world transport; 1:1 via `/id-envelope/1`           |
| NAMES                | roster = group fold + liveness annotation                                           |
| WHOIS                | node ID → last-seen, channels in common, petname (local only)                       |
| TOPIC                | a signed channel event (settled, like world chat) — one key, last-writer-per-policy |
| KICK/BAN/MODE +o     | group policy record: Admin role, member cap (already exist)                         |
| History              | per-channel settled events; catch-up like `retained_events`, not a bouncer (yet)    |

---

## 3. Growth path that does not break the core

Each Discord/Slack/Teams feature attaches at a named seam, ships later, and degrades to
plain IRC text on clients that do not implement it:

- **Threads:** a message may carry `in_reply_to: <hash>`; clients render trees, the
  protocol still delivers a flat sequence. No new ordering semantics.
- **Reactions:** ephemeral presence-path annotations on a message hash (never settled —
  they are liveness, like typing). A settled "reaction" is a new message kind, not v1.
- **Servers/guilds:** a named bundle of channels = a group template (`groups/` already
  has templates) plus a TOPIC-style directory event. No new authority model.
- **Bouncers/history-as-service:** the hosted-edition bridge (#2058: the DO as passive
  always-reachable endpoint) is the natural always-on member that holds retention
  beyond `retained_events` — but it stays a member with no special ordering power (the
  #2058 rules: no bridge-assigned ordering, no bridge-held trust).
- ** moderation at scale:** per-counterparty budgets + block lists from the
  abuse-controls investigation apply unchanged; channel scope just adds "per channel"
  as one more budget key.

---

## 4. Open questions (owners named)

| #   | Question                                                                                                                                                                   | Blocks / owner                                        |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Q1  | Journal/ephemeral boundary for channel chat: settle every channel message like world chat, or settle on explicit save/quote? Cost is event-store + sync volume per chatter | #2051 synthesis — owner: #2051 (input: this doc §2.2) |
| Q2  | Gossip topic-per-channel vs. one chat topic with channel tags: fan-out efficiency vs. join/part churn                                                                      | #2051 synthesis — owner: #2051                        |
| Q3  | Link `hint` freshness: tickets rot when nodes change addresses — resolution fallback depth before giving up                                                                | #2050 retrieval — owner: #2050                        |
| Q4  | DM ranging: implicit two-member groups vs. pure envelope exchange with no group record — do DMs need a roster entry?                                                       | #2051 synthesis — owner: #2051                        |
| Q5  | Hosted-edition presence: the DO cannot initiate Iroh — who gossips presence to a hosted client (WS fan-out inside the DO, per #2058)?                                      | #2051 + bridge follow-ups — owner: #2051              |

---

## 5. Follow-ups filed from this investigation

- (Filed after the doc PR — links recorded on #2049 when opened.)
- PoC scope if the orchestrator wants one: two CLI nodes exchanging §2.2 frames over
  `/id-envelope/1` with a `blob` link resolved from the local store — no permanent
  feature code on top of it (per program rules, no building on an unreviewed PoC).

## Method note

Verified against `origin/main` at `c90fcbc7`: issue #2049 open with no owning PR and
no prior investigation doc; #2050 and #2051 open as the retrieval/synthesis partners;
all file references above read at that revision. Nothing in the tree contradicts the
issue's premise — chat outside game worlds is genuinely undesigned, and every
primitive the sketch reuses exists and is cited.
