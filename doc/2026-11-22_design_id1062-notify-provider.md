# F12.5 `notify` provider — design for the fourth slice of #1062

Issue: developing-today/code#1062 (F12.5 first new providers). The
2026-11-21 decision on that issue fixes the order — `kv` first
(landed, PR #12489), then `blob` (landed, PR #12529), then
`http.fetch` (landed, PR #12559); `notify` last — and the exit
shape: each provider has bound, refusal, and replay tests. The
F12.5 text adds the provider's own shape: "`notify` — user-visible
outbound notifications, **rate limited** (this also covers the
roadmap's 'outbound notifications' idea)". This document covers
only the `notify` slice, and follows the `kv` design
(`doc/2026-11-22_design_id1062-kv-provider.md`), the `blob`
design (`doc/2026-11-22_design_id1062-blob-provider.md`) and the
`http.fetch` design
(`doc/2026-11-22_design_id1062-http-fetch-provider.md`) wherever
the providers share a question.

## What a notification is in this codebase

The F12.5 text names the effect — a user-visible outbound
notification — but no delivery channel, so the channel is read
off the code. The world runtime has exactly one user-visible
broadcast surface: the committed world event stream. Committed
events are sequenced, journaled, broadcast to participant
sessions, retained for reconnect catch-up, and exported through
the admin history. That is the surface every user of a world
actually sees, and it is the one `chat.say` already writes to.
There is no push, webhook, or mail channel anywhere in the world
runtime to deliver through instead (the PWA push design in
`doc/2026-11-01T03-54-00Z_design_pwa_push_notifications.md` is a
different product surface, the PM aggregator, and no world code
reaches it).

A notification is therefore a new committed event kind,
`WorldEventKind::Notification { text }`, committed by the host
with `participant_id` 0 on the program's behalf — the
`prepare_system_chat` mould — and broadcast like any committed
event. It is deliberately *not* a chat line wearing a flag:

- it does not enter chat history (`chat.history` reads `Chat`
  events; a notification is not conversation and must not
  scroll back as if it were),
- it reaches every joined participant: the join projection
  (`project_event_for`) returns it as-is, not gated on the
  chat-read scope that hides `Chat` from join-only participants
  — a notification is addressed to the world's users, not to
  its chat readers,
- clients can render it as what it is: the session wire
  projection gains a `notification` frame kind beside
  `chat_posted`, instead of a chat line a client must guess is
  an alert.

The program itself does not receive the event back: the sandbox
and native program runtimes ignore `Notification` in `update`,
exactly as they ignore `Chat` — the program originated it; the
users are its audience. Version 1 carries a single `text`: the
chat precedent is one bounded line, and a title or level is an
additive follow-up if a use appears, not a v1 guess.

## Shape

One request capability:

- `notify` — args `{"text"}`; result `{"sequence"}`, the
  sequence of the committed notification event (the `chat.say`
  result shape).

No new Wasm import: the request travels in the existing `wants`
document, the result in the existing result events. Each use
passes the standard grant/ceiling/scope decision in the actor's
`execute`, which writes the audit entry per use, exactly as
every other provider.

## Off by default

`notify` is outward-facing like `http.fetch`: its effect leaves
the program and lands on users who did not ask it anything. It
therefore registers with `DefaultCeiling::Excluded` — in no
default ceiling, neither the server's `server_caps` nor a fresh
world's `caps_ceiling` — so a use before an operator enables it
is refused `outside_ceiling` by the ordinary ledger decision,
before any provider code runs. Enabling it is the operator's
explicit act: name `notify` in the server caps *and* the world's
ceiling, plus the usual grant.

There is no second, allowlist-style gate: `http.fetch` needs one
because its target varies per request and is the admin's to
bound. A notification's target is fixed — the world's own
users — so there is nothing per-request for an admin list to
admit. The ceiling is the gate, and the rate bound below is the
volume control.

## Bounds (constants in `id-caps` `world_caps`)

- Text: at most the world's `chat_bytes` limit (2,048 bytes by
  default) — the bound the host already applies to its own
  user-visible lines (`prepare_system_chat`), declared in the
  manifest exactly as `chat.say` declares it
  (`ProviderBound::WorldLimit("chat_bytes")` beside the
  `MAX_ARGS_BYTES` args cap). A notification is a user-visible
  line of the same kind, so it takes the same bound rather than
  a new constant with a new derivation; an operator who tunes
  the line bound tunes both. The text is sanitized with the
  chat sanitizer (`sanitize_chat`: control characters stripped)
  and a text that is empty after sanitizing is refused — a
  notification with nothing to say is a bug, not an alert.
- Rate: at most `MAX_NOTIFY_BURST` (5) notifications per world
  per rolling `NOTIFY_RATE_WINDOW_MS` (60,000), counted on the
  world clock — the `http.fetch` window machinery, halved.
  A fetch is invisible to users; a notification interrupts
  every one of them and occupies a slot in the retained event
  window, so it is costlier per use and takes the smaller
  burst: five a minute is a sustained one per twelve seconds,
  which covers event-driven alerts (a match ending, a turn
  prompt, an operator notice) with room, and a program that
  needs a faster channel to users already has `chat.say` under
  its own abuse limits. Attempts are counted when they are
  made — once the check passes, before the event is committed
  — so a notification whose commit then fails has still spent
  its interruption budget, as a failed fetch spends its
  traffic budget.

Bounds are enforced in `WorldCore::check_notify` at request
time (the program sees `bad_args` with a reason: oversized,
empty, or rate), mirroring `check_http_fetch`. The prepared
event is built by `WorldCore::prepare_notification`, beside
`prepare_system_chat`.

## Durability: the notification is a committed event

`kv` and `blob` journal because they are world state;
`http.fetch` journals nothing because its result is not state
and journaling it would not make replay a pure function of the
entries. A notification sits between those poles, and the
`chat.say` precedent decides it: a notification, once sent, is
a fact about the world's history that users may already have
seen, so it is journaled *before* its result is delivered, as
the ordinary `JournalEntry::Committed` carrying the
`Notification` event — no new journal entry kind, and replay
applies nothing for it beyond rebuilding the event log it is
part of, exactly as for a chat line. Restoring a journal
rebuilds the notification in the retained window at its
sequence; the restore author check classifies it with the
other host-authored kinds (`participant_id` 0, no participant
scope authored it — the authority was the cap decision, whose
record is the audit entry).

Two consequences follow, both precedented:

- **Journal version 15.** A new event kind is a thing an older
  binary cannot parse: a version-14 binary reading a journal
  whose committed entries carry a `Notification` event would
  fail mid-replay. That is precisely the `EntityInput`
  precedent, which took `JOURNAL_VERSION_REGION_HANDOFF` for
  the same reason. `JOURNAL_VERSION_NOTIFY` is therefore 15:
  `entry_kind_version` maps a `Committed`/`System` entry
  carrying a `Notification` event to it, a checkpoint whose
  retained events or admin history carry one requires it (the
  bundle-provenance rule's both-places check, not only the
  retained-events check), and `MAX_JOURNAL_VERSION` moves to
  it. A journal that never carries a notification is never
  migrated, per the #227 contract.
- **At-most-once across a reopen.** A request still listed in
  `wants` at a restored open is refused (`restarted`), not
  re-executed — `after_change`'s stale-wants rule, whose
  comment already names `notify` among the non-idempotent
  providers it protects. A notification is never delivered
  twice because the host restarted.

The rate window is ephemeral, as for `http.fetch`: attempt
timestamps live in `WorldCore` memory only, and a restart
resets the window. That is the honest scope of the bound — it
shapes a live program's interruptions, it is not a durable
quota.

There is no replay-side bound re-check, and that is the chat
rule, not an omission: the text bound is a request-time bound
on what the host will commit, as `chat_bytes` is for chat
lines, and the journal's own entry-size cap bounds what a
journal can carry. Replay rebuilds history verbatim; it does
not re-derive it. (The `kv`/`blob` replay re-checks exist
because replay *rebuilds state* from those entries; an event
log is not rebuilt state.)

## Registry

`notify` registers in the F12.4 provider registry as a
version-1 host request, `RequiredScope::Join`,
`DefaultCeiling::Excluded` — the outward-facing mould of
`http.fetch`, against `kv.*`/`blob.*`'s `Included`. The `text`
argument is declared with the schema's `String` kind (the
`chat.say` argument); the executor validates its form through
the core check. Its manifest bounds are the args cap and the
`chat_bytes` world limit, as `chat.say`'s are.

## Tests (the #1062 exit shape, for this provider)

- Bound (core): `notify_checks_refuse_each_violation_with_a_reason`
  — oversized text, text empty after sanitizing, a well-formed
  text admitted, and the rate bound refusing the sixth attempt
  inside the window and admitting again after it (manual
  clock). Bound (caps): `notify_is_registered_but_off_by_default`.
- Refusal: with default ceilings a granted `notify` is refused
  `outside_ceiling` (off by default) and commits no event; on
  an enabled world an ungranted notify is refused `cap_denied`
  and counted; past the rate bound an enabled, granted notify
  is refused `bad_args` naming the rate.
- Round trip: an enabled world notifies and the committed
  event — kind `Notification`, sanitized text, the result's
  sequence — is in the world's event log and on its broadcast
  channel.
- Replay: a notify journals exactly one entry of its own (the
  `Committed` notification, beside the input and result
  entries every request adds); restoring from the journal
  rebuilds the event log with the notification intact; the
  entry requires journal version 15
  (`notify_events_require_journal_version_15`, with the
  max-version pin in
  `write_lease_entries_require_journal_version_7` moved to 15
  and its version trail extended).
