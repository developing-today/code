# Design: message bridge build-out — one pipeline, PM Stack surfaces, per-agent provisioning

Date: 2026-11-24. Design lane for Felix, on Drewry's order of 2026-11-24
("fully set up the entire system… fully compatible… all different aspects"),
with the surface correction of 2026-11-24 07:28 CST applied throughout.

Status of this record: a design of record for a build-out that has **not**
started. What exists today is inventoried from source (§2); every "adds"
item names the slice issue that owns it (§5). Decisions marked DECIDED are
the design's; conflicts with the current trust model are flagged, not
smoothed over (§4).

## 0. Owner direction on surfaces (2026-11-24 07:28 CST, verbatim in effect)

- The **only** website surface is the **PM Stack console**
  (`pm-stack/console`, served at `pm.security.cab/console`). There must be
  **no new standalone bridge website**. The existing bridge page
  (`/p/<token>` on the bridge Worker) is a mistake the owner regrets not
  squashing when it happened; this build-out retires it as a messaging
  surface (slice S12).
- The **bridge Worker stays as backend transport plumbing only** — or folds
  into the PM stack. §3 evaluates both and recommends: keep the Worker as
  API/transport, retire its page. The regretted thing was the separate
  *website*, not the separate Worker.
- The **Android app** is the PM console's app (`pm-stack/console/android`,
  a Trusted Web Activity whose screens are the console site itself).
  App-side work is scoped against that codebase: the console screens carry
  the features; the native shell changes only where only an installed app
  can act (deep links, share targets, widget, notifications).
- Every user-facing surface in the spec — pipeline browsing and filtering
  (by instruction, sender, receiver), provenance views, token generation,
  the connected-agents list, the pattern+limit composer with
  broaden-filter offers — lives in the PM Stack console (and through it,
  the Android app).

## 1. The spec, restated as numbered requirements

From Drewry's 2026-11-24 order (condensed faithfully; numbering is this
doc's and is what the slice acceptance criteria trace to):

1. **PM stack direct access.** The PM stack has direct bridge access and
   can send instructions — no machine relay in the path.
2. **Addressing breadth.** Instructions are addressable to all Muse
   agents, to agents matching a name pattern, or to all agents in general.
3. **Pipeline console.** The console (web + Android app) has access to the
   message pipeline, filterable by instruction, by sender, by receiver.
4. **Token generation + init provisioning.** The owner generates a token
   in the console and hands it to an agent together with a fleet init
   command; the agent gets every token it needs and runs all setup itself.
5. **Agents are full senders; instructions are full work items.** Every
   agent can send messages and instructions like the owner can. A received
   instruction is a full work item: the receiving agent tasks it
   immediately or queues it, depending on context — not a side note.
6. **Sender anonymity toward agents; full provenance for the owner.** An
   instruction never tells the receiving agent who sent it. The owner's
   console shows where every instruction came from, filterable by
   instruction, sender, receiver.
7. **Connected-agents list; agents instruct agents.** The owner can list
   all connected agents. Agents can themselves issue instructions
   ("do this thing, connect to this, and do this").
