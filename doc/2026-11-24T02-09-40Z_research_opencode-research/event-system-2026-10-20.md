# The OpenCode event system — what it is, what of yours it touches, and what it's good for

Researched 2026-10-20 from: the fork source at `~/git/devtoday-opencode` (packages/core,
packages/schema, packages/server, packages/protocol), OpenChamber at
`~/git/devtoday-openchamber` (packages/web/server/lib/event-stream), the deployed plugin
set in `~/.config/opencode/plugin`, and the shipped opencode 1.18.31 binary itself
(nixpkgs build, string census of the compiled bundle).

## TL;DR

- The "experimental event system" is OpenCode 2.x's event-sourced bus: typed,
  schema-defined events, the important ones written to a durable per-session log in
  SQLite with sequence numbers, replayable over HTTP. It replaces the 1.x in-memory
  fire-and-forget bus.
- The flag `OPENCODE_EXPERIMENTAL_EVENT_SYSTEM` is a **vestige**. In the 1.18.31 build
  you run today it is parsed and then ignored (evidence below). In your fork's 2.x
  source it is not read anywhere at all — the new bus is the only bus, there is nothing
  left to switch. **Enabling it changes nothing on any system you own, today.**
- The real change to plan around is not the flag, it's the 1.x → 2.x move itself
  (which OpenChamber already requires): event names, payload shape, and endpoints all
  change. Anything you ever write against the event stream should target the 2.x shape.
