# F12.5 `http.fetch` provider — design for the third slice of #1062

Issue: developing-today/code#1062 (F12.5 first new providers). The
2026-11-21 decision on that issue fixes the order — `kv` first
(landed, PR #12489), then `blob` (landed, PR #12529);
`http.fetch` and `notify` after, each off by default with a
refusal test — and the exit shape: each provider has bound,
refusal, and replay tests, and an unlisted host is refused. The
F12.5 text adds the provider's own shape: "admin allowlist, size
and rate bounds, no redirects off the list, **off by default**".
This document covers only the `http.fetch` slice, and follows the
`kv` design (`doc/2026-11-22_design_id1062-kv-provider.md`) and
the `blob` design (`doc/2026-11-22_design_id1062-blob-provider.md`)
wherever the providers share a question.

## Shape

One request capability:

- `http.fetch` — args `{"url"}`; result
  `{"url", "status", "body", "body_bytes"}` where `url` is the
  URL that produced the response (after any followed redirects),
  `status` is the HTTP status code, `body` is the response body
  as a base64 string (the `blob` wire spelling for bytes), and
  `body_bytes` is its length before encoding.

GET only: a fetch names no method and sends no request body, so
there is nothing to bound on the request side beyond the URL
itself, and no way for a program to spend the host's authority on
a side-effecting verb. POST and other methods are follow-up work
if a use appears. No new Wasm import: the request travels in the
existing `wants` document, the result in the existing result
events. Each use passes the standard grant/ceiling/scope decision
in the actor's `execute`, which writes the audit entry per use,
exactly as every other provider.

## Off by default, and the admin allowlist

`http.fetch` is the first outward-facing provider: its effect
leaves the node. Two independent admin gates therefore stand in
front of it, and both default to closed.

- **Ceiling.** It registers with
  `DefaultCeiling::Excluded` — the registry's mechanism for a
  sensitive provider — so it is in no default ceiling: neither
  the server's `server_caps` (which defaults to
  `default_ceiling_names()`) nor a fresh world's `caps_ceiling`
  names it. A use before an operator enables it is refused
  `outside_ceiling` by the ordinary ledger decision, before any
  provider code runs. Enabling it is the operator's explicit act:
  name `http.fetch` in the server caps _and_ the world's ceiling,
  plus the usual grant. (It is not a sensor cap: the sensor set
  is the `loc.*`/microphone family, whose per-use consent seam
  does not apply to a server-configured outbound fetch.)
- **Allowlist.** `WorldLimits::http_fetch_hosts` is the admin's
  set of hosts a world may fetch from — runtime configuration in
  the `server_caps` mould, never journaled, empty by default.
  The executor refuses a URL whose host is not in the set before
  any connection is made. Matching is exact on the normalized
  host (lowercase, one trailing dot stripped): listing
  `example.com` does not admit `api.example.com`, and there are
  no wildcards — an admin who wants a subdomain lists it.

Either gate alone keeps the provider closed; a fetch needs both
open, for that host.

## Bounds (constants in `id-caps` `world_caps`)

- URL: at most `MAX_HTTP_FETCH_URL_BYTES` (2,048) bytes — the
  de-facto interoperable URL length, and a quarter of the
  `MAX_ARGS_BYTES` (8 KiB) document cap, so a maximal URL leaves
  the args budget its framing. Only `http` and `https` URLs with
  a host and no userinfo pass `validate_http_fetch_url`, which
  also returns the normalized host for the allowlist check:
  credentials in a URL would leak into logs and audit digests,
  and other schemes are not fetches.
- Response body: at most `MAX_HTTP_FETCH_RESPONSE_BYTES`
  (32 KiB). The result travels in a result event capped by
  `MAX_EVENT_BYTES` (64 KiB); the body rides base64 (4/3
  expansion), so 32 KiB of body is 43,691 characters — inside
  the event cap alongside the `url` (≤ 2,048 bytes), the status
  and the envelope, with slack. The next power of two, 48 KiB,
  encodes to 65,536 characters — the entire event budget before
  any other field — so 32 KiB is the largest power of two that
  fits. A longer body is refused (`response_too_large`), never
  silently truncated: the executor reads at most the cap plus
  one byte, so an unbounded stream costs bounded memory.
- Redirects: at most `MAX_HTTP_FETCH_REDIRECTS` (3) hops,
  followed manually — the client never follows one on its own.
  Every hop is resolved against the current URL and put through
  the same validation and allowlist check as a fresh request; a
  hop off the list refuses the fetch (`host_not_allowed`), and a
  fourth redirect refuses it (`too_many_redirects`). That is the
  F12.5 "no redirects off the list" rule enforced at the only
  place it can be: before the connection to the new host.
- Timeout: `MAX_HTTP_FETCH_TIMEOUT_MS` (10,000) for the whole
  fetch, all hops together — a per-request budget the executor
  spends down across hops. The client-provider deadline
  precedent is 30 s; a fetch is a single round trip, and because
  `execute` is synchronous (below) the timeout also bounds how
  long one fetch can occupy the actor.
- Rate: at most `MAX_HTTP_FETCH_BURST` (10) fetch attempts per
  world per rolling `HTTP_FETCH_RATE_WINDOW_MS` (60,000),
  counted on the world clock. The chat precedent is a burst of
  50 per 10 s for local lines; an outbound fetch is costlier —
  network traffic under the host's name, and up to the timeout
  of actor time each — so the bound is a burst of 10 per minute:
  a program may poll one endpoint every six seconds, and its
  worst-case actor occupancy from fetches is ten timeouts per
  minute. Attempts are counted when they are made, so refused
  and failed fetches spend the budget too — the bound is on
  traffic attempted, not on responses received. Redirect hops
  are part of their fetch and spend nothing extra.