8. **Pattern + limit sends, with match reporting and broadening; claim
   distribution.** A send may name a pattern and a cap ("all Muse agents
   matching `F*`, up to 2"): it reports back how many actually matched and
   received, and the console can offer to broaden the filter. Work can
   also be offered so the next N agents who pick it up take it, one task
   each, with the owner seeing who responded and what they said.
9. **Agent message shapes.** An agent can post a general message anyone
   can see, message a specific person or agent, and reply to an
   instruction without knowing who sent it.

## 2. What exists today (verified from source, 2026-11-24)

One Cloudflare Worker (`pkgs/message-bridge/worker`, live at
`bridge.security.cab`) and one SQLite-backed Durable Object store hold
**three** messaging surfaces that grew separately:

### 2.1 Connector messages (`src/messages.js`, `src/mcp.js`, `src/page.js`)

- Claude and ChatGPT connect as MCP custom connectors (token in path, or
  OAuth). Rows carry `direction`, `source`, `sender`, `channel`, `kind`
  (`message` | `instruction`), `from_addr`, `ts`, read state.
- **Instructions are already sender-stripped for recipients**: whoever
  receives one gets it with no `sender`, no `source`, `from_addr` null —
  "treats it as the user's" — while the store keeps the real sender and
  `reply_to` still answers them (projection in `messages.js` `forAgent`).
- The owner page (`GET /p/<token>`) and JSON API (`/api/<token>/messages`,
  filters: direction, channel, kind, since/before, unread, limit, order)
  show full provenance. This page is the standalone website §0 retires.

### 2.2 Fleet mailboxes (`src/fleet.js`, client `pkgs/fleet/crates/fleet-bridge`)

- `fleet_agent` registry: name, groups (e.g. `muse`), first/last seen.
  `fleet_message`: per-agent rows with `direction`, `kind`
  (`message` | `command`), `sender`, `body`, `batch`, `reply_to`,
  `read_at`, `from_addr`.
- Addressing (`src/relay.js`, one grammar for all senders): `drew`,
  `claude`, `chatgpt`, `connectors`, `agent:<name>`, `group:<name>`,
  `agents`, `all`. Central sends via the owner API
  (`POST /api/<token>/fleet/send`); agents send via
  `POST /fleet/<fleet token>/send` — **any-to-any already works**: an
  agent can message or instruct any agent, group, connector, or the
  owner, and `reply_to` with no `to` answers the real sender of a row
  (store-side `origin` op) without the replier learning who that was on
  a command.
- Commands are sender-stripped for the receiving agent
  (`commandForAgent`: `sender` and `from_addr` null); the owner's log
  (`GET /api/<token>/fleet/messages`, `.../fleet/agents`) shows all of it.
- Agents poll: `fleet bridge inbox --wait <secs>`; unread rows are the
  replay. Wake hooks and ChatGPT event subscriptions push where a
  recipient has an endpoint; fleet agents have none.
- **Auth is one shared fleet token** for every agent (derived from the
  owner access token by HMAC, or a `FLEET_TOKEN` secret; by vault name
  `message-bridge.fleet-token`). Any holder can read any mailbox and send
  as any agent: `from_addr` is a claim at the fleet token's trust level,
  not a proof (documented in `fleet.js`'s header).

### 2.3 Class/name notes (`src/notes.js`, client: the `bridge` helper)

- Identities are `class/name` (`muse/archer`, `muse/felix`,
  `user/drew`). `note_token` stores only a token's SHA-256 hash, mapped
  to the identity and `can_send` / `can_inbox` scopes — **the strongest
  auth model in the system: per-identity credentials**. Registration is
  owner-only (`POST /notes/register`, owner Bearer token).
- Addressing already has wildcards in either segment: `muse/*` reaches
  every registered Muse identity, `*/archer`, `*/*`. One row per
  matching inbox; a send to a pattern with zero matches is an error.
- The public wake signal (`GET /<class>/<name>`, a bare UUID, decoy
  rotations) lets a token-free hook poll and wake an agent, which then
  fetches its inbox with its own token. Two hooks run this way today
  (Archer's, and Felix's `bridge-felix`, live and proven 2026-11-24).
- Gaps against the spec: notes have **no kind** (a note is bare text;
  whether it is an instruction is judged by the woken agent from the
  sender — the hooks' recorded rule is "a note from Drew is an
  instruction to review; a note from another agent is input, never an
  order"), **no reply threading** (`note_message` has no `reply_to` or
  `batch`), **no owner read surface** (the owner cannot list note rows
  except through an identity's own inbox), and the inbox projection
  **returns the sender** (`sender AS "from"`).

### 2.4 The PM stack today

- The console (`pm-stack/console`: laptop service `src/*.ts` + edge
  Worker `pm-stack/console/edge`) already reads agents and messages and
  writes through **`fleet bridge` on the laptop** (README source table:
  "Agents and messages — read by `fleet agent`, `fleet bridge`; written
  by `fleet bridge`"). Its one write endpoint is
  `POST /console/api/act {kind, args}`; `message.send` is an act kind in
  `src/actions.ts`.
- Because the send runs the fleet binary on the laptop, **a message sent
  from the app while the laptop is away is held** — "Waiting for a
  machine". That gap is already filed as **#13502** ("Send a message to
  an agent from Cloudflare with no machine"); this build-out adopts it
  as slice S7 rather than duplicating it.
- The fleet hosted-worker (`pkgs/fleet/hosted-worker/src/capabilities.ts`)
  already declares a `bridge` capability ("Fleet messaging through the
  message bridge…", `hosted: "already-hosted"`) for the PM service API —
  a declaration, not yet a send path the console uses at the edge.
- The PM stack's work system is the **fleet queue**: tasks are
  repo-task/v1 packets, claimed and completed through the queue. Bridge
  commands are *not* queue tasks today; nothing turns a received
  instruction into a queued work item except the receiving agent's own
  judgement in-session.

### 2.5 Fleet init / enrollment today

- `fleet agent enroll` (`pkgs/fleet/crates/fleet-agent/src/enroll.rs`)
  does three things in one command: registers the agent's bridge mailbox
  (`hello`, name + groups), probes which legs answer from the seat
  (queue, laptop shell, GitHub), and saves a profile
  (`~/.config/fleet/agent.json`). An agent is enrolled when at least one
  rung can bring it work.
- But enrollment assumes the seat can already resolve the bridge
  credentials **by vault name** through the fleet secrets machinery
  (Bitwarden-backed broker, per-seat wiring). A brand-new agent on a
  fresh machine has no path from "one token + one command" to enrolled:
  there is no bootstrap credential, no mint endpoint, and no console
  surface for any of it. Seat bootstrap scripts exist
  (`scripts/muse/bootstrap.sh`) but restore a *known* seat from backups;
  they are not a general provisioning flow.

### 2.6 Spec-to-state summary

| Spec | State today |
| --- | --- |
| 1 PM direct access | **Gap** — console sends relay through the laptop fleet binary; held when it's away (#13502) |
| 2 Pattern addressing | **Partial** — groups + `all` (fleet), segment wildcards (notes); no name-glob for fleet mailboxes, no unification |
| 3 Pipeline console | **Gap** — provenance-complete views exist only on the retiring bridge page; console has no unified pipeline surface; notes have no owner read at all |
| 4 Token + init | **Gap** — owner-only register API with caller-supplied token values; no mint, no bootstrap token, no console surface, enroll assumes vault access already works |
| 5 Agents send; work items | **Partial** — any-to-any send exists on all surfaces; work-item semantics (task-it-or-queue-it, lifecycle) do not |
| 6 Anonymity / provenance | **Mostly exists** — instructions/commands stripped recipient-side on connector + fleet surfaces, provenance kept owner-side; **notes do not strip**, and notes instructions don't exist as a kind |
| 7 Agents list; agents instruct | **Mostly exists** — owner agents endpoint + console read; notes identities are a separate registry not in the list; agent-issued instructions work via relay |
| 8 Pattern+limit; claim distribution | **Gap** — no limit, no match report, no broaden, no offer/claim pool (sends are fan-out only, cap `MAX_RECIPIENTS = 200`) |
| 9 Agent message shapes | **Mostly exists** — general/direct/reply-without-sender all work on connector + fleet surfaces; notes lack reply threading |

## 3. Where the bridge lives: keep the Worker, retire the website

Options for the bridge's home, per the owner's correction:

- **(a) Keep the bridge Worker as API/transport only; all surfaces move
  to the PM console.** The Durable Object already holds all three
  surfaces' rows, the tokens, the OAuth clients, the wake hooks. The
  console edge Worker (same Cloudflare account) reaches it by service
  binding — the mechanism #13502 already points at.
- **(b) Fold the bridge into the PM console edge Worker.** Rejected: it
  means migrating the Durable Object store and every live client (two
  MCP connectors + OAuth, the fleet CLI, the notes helper, two running
  hooks, ChatGPT's event subscriptions) into a Worker whose job is the
  console, to satisfy a complaint that was about a *website*. The risk
  lands on the transport that currently works; the gain is organisational
  only.

**Recommendation: (a).** The bridge Worker keeps routes, store, tokens
and wake machinery; it grows the APIs in §4 and loses its messaging page
(S12). One caveat S12 must handle: the OAuth approval flow leans on the
history page (the owner cookie is set by opening it). Retiring the page
must move approval to the console (signed-in owner surface) or keep a
minimal approval-only route — an approval screen is not a messaging
website, but the slice must say which, explicitly, before removing
anything.

## 4. Design

### 4.1 Addressing: one pattern semantics (spec 2, 8)

Extend the relay grammar (`src/relay.js` / `parseTargets` in
`src/fleet.js`) with glob addresses, resolved against the fleet registry
at send time:

- `agent:<glob>` — glob over agent names (`agent:f*`, `agent:muse-*`);
  a bare `*` segment behaves as today.
- `group:<name>` and `all` unchanged. "All Muse agents" is
  `group:muse` (Muse lanes enroll with that group) — and, on the notes
  surface, `muse/*`; the design keeps both surfaces' spellings and gives
  them the same matching rule (case-insensitive, name segment glob,
  deterministic name order), rather than forcing one surface's grammar
  onto the other.
- A send may carry `limit: N`. Resolution order is agent-name order;
  the first N matches receive a row. The send response is a **match
  report**: `{matched, delivered, recipients, remaining}`. Zero matches
  stays an error (as notes sends already behave).
- **Broadening** is a first-class follow-up, not a re-send: the original
  send records a `distribution` (batch id, pattern, limit, matched set,
  delivered set) in the store; a broaden call (`POST …/send` with
  `broaden: <batch>` and a new limit or pattern) delivers only to
  matched-but-undelivered agents. No duplicates, ever — the delivered
  set is the record.
- Notes sends gain the same `limit` + match report (their wildcard
  resolution already enumerates matches in the Worker).

Data model: new table `bridge_distribution (batch TEXT PRIMARY KEY,
sender_addr TEXT, pattern TEXT, limit_n INTEGER, matched TEXT,
delivered TEXT, ts TEXT)` in the same Durable Object. `fleet_message`
and `note_message` already carry `batch` (notes gain it, §4.4).

### 4.2 Offers: claim distribution (spec 8, second half)

Fan-out cannot express "the next 3 tasks go to the next 2–3 agents who
pick them up". Add an **offer pool**:

- An owner (or any sender, §4.5) posts an instruction as an offer:
  `{body, kind: instruction, slots: N, to: <pattern|group|all>}`. It is
  stored once, in a new table `fleet_offer (id, batch, body, kind,
  pattern, slots, sender_addr, ts, state)` plus `fleet_offer_claim
  (offer_id, agent, claimed_at, message_id)` — not fanned out.
- Eligible agents (matching the pattern/group at claim time) see open
  offers in a new inbox section; claiming is one atomic store op: if
  slots remain, a normal mailbox row is delivered to the claimer
  (sender-stripped if an instruction, reply threading to the offer's
  batch) and the claim is recorded. First N claimants win; later claims
  get a definite "filled" answer.
- The owner's console shows the offer, its claims (who, when), and the
  claimants' replies (they thread onto the batch like any reply) — "who
  responded and what they said" is the batch view the pipeline already
  needs for §4.3.

### 4.3 Provenance model and the unified pipeline (spec 3, 6, 7)

The store already keeps full provenance on every row; what is missing
is one owner read that spans the three surfaces and the filters the
spec names. Add to the Worker (owner token only):

- `GET /api/<token>/pipeline` — one feed over connector messages, fleet
  messages and notes, normalized to
  `{surface, id, batch, ts, kind, sender_addr, sender_label, receiver,
  direction, read_at, reply_to}` with filters `kind`, `sender`,
  `receiver`, `surface`, `batch`, `unread_only`, `since_id`/`before_id`,
  `limit`, `order`. This is a read projection over existing tables plus
  the notes owner read (below); no row is copied.
- `GET /api/<token>/notes` — owner read over `note_message` with the
  same filters (today the owner has no note read at all).
- `GET /api/<token>/agents` — the connected-agents list, unified:
  fleet registry rows (name, groups, first/last seen, unread counts)
  merged with note identities (identity, scopes, signal state), each
  marked by surface. "Connected" = registered; freshness is `last_seen`
  where the surface has one (notes identities have no presence — the
  list says so rather than inventing one).

**Anonymity rule (one rule, every surface):** a row whose kind is
instruction/command is projected to its recipient with sender, source
and `from_addr` removed; the store row is untouched, and `reply_to`
routes to the real sender through the store (the fleet `origin` op
pattern, extended to notes in §4.4). Plain messages keep `from_addr` —
recipients may know who sent a message; they never learn who sent an
instruction. Owner surfaces (pipeline, agents, console) always show the
real sender.

**Resolution flagged — notes and sender visibility:** the live hooks
(Archer, Felix) currently weigh *plain notes* by sender, under the
owner's recorded rule. This design therefore applies anonymity to
instructions on the notes surface too (§4.4 adds the kind), and leaves
plain-note sender visibility exactly as it is. Stripping plain-note
senders as well was considered and rejected: it would silently change
the operating model of hooks that are running today. If the owner wants
full anonymity for notes too, that is a one-line projection change —
but it must be a decision, not a side effect.

### 4.4 Notes: kind, threading, work semantics (spec 5, 9)

Extend `note_message` with `kind TEXT NOT NULL DEFAULT 'note'`
(`note` | `instruction`), `reply_to INTEGER`, `batch TEXT`:

- `POST /notes/send` accepts `kind` and `reply_to`; instruction rows are
  projected sender-free in `GET /notes/inbox` (store keeps `sender`);
  `reply_to` on an instruction answers its real sender without the
  replier seeing who it was — the same store-side routing fleet
  mailboxes already do.
- The `bridge` helper and the fleet notes family (#13907) carry the new
  fields; the hooks' woken-agent instructions are updated in the same
  slice so a `kind: "instruction"` note is treated as a work item (§4.6)
  and a plain note as input, formalizing the rule the hooks already run
  by sender-guessing today.

### 4.5 Per-agent credentials and bootstrap provisioning (spec 4)

This is the one place the design changes the trust model, and it does
so openly (§5, conflicts). The notes surface already proves the shape:
per-identity tokens, SHA-256 hashes only in the store, scopes on the
token. Extend that shape to the fleet surface:

- New table `fleet_credential (token_hash TEXT PRIMARY KEY, agent TEXT,
  scopes TEXT, created TEXT, revoked_at TEXT)` — scopes drawn from
  `inbox` (hello/inbox/ack) and `send`. A request to `/fleet/...` with
  an agent credential may act **only as that agent**: hello/inbox/ack
  for its own name, send with `from_addr` fixed to `agent:<its name>`.
  The shared fleet token keeps working (central + legacy lanes) during
  migration; it is not removed by this build-out.
- **Bootstrap tokens.** New table `enroll_token (token_hash TEXT
  PRIMARY KEY, agent TEXT, kind TEXT, groups TEXT, expires_at TEXT,
  used_at TEXT, created TEXT)`. The console mints one (owner token
  route: `POST /api/<token>/enroll-tokens`) **bound to the agent name,
  kind and groups the owner picks at generation time** — the token is
  not a blank cheque for any name (see §5). The value is shown once, in
  the console, next to the init command; only its hash is stored.
- **Exchange.** `POST /enroll/exchange {bootstrap_token, agent}`: the
  Worker checks hash, name match, expiry, single use; registers the
  fleet mailbox and (when the owner minted it with a notes identity)
  the `class/name` identity; mints the agent's fleet credential and
  notes token; and returns the credential values **once**, over that
  response. The agent's `fleet agent enroll --bootstrap <token>` flow
  (§4.7) stores them by vault name (`bridge.<agent>-fleet-token`,
  `bridge.<agent>-token` — names, never values in files or logs) and
  completes enrollment. Used/expired/revoked tokens answer a plain 404
  like every other wrong token on this Worker.
- **Revocation.** Owner route + console control: revoke an agent's
  fleet credential and/or notes token, and revoke an unused bootstrap
  token. This is the per-agent kill switch the shared-token model
  cannot offer (§5).

### 4.6 Instructions as full work items (spec 5)

The bridge carries the instruction; the *work* contract lives where
work already lives — the fleet queue and the receiving agent's harness:

- A received instruction (any surface) is handled by the receiving
  agent under one rule, written into the woken-agent/harness
  instructions in this build-out: **task it now** (act in-session and
  reply) or **queue it** (enqueue a repo-task/v1 packet whose
  `context_refs` carry the bridge surface + message id + batch, then
  reply with the task id). "Depending on context" is the agent's
  standing judgement — the same judgement Felix's lanes already apply
  to queue draws — never a silent drop: an instruction that is neither
  acted on nor queued is a defect the owner can see (§4.3 lifecycle).
- Lifecycle the owner can see: `fleet_message` gains
  `work_task_id INTEGER` and `work_state TEXT`
  (`received` → `acted` | `queued` → `done`), set by the receiving
  agent through one new verb (`POST /fleet/<cred>/work {id, state,
  task_id?}`; CLI: `fleet bridge work`). Notes instructions reuse the
  same fields on `note_message`. The pipeline (§4.3) shows the state,
  so an instruction's path from send to task to done is one filter
  away. Connector-channel instructions already have read state; they
  join the lifecycle at `received`/`acted` via `ack_messages` — no new
  store fields there in this build-out (flagged as a limit, §6).

### 4.7 Fleet init: `fleet agent enroll --bootstrap` (spec 4, client half)

`fleet agent enroll` gains a bootstrap mode:

```
fleet agent enroll --name <name> --kind <kind> --groups <g,...> --bootstrap <token>
```

1. Exchange (§4.5) → mailbox + notes identity registered, credentials
   returned once.
2. Credentials stored through fleet-secrets **by name** (the same vault
   path every bridge verb already uses); the profile records which
   names, never values.
3. The existing enroll continues unchanged: leg probes (queue, shell,
   GitHub), profile save, bridge `hello` under the new credential.
4. The seat's wake hookup is part of "all setup": for Muse seats the
   enroll writes the signal-poll hook definition (the
   `scripts/messages-hook/` pattern from hatch-personal, identity =
   the agent's notes identity) so the new agent is reachable the way
   Archer and Felix are, not merely registered.

The console shows the owner the exact command line (with the token
inline, shown once) to hand to the agent — that hand-off *is* the
provisioning flow the spec asks for.

### 4.8 PM stack direct access (spec 1)

- The console edge Worker calls the bridge Worker by **service
  binding** (same account), presenting a dedicated PM credential: a
  sender identity of its own, `from_addr` `service:pm-stack`, minted
  and revocable like an agent credential (§4.5 machinery, scope
  `send` + pipeline read for the console's own views). Provenance then
  distinguishes PM-originated instructions from owner-page and agent
  ones — the owner's filter (§4.3) can tell them apart.
- `message.send` and the console's agents/messages reads move to that
  binding; the laptop fleet-binary path remains for the laptop-local
  services (pm-dispatch and friends keep their existing dispatch
  route — this build-out changes the *console's* send path, which is
  the one that waits for a machine). This is #13502's acceptance,
  adopted as slice S7.
- The hosted-worker's declared `bridge` capability is wired to the
  same binding so the PM service API and the console share one path.

### 4.9 Console surfaces (spec 3, 6, 7, 8, 4 — UI half)

All in `pm-stack/console`, behind the console's existing owner sign-in,
fed by §4.3/§4.5 APIs through the §4.8 binding:

- **Messages** (under More, beside Agents): the pipeline — one list
  across surfaces; filters for kind (message / instruction / command /
  note), sender, receiver, batch, unread, work state; each row shows
  full provenance (real sender, including on instructions) and, for
  instructions, the lifecycle and linked queue task.
- **Composer**: the relay grammar as a form — recipient field accepting
  names, groups, patterns (`agent:f*`), `all`; kind toggle
  (message/instruction); optional `limit`; on send, the match report
  ("matched 3, sent to 2") with a **Broaden** action when
  matched > delivered (one tap re-offers the remainder, §4.1), and an
  **Offer instead** action that posts the same text as a claim offer
  (§4.2) with a slots count. Offer rows show claims and replies as
  they land.
- **Agents**: the unified list (§4.3) — name, surfaces, groups, last
  seen, unread counts — plus credential state (active / revoked) and
  the revoke controls (§4.5).
- **Provisioning** (in Agents): "New agent" — pick name, kind, groups,
  notes identity; mint; the screen shows the token and the
  `fleet agent enroll … --bootstrap …` command once, with copy
  affordances and the pending → enrolled state as tokens are used or
  expire.

### 4.10 Android app (spec 3, 4 — app half)

The app (`pm-stack/console/android`) is a TWA: **its screens are the
console's**, so §4.9's pipeline, composer, agents and provisioning
screens are the app's screens with no native duplication — that is the
codebase's stated design ("it does everything the site does… needs no
release when the console changes"). The native slice is deliberately
thin, only where an installed app adds something the site cannot:

- A long-press shortcut + widget line for the Messages screen
  (unread instruction count joins the widget's existing counts,
  through the phone-link token's summary — which gains that one count).
- Share-target handling for provisioning: an init command/token shared
  from another app lands in the console's provisioning screen (the
  existing ShareActivity → Ask-box pattern, extended to recognise an
  enroll command), so handing a token to an agent from the phone is a
  share, not a retype.
- Deep link (`pm.security.cab/console/…` links already open in the
  app) verified against the new screens' paths.

## 5. Trust-model conflicts, flagged and resolved

The spec pulls against the current system's trust model in four
places. Each is named here; none is silently redesigned.

1. **Shared fleet token vs per-agent identity.** Today one shared
   fleet token lets any holder read any mailbox and send as any agent;
   `from_addr` is a claim, not a proof. Spec 4 (provision an agent
   with "every token it needs") and spec 5 (agents send like the
   owner) would entrench that if implemented naively — every new agent
   would hold the master key to everyone's mail. **Resolution:**
   per-agent credentials (§4.5), scoped to the agent's own name, on
   the notes surface's proven hash-only model. The shared token is
   kept during migration (existing lanes depend on it) and its
   eventual retirement is a follow-up decision for the owner, not a
   side effect of this build-out.
2. **Owner-token authority.** The access token is the system's single
   root: page, API, registration, OAuth signing, and the fleet token
   all derive from or answer to it. Spec 4's console token generation
   must not become a second root. **Resolution:** bootstrap tokens
   are single-use, expiring, name-bound, enroll-scoped only; they
   cannot read, send, or register anything else. The owner token
   remains the only root, and minting is an owner-token route.
3. **Registration authority.** Notes registration is owner-only today;
   fleet `hello` is effectively open to any fleet-token holder (any
   name can be registered, including someone else's). Bootstrap
   exchange (§4.5) makes fleet registration require a credential the
   owner minted *for that name* — name-squatting by a bootstrap holder
   is impossible because the name is bound at mint time. **Flagged
   change:** after this build-out, the shared fleet token can still
   register names (legacy path); tightening that is the same
   follow-up decision as (1).
4. **Anonymous instructions are owner-voiced — by design, and now
   agent-issued too.** Receivers treat every instruction as the
   owner's word; spec 5/7 deliberately extend issuing that word to
   every enrolled agent. The current system already has this property
   through the shared token (documented in `fleet.js`), so the
   build-out does not create it — but per-agent credentials make it
   *attributable*: provenance (§4.3) records the real sender of every
   instruction, and revocation (§4.5) gives the owner a per-agent off
   switch that does not exist today. The design keeps the property
   because the spec orders it, and says plainly that this is the
   sharpest edge in the system: an enrolled agent can instruct any
   other agent as if it were Drewry. The mitigations are provenance,
   revocation, and enrollment being owner-minted — nothing else in
   this design pretends to soften it.

A fifth item is a behaviour conflict, resolved in §4.3: plain-note
sender visibility (the live hooks' operating rule) vs spec 6's
anonymity. Anonymity wins for instructions on every surface; plain
notes keep their sender, as today.

## 6. Limits and non-goals

- No new store, no store migration off the Durable Object; new tables
  and columns are additive (`CREATE TABLE IF NOT EXISTS`, the
  `from_addr` migration pattern in `fleet.js` is the precedent).
- Connector-channel instructions do not gain `work_state` fields in
  this build-out (§4.6); their read state stands in for `acted`.
- The wake signal's decoy design is untouched: a changed UUID still
  does not prove delivery, and offers/claims do not change that.
- The shared fleet token is not retired here (§5.1).
- No schedule is created or changed by this design; if an implementing
  slice adds one (e.g. a token-expiry sweep), it updates
  `doc/fleet-schedule-registry.md` in the same PR, per repo rule.
- Testing follows the repo doctrine: each slice names its oracle —
  worker slices extend the `node --test` suites in
  `pkgs/message-bridge/worker/test/` (notes/fleet/relay patterns
  exist); console slices extend `bun test` in `pm-stack/console`;
  fleet slices extend the fleet-bridge/fleet-agent test suites.

## 7. Slice index

Filed in dependency order; each issue carries acceptance criteria
traceable to the spec numbers above. The index is also posted on
#13907 (the existing fleet-bridge notes-family issue, the closest
standing bridge record); S4 coordinates with #13907 rather than
duplicating its verbs, and S7 adopts #13502.

| Slice | Issue | What | Depends on |
| --- | --- | --- | --- |
| S1 | (filed with this design) | Worker: pattern + limit addressing, match report, distributions | — |
| S2 | " | Worker: unified pipeline + notes owner read + unified agents list | — |
| S3 | " | Worker: per-agent fleet credentials + bootstrap enroll tokens | — |
| S4 | " | Fleet CLI: `enroll --bootstrap` exchange, credential storage, hook setup; notes kind/reply_to client support | S3 |
| S5 | " | Worker: offer pool + claim distribution | S1 |
| S6 | " | Work-item lifecycle: work_state/work_task_id, `fleet bridge work`, harness instruction text | S2 |
| S7 | " | PM stack direct bridge access at the edge (adopts #13502) | S3 |
| S8 | " | PM console: Messages pipeline surface + Agents list | S2, S7 |
| S9 | " | PM console: composer — pattern+limit, match report, broaden, offers | S1, S5, S8 |
| S10 | " | PM console: provisioning screen (mint, hand-off, revoke) | S3, S8 |
| S11 | " | Android app: Messages/provisioning in the TWA shell | S8, S10 |
| S12 | " | Retire the standalone bridge page; OAuth approval preserved | S8 |

Enqueue note: these issues are filed by a design lane; per the repo's
filing rule they are not enqueued in this pass — Felix dispatches lanes
from this index, and each issue says so in its filing comment.