- Analytics: no dashboards, counters, or metrics endpoints. But usage (cost + tokens)
  is recorded as durable event data, and the log is queryable — the raw material for
  analytics is there; aggregation is DIY (by design — that's what projectors are for).
- Bridge potential is the genuinely interesting part: typed subscriptions, replay
  with cursors, a catch-up watermark, and plugin RPC events on the same stream make
  this a real integration surface, where the 1.x bus was a UI convenience.

---

## 1. What the reworked event system actually is

### The old system (1.x — what opencode 1.18.31 runs today)

- An in-memory publish/subscribe bus inside the server process. Modules define events
  ad hoc; publishing is fire-and-forget. Nothing is stored, nothing is ordered, events
  have **no IDs** (OpenChamber's own stream docs note: "OpenCode 1.18 sends no ids at
  all").
- Wire: SSE at `GET /event` (per-directory) and `GET /global/event`, frames shaped
  `{ type, properties }`. Vocabulary: `message.updated`, `message.part.updated`,
  `message.part.delta`, `session.created/updated/deleted`, `session.idle`,
  `session.status`, `permission.asked`, `todo.updated`, `file.edited`, `lsp.updated`,
  `installation.updated`, …
- A consumer that disconnects, restarts, or falls behind has exactly one recovery:
  refetch all state and hope. There is no history to ask for.
- Plugins receive events through one catch-all `event` hook.

### The new system (2.x — your fork's source, unconditional there)

One typed bus (`packages/core/src/bus.ts`) fed by schema-defined events
(`packages/schema/src/*-event.ts`, inventoried in `event-manifest.ts`). Two classes:

- **Durable events** — facts about an _aggregate_ (almost always a session). Written
  to SQLite (`event` + `event_sequence` tables, `packages/core/src/event/sql.ts`) in
  the same transaction as the state projections they produce, with a per-aggregate
  sequence number and a per-type schema version. Session state is a projection folded
  from this log — this is event sourcing, not a notification bus with a database
  attached. (The repo's own AGENTS.md rule: "Keep durable events minimal: record
  irreducible new facts and do not repeat state derivable by folding the ordered
  aggregate history.")
- **Ephemeral events** — live-only, never stored: the streaming deltas
  (`session.text.delta`, `session.reasoning.delta`, `session.tool.input.delta`),
  `session.usage.updated`, TUI events, etc.

Every payload on the bus has the same envelope:
`{ id: "evt_…", created, type, data, location?, metadata?, durable?: { aggregateID, seq, version } }`.

Bus operations (`packages/core/src/bus.ts` `Interface`):

- `publish` / `publishAll` — typed, schema-validated.
- `subscribe()` — all events, one type, or a list of types; with an ambient Location
  you only get that location's events plus global ones. Session moves deliver to both
  the old and new location.
- `log({ aggregateID, after, follow })` — read one aggregate's durable log from a
  sequence cursor, optionally transitioning from replay to live; emits one
  `log.synced` marker at the replay watermark so a consumer _knows_ it has caught up.
- `project(definition, projector)` — register a fold over an event type (how read
  models/projections are built).
- `replay(...)` — re-ingest serialized events with divergence/ownership checks (this
  is the clustering/import path; note the fork docs keep "event replay ownership
  separate from clustered Session execution ownership").

### Event types (from `packages/schema/src`)

~55 `session.*` types alone, in families:

- Lifecycle: `session.created`, `forked`, `moved`, `deleted`, `renamed`, `viewed`,
  `metadata.updated`, `instructions.updated`, `agent.selected`, `model.selected`,
  `session.permissions`
- Execution: `session.execution.started|succeeded|failed|interrupted`,
  `session.step.started|streamed|ended|failed`, `session.retry.scheduled`
- Prompt queue (the inbox is event-sourced too): `session.inbox.enqueued|delivered|
cancelled|delivery.changed`
- Content: `session.text.started|delta|ended`, `session.reasoning.*` (same trio),
  `session.tool.input.started|delta|ended`, `session.tool.called|progress|success|
failed`, `session.message.content.updated`
- Context: `session.compaction.started|delta|ended|failed`, `session.compacted`,
  `session.skill.activated`, `session.shell.started|ended`
- Accounting: `session.usage.recorded` (durable, internal), `session.usage.updated`
  (ephemeral)
- Revert: `session.revert.staged|committed|cleared`

Plus: `agent/provider/model/command/config.updated` family, `mcp.status.changed`,
`mcp.resources.changed`, `mcp.tools.changed`, `lsp.updated`, `vcs.branch.updated`,
`worktree.ready|failed`, `workspace.ready|failed|status`, `location.shutdown`,
`installation.update-available|updated`, `tui.command.execute|prompt.append|
session.select|toast.show`, `messaging.message.created` (your fork's inter-session
messaging), `server.connected`, `server.resync`, `global.disposed`, and plugin-defined
`rpc.*` events.

### Wire format / API changes for consumers

- Endpoint: `GET /api/event` (SSE), serving the server manifest
  (`EventManifest.ServerDefinitions`) + `rpc.*` plugin events + the two server frames.
  The per-directory `/event` and `/global/event` endpoints and the
  `{ type, properties }` shape are gone; payloads are now `{ id, created, type, data,
location?, metadata? }`.
- Frames carry real SSE ids of the form `<instance>:<sequence>`. Reconnect with
  `Last-Event-ID` and the server replays from a bounded in-process buffer (1,024
  frames / 4 MB, `packages/server/src/event-feed.ts`); if your cursor is foreign or
  evicted you get exactly one `server.resync` frame and refetch state — no silent
  gaps, no duplicates across the replay/live seam.
- Backpressure is explicit: a subscriber queue is 4,096 frames; a slow consumer's
  stream is _failed_ rather than stalling the bus. 15 s heartbeat comments.
- History endpoint: `GET /api/experimental/session/:sessionID/log?after=<seq>&
follow=true` streams one session's durable log (replay, then live if asked).
- There is **no native `session.status` / `session.idle`** in the 2.x vocabulary.
  Busy/idle/retry is derived from `execution.*`, `step.started`, and
  `retry.scheduled`. (The schema still defines status/idle types for the TUI's public
  stream; OpenChamber synthesizes the old vocabulary from execution events — see §2.)
- One retention caveat: durable _payloads_ are only kept for historical reads when
  the server runs with `events.persist` on. The default local/embedded profile leaves
  it off (sequences and projections are still maintained; the hosted workerd profile
  sets `persist: true` — `packages/server/src/workerd.ts`, wiring in
  `packages/server/src/routes.ts`). So "durable" by default means "drives state and
  sequencing"; full history retention is a server option to flip on deliberately.

### The flag itself — the evidence

- The docs list it (`packages/web/src/content/docs/cli.mdx`, env table) — inherited
  from upstream.
- In the shipped 1.18.31 binary: the string `OPENCODE_EXPERIMENTAL_EVENT_SYSTEM`
  occurs exactly twice — both are the Flag-service definition
  (`experimentalEventSystem: bool("OPENCODE_EXPERIMENTAL_EVENT_SYSTEM")`, present in
  two bundles inside the binary). The parsed property `experimentalEventSystem` has
  exactly **one** consumer in the whole binary: the TUI's built-in plugin-list
  builder, which receives the value and returns the same plugin list either way.
  The server, CLI, and plugin host never read it.
- In your fork's 2.x source: the string occurs **nowhere in code** (docs only). There
  is no gate because there is no second system.

Interpretation: during the 1.x → 2.x transition the flag selected the TUI's new
event handling; the migration landed, the branch was removed, and the plumbing stayed.
Setting it now is harmless and meaningless in both lines.

---

## 2. Inventory — your systems, one by one

| System                                                                        | Consumes opencode events?                           | Effect of the flag                                                                                                                                                                                                           |
| ----------------------------------------------------------------------------- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| opencode server 1.18.31 (this sandbox; your current builds)                   | Produces the 1.x stream (`/event`, `/global/event`) | **None.** Server code never reads the flag (binary census above).                                                                                                                                                            |
| opencode TUI 1.18.31                                                          | Yes — it's the primary 1.x consumer                 | **None in this build.** The flag is plumbed into the TUI plugin-list builder, which ignores the value.                                                                                                                       |
| OpenChamber (your fork)                                                       | Yes — heavily                                       | **None, and it doesn't pair with 1.18.31 at all.** OpenChamber requires OpenCode ≥ 2.0.20 (`packages/web/server/lib/opencode/compatibility.js`) and consumes the 2.x `/api/event` stream. On 2.x builds the flag isn't read. |
| OpenCode desktop app (fork `packages/desktop` / `packages/app`)               | Yes, via the 2.x client                             | **None.** The desktop shell itself sets `OPENCODE_EXPERIMENTAL_FILEWATCHER` and `…_ICON_DISCOVERY` (`packages/desktop/src/main/lifecycle/environment.ts`) — notably _not_ the event-system flag.                             |
| Plugin: mcpx-session.ts                                                       | No                                                  | **Untouched.** It registers `experimental.chat.system.transform` and dynamic tools only; it never subscribes to bus events.                                                                                                  |
| Plugin: mcpx bridge (`plugin/mcpx/daemon.ts`)                                 | No                                                  | **Untouched.** Its `event` fields belong to mcpx's _own_ durable log schema, not opencode's bus.                                                                                                                             |
| Plugin: subtask2 (`@openspoon/subtask2` 0.3.9)                                | No                                                  | **Untouched.** Hooks are `tool.execute.before/after`, `experimental.chat.messages.transform`, `experimental.text.complete` — no `event` hook (checked in the installed dist).                                                |
| mcpx daemon                                                                   | No                                                  | **Untouched.** Separate service; doesn't consume opencode events.                                                                                                                                                            |
| Your automation driving the server API (fleet agents, this sandbox's tooling) | No — request/response only                          | **Untouched.** (And this sandbox's server runs `--pure`, so plugins don't even load here.)                                                                                                                                   |

### OpenChamber deserves a paragraph of its own

OpenChamber is your most sophisticated event consumer, and it is _already_ a 2.x
citizen — the flag is irrelevant to it, but the 2.x event system is its lifeline:

- Its web server keeps **one shared upstream SSE reader** on `/api/event`
  (`packages/web/server/lib/event-stream/global-hub.js`), fans out to browsers over
  WebSocket, and keeps its own bounded replay buffer (2,048 frames / 8 MiB) keyed by
  the server's `<instance>:<sequence>` ids, with `Last-Event-ID` resume and a
  `replayReset` → refetch fallback when a cursor can't be honored.
- `translate-v2.js` is the single place mapping 2.x wire events onto the v1
  vocabulary its server-side consumers (notifications, message queue, session
  activity, goal mode, permission auto-accept, Linear status) were written against.
  Load-bearing translations: busy/idle/retry synthesized from `session.execution.*`
  - `session.retry.scheduled` + `session.step.started`; each `session.step.ended`
    becomes an assistant `message.updated`, only the last carrying `finish: "stop"`.
- A **delta coalescer** merges the three v2 delta types before replay/fan-out
  (measured on a real stream: 3,402 delta frames / 1.3 MB → 483 frames / 197 KB,
  lossless). This matters for §3/§4: raw 2.x streams are ~97% delta frames.
- Browser clients receive raw wire payloads and translate in the UI
  (`packages/ui/src/lib/opencode/events.ts`).

---

## 3. Analytics — what's actually there

**No built-in analytics.** No metrics endpoint, no counters/timings API, nothing
Prometheus-shaped in the server or protocol packages (searched). `packages/stats` in
the fork is the opencode.ai public stats _website_ (a separate SST app), unrelated to
the event system. The core's `event-logger.ts` merely logs five config-family event
types (`agent/provider/model/command/config.updated`) at info level.

What exists is raw material — substantially better than 1.x, where the answer was
"nothing at all once the moment passed":

- **Usage is event data.** `session.usage.recorded` is a durable event carrying cost
  (USD) and full token usage; the schema comment calls it "the accounting source for
  session totals and stats". It's internal (never sent to clients); step/compaction
  end events repeat the figures publicly. Per-session, per-step cost/token accounting
  can be folded from the log exactly.
- **The log is queryable.** Per session over HTTP
  (`/api/experimental/session/:id/log`), and — with `events.persist` on — directly in
  SQLite: the `event` table has `(aggregate_id, seq, created, type, data)` with an
  index on `(aggregate_id, type, seq)`. Event rates, tool-call counts, step durations
  (started/ended pairs), failure counts, retry frequency: all computable offline with
  plain SQL.
- **Aggregation is the consumer's job, by design.** The intended mechanism is a
  projector (`bus.project(...)`) or an external subscriber folding the stream into
  whatever counters it wants. If you want a dashboard, you build a small projector —
  the system gives you a complete, ordered, typed feed to build it from, which the
  old bus never could (no history, no ordering, no IDs).

---

## 4. Bridge potential — building on top of it

This is where the new system genuinely earns its keep for your "advanced features"
idea. Things that were impossible or fragile on the 1.x bus:

- **Reliable external automation.** A bridge process subscribes to `/api/event`,
  persists its `Last-Event-ID` cursor, and on restart replays what it missed
  (bounded window) or gets an explicit `server.resync` instead of silently missing
  events. The 1.x pattern — reconnect and refetch everything, race the live stream —
  is gone.
- **Catch-up correctness.** The per-session log API's `log.synced` watermark tells a
  consumer "you now hold everything up to seq N" before it starts reacting — the
  difference between an automation that's right and one that's usually right.
- **Selective subscription.** In-process consumers subscribe by type
  (`bus.subscribe(SessionEvent.ToolCalled)` etc.); external ones filter a typed,
  schema-stable vocabulary instead of regexing an ad-hoc stream. You can take
  `session.tool.*` + `session.execution.*` and skip the delta firehose entirely.
- **Plugins as event citizens.** Plugin-defined `rpc.*` events ride the same stream,
  and external callers can invoke plugin RPC over HTTP — plugins become bridge
  endpoints, not just in-process hooks. Your fork's plan-mode plugin already shows
  the in-process pattern (`ctx.event.subscribe()` filtered to `session.created` /
  `session.agent.selected`, reacting by injecting synthetic prompts).
- **Multi-project routing.** Events carry `location` (directory + workspace); one
  bridge can serve many projects and route per directory, and session moves are
  delivered to both locations so nothing falls through a crack mid-move.
- **Sessions as actors.** Your fork's inter-session messaging
  (`messaging.message.created`, session inbox events) is itself built on this bus —
  it's the clearest proof of concept for event-driven cross-session automation.
- **New read models without producer changes.** Any projection — a dashboard, an
  audit trail, a cost ledger — is a fold over the same log. Producers never change.

Honest limits:

- The live `/api/event` replay buffer is small and per-process (1,024 frames /
  4 MB). Long offline gaps still end in `server.resync` + refetch. Cross-restart
  history needs `events.persist` enabled.
- The durable manifest currently covers session events (+ `worktree.resolved`);
  other aggregates aren't replayable logs yet.
- Delta volume is real: budget for coalescing/filtering in anything that stores or
  forwards the raw stream (OpenChamber's numbers: 97% of frames).

---

## 5. Bottom line

**Does enabling it affect you today? No — positively, negatively, or at all.**
On the 1.18.31 builds you run, the flag is parsed and ignored; on your fork's 2.x
builds it isn't read. None of your consumers (TUI, OpenChamber, desktop, plugins)
change behavior because of it. Keeping it in the config PRs is harmless; removing it
would be equally safe. It's documentary at this point — a signpost that the
transition it once gated has finished.

**What actually deserves your attention is the 2.x event vocabulary**, because
OpenChamber already depends on it and anything new you build should too:

- After your next real work session on a 2.x build, watch:
  1. **OpenChamber's status indicators** around aborts and provider retries — busy/
     idle/retry there is _synthesized_ from execution events (translate-v2.js), the
     most likely place for edge-case weirdness.
  2. **Anything of yours that ever parsed the 1.x stream** (`message.part.delta`,
     `{type, properties}` shape, `/global/event`) — it will not work against 2.x;
     the equivalents are `session.text.delta` / `session.tool.input.delta` with a
     `{id, type, data, location}` envelope on `/api/event`.
  3. **Whether `events.persist` is on** for the server you care about, the first
     time you want history or analytics from it — it's off in the default local
     profile.
  4. **Delta handling** in any new consumer: filter or coalesce from day one.

And the opportunity, plainly: if you build those advanced bridge features, build
them on the 2.x stream + per-session log API. It's the first version of OpenCode
where an external system can watch, react, restart, and catch up without losing
events — that part is real, and it's yours already.