Bounds are enforced in `WorldCore::check_http_fetch` at request
time (the program sees `bad_args` with a reason: malformed URL,
unlisted host, or rate), mirroring `check_kv_set` /
`check_blob_put`. There is no replay-side enforcement, because
there is nothing to replay (below).

## Durability: a fetch journals nothing

`kv` and `blob` journal because they _are_ world state: replay
must rebuild the store from the entries alone, and a response
body is not state. A fetch is an effect against the live
network — its result depends on the world outside the journal,
so journaling it would not make replay a pure function of the
entries; it would only pretend to. The executor therefore
persists no fetch state: no new journal entry kinds, **no
journal version bump** (the version trail stays at 14,
`JOURNAL_VERSION_BLOB`; `entry_kind_version` and the
`write_lease` version-trail test are untouched), and no
checkpoint field. The entries a fetch ask does produce are the
ones every request produces, reads included: the input's
`Committed` event, the result's `System` event (the delivery
replay feeds the program, not an effect replay applies), and —
when grant-on-use mints the grant — the pre-existing
`CapsGranted`, which journals the _authority_, as it does for
every provider. No entry records the URL, the response, or any
fetch effect.

Two existing mechanisms cover the replay questions a
non-idempotent effect raises, and the design leans on both
rather than adding a third:

- **Restore purity.** `WorldCore::restore` applies no fetch;
  a journal written between fetches restores exactly the state
  its state entries describe. The replay test proves this
  shape: a `kv` write interleaved with fetches restores
  identically, and the fetch ask is shown to add no state entry
  — only the committed input and result-delivery entries every
  request adds.
- **At-most-once across a reopen.** A request still listed in
  `wants` at a restored open is refused (`restarted`), not
  re-executed — `after_change`'s stale-wants rule, whose comment
  already names `http.fetch` among the non-idempotent providers
  it protects. A program that still wants the data re-issues the
  request as new intent.

The rate window is likewise ephemeral: attempt timestamps live
in `WorldCore` memory only, beside the chat-abuse state, and a
restart resets the window. That is the honest scope of the
bound — it shapes a live program's traffic, it is not a durable
quota.

## Execution

`execute` is synchronous and the actor's other providers are
in-memory or journal writes, so the fetch is the first provider
whose work is real network I/O. It runs on a dedicated OS thread
per fetch, joined before the result is delivered: the blocking
client is only ever constructed and used on that thread (the
`reqwest` blocking API panics inside an async runtime), the
actor's runtime is never asked to block, and the pattern needs
no pending/completion machinery of the client-provider broker
kind — the request's result is still delivered by the ordinary
synchronous path. One thread per fetch is cheap next to a
network round trip, and the rate bound caps how many can exist.

The client is a process-wide `reqwest` blocking client
(new direct dependency of `id-worlds`: reqwest 0.13, already in
the workspace lock as iroh's dependency; features `blocking` +
`rustls`, default features off so no system proxy is consulted —
a fetch goes to the allowlisted host, not through whatever
proxy the environment names). Automatic redirects are disabled
(the executor follows them manually, above), and the response
body is read incrementally up to the cap. URL parsing and
validation live in `id-caps` (`validate_http_fetch_url`, over a
new `url` dependency — 2.5, likewise already in the lock), so
the registry crate owns the URL form exactly as it owns the
`kv` key and `blob` hash forms; the executor re-validates every
redirect target through the same function.

One stall is accepted and bounded: because `execute` is
synchronous, a fetch occupies the world's actor for up to the
10 s timeout. The alternatives — broker-style pending
completions for a host provider — are a larger mechanism than
this slice needs; if the stall ever bites, the rate bound says
how often, and moving the fetch to a completion channel is
follow-up work that changes no wire shape.

## Registry

`http.fetch` registers in the F12.4 provider registry as a
version-1 host request, `RequiredScope::Join`,
`DefaultCeiling::Excluded` — the outward-facing counterpart to
`kv.*`/`blob.*`'s `Included`. The `url` argument is declared
with the schema's `String` kind; the executor validates its
form. Its manifest bounds name the args cap, the URL cap as the
identifier bound, and the response cap.

## Tests (the #1062 exit shape, for this provider)

- Bound (caps): `validate_http_fetch_url` cases — scheme,
  userinfo, missing host, oversized URL, host normalization.
  Bound (core): `http_fetch_checks_refuse_each_violation_with_a_reason`
  — malformed URL, unlisted host, allowlisted host admitted, and
  the rate bound refusing the eleventh attempt inside the window
  and admitting again after it (manual clock).
- Refusal: with default limits a granted `http.fetch` is refused
  `outside_ceiling` (off by default); on an enabled world an
  ungranted fetch is refused and counted; **an unlisted host is
  refused and the hermetic server sees no request** (the
  decision's named test); a redirect to an unlisted host is
  refused the same way.
- Round trip: an enabled world fetches an allowlisted hermetic
  endpoint (a loopback `TcpListener` in the test — no live
  internet) and gets its status and body back; an on-list
  redirect is followed; an over-cap body is refused.
- Replay: fetches journal no state — a fetch interleaved with
  `kv` writes adds no state entry to the recorded journal (only
  the input/result entries every request adds), and restoring
  from it rebuilds the same state.
