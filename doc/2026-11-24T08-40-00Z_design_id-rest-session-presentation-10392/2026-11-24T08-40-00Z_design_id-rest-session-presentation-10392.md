# ID binary REST session and presentation model — invocation, sessions, resume, app loader

Status: **PROPOSED** (design pass 3 of the fleet-surface design mill, parent
#10389). Tracking: developing-today/code#10392. Seam owner: pass 2 (#10391),
record `doc/2026-11-24T05-55-51Z_design_fleet-iroh-id-transport-10391/`.
Lane oc-code-05, queue task 22718.

This record decides the ID binary's **serving** side over REST. It does not
re-decide the pass-2 transport choices (iroh endpoint, key lifecycle, mode
rule, postcard handling). Where pass 2 fixes a contract, this record refers to
it and does not restate it.

## 1. Problem

The ID binary must serve every ID subcommand over REST with full argument
parity: a REST call carries the same argv, environment, and input a shell
invocation would. Output streams back as events. Some apps close at once (a
plain REST response); others stay alive until they exit or an idle timeout
closes them. Clients must be able to detach, reattach, and replay what they
missed. Browser and non-browser clients need an app loader that declares what
an app needs before it runs.

## 2. Goals and non-goals

Goals:

- One invocation shape for every subcommand, with argv parity.
- Explicit separation of finite piped input from long-lived interactive input.
- Session ids that exist before any output, and resumable attach.
- Replay by sequence number, bounded in size and time.
- A capability declaration an app loader can check before starting an app.
- Presentation assets and bundles served by the ID binary itself.

Non-goals:

- Changing the pass-2 envelope, ALPN, or key handling.
- Authorization policy beyond the pass-2 §4.5 rule (localhost by default;
  non-local needs the session credential). Per-user authorization is out of
  scope here.
- Exact CLI flag spellings. Pass 1 owns syntax; this record owns semantics.

## 3. Alternatives considered

| Alternative | Benefits | Costs / risks | Decision |
|---|---|---|---|
| Input and output both over SSE | One transport. | SSE is server-to-client only; input needs a second channel anyway. | Rejected for input. SSE is the output default. |
| WebSocket for everything | One bidirectional channel; matches pass 2. | Some clients cannot hold a socket; plain REST apps need no socket at all. | Accepted as an option, not the only path. |
| Separate protocol per presentation type | Tailored to each output. | Breaks argv parity and the replay model. | Rejected. One event model; presentation is a payload kind. |
| Unbounded session log | Simple replay. | Unbounded memory and disk; violates "nothing kept indefinitely". | Rejected. Ring log with byte cap and TTL (§4.4). |

## 4. Chosen approach

### 4.1 Invocation shape

A run is a single request document:

```
POST /v1/runs
{
  "app": "<app id>",            // optional; omitted for a plain subcommand
  "argv": ["..."],              // full argv after the binary name
  "env": { "NAME": "value" },   // allowlisted names only
  "stdin": { "mode": "none" | "body" | "session" },
  "client": { "kind": "web" | "wasi" | "cli" | "other", "features": ["..."] },
  "capabilities": ["fs.read", "device.audio", ...]   // what the client can grant
}
```

- `stdin.mode = none`: no input; stdin is closed at start.
- `stdin.mode = body`: input is the request body, finite, ends at end of body.
  This is piped input. It is read to EOF before the run starts reading.
- `stdin.mode = session`: input is open-ended. It is sent as numbered input
  frames to the session's input endpoint (§4.2). This is interactive input,
  and the run sees EOF only when the client closes input.

The distinction is explicit in the request, never inferred from whether bytes
happen to arrive. An app that needs a terminal declares `session`.

### 4.2 Output and input streaming

- **Output** streams as events. Default transport is SSE
  (`text/event-stream`). Each event carries `id: <seq>`, `event: out | err |
  exit | app`, and `data:` as a JSON object.
- **Input** for `session` mode is a numbered frame sequence:
  `POST /v1/sessions/{id}/input` with `{ seq, bytes }`. The server accepts
  frames in order, rejects gaps and duplicates with a distinct error, and
  treats a repeated `seq` with identical bytes as idempotent.
- **Bidirectional clients** may use a WebSocket upgrade on the session
  instead. The socket carries the same event and input objects. It does not
  introduce new message types.
- **Pass-2 envelope.** Any event that crosses to a fleet socket is wrapped in
  the pass-2 §4.5 envelope `{ v, seq, kind, alpn, payload, digest? }`
  unchanged. This record does not add, rename, or fork fields.

### 4.3 Sessions

- `POST /v1/sessions` returns `{ id }` immediately, before any output. The
  run may be attached to it now or later.
- `POST /v1/runs` with `stdin.mode != none` and no session id creates one
  implicitly and returns its id in the first event.
- Attach: `GET /v1/sessions/{id}/events` (SSE) or the WebSocket upgrade, with
  a `since` cursor.

### 4.4 Resume, replay, and retention

Each session keeps an ordered event log with sequence numbers. Clients read
it with the replay modes from the design brief:

| Mode | Query | Returns |
|---|---|---|
| since last time | `?since=<seq>` | events with `seq > since` |
| after | `?after=<seq>` | same as since, exclusive, named for clarity in apps |
| before and after | `?before=<seq>&after=<seq>` | a window |
| all | `?all=1` | the whole retained log |

Retention is bounded in two ways: a byte cap per session (default 16 MiB,
configurable) and an idle TTL (default 1 hour locally, configurable up to 24
hours). When either limit is hit, the oldest events are dropped and a
`app: truncated` event records the first retained `seq`. A client asking for
events below that seq gets the truncation marker, never silent gaps.

Replay is a pure function of the log: for any split point `k`, the
concatenation of `since=0..k` and `since=k..end` equals the full log. This is
the invariant the replay test checks (§7).

Close-immediately apps are runs with `stdin.mode = none` and no session. The
response is the final event stream in one HTTP response, equivalent to a plain
REST call.

### 4.5 App loader and capability declaration

An app ships a manifest:

```
{
  "app": "<id>", "version": "<semver>",
  "client_side": ["assets/app.js", "assets/app.css"],
  "capabilities": { "required": [...], "optional": [...] }
}
```

Loader rules:

1. The client announces `client.kind` and `client.features`.
2. The loader refuses a run whose `required` capabilities the client does not
   grant. It never starts the app with a missing required capability.
3. Optional capabilities are granted only when present in the request.
4. Non-web clients (WASI-style) declare filesystem and device needs as
   capabilities. Those needs are checked, not assumed.

A web frontend sets up an app through `GET /v1/apps/{app}/manifest`, then
starts a session. It gathers any `client_side` data first.

### 4.6 Presentation layers

- **HTML presentation.** An app may return DOM/JS/CSS/HTML as assets. Assets
  are served by the ID binary under `/v1/apps/{app}/assets/{path}` with
  content-type from the manifest or file extension and a content hash in the
  URL. Static and dynamically rendered assets use the same path.
- **Bundles.** The whole web version is available as one `.tar.zst` or `.zip`
  (`GET /v1/apps/{app}/bundle`), or as a single HTML file with assets inlined
  (`GET /v1/apps/{app}/bundle.html`).
- **Multi-page apps.** Default is one request per page. The single-file bundle
  is the alternative, where client JS switches pages locally. The choice is
  per app and recorded in the manifest.

### 4.7 Auth and binding

Same rule as pass 2 §4.5: the server binds localhost by default. A non-local
bind requires the session credential. No anonymous non-local serving. Run
requests on localhost do not require a credential.

## 5. Decisions and defaults

| # | Decision | Default taken here | Who may change it |
|---|---|---|---|
| D1 | Input transport for `session` mode | Numbered POST frames; WebSocket optional | Pass 1 if it needs a different framing |
| D2 | Default retention | 16 MiB per session, 1 h idle TTL | Pass 3 with measurements |
| D3 | Replay modes | Four modes in §4.4 | Pass 3 |
| D4 | Pass-2 envelope | Unchanged | Only via a joint `fleet-id-cbor-v2` bump |
| D5 | Flag and env spellings | Names as used here | Pass 1 wins on syntax |

## 6. Build slices (dependency order)

1. **Run request and argv parity.** `POST /v1/runs` with `stdin.mode = none`
   and `body`, for every CLI subcommand. Done check: each subcommand's REST
   output equals its CLI output for the same argv.
2. **SSE output and close-immediately runs.** Event ids, `exit` event.
   Depends on 1.
3. **Sessions, input frames, replay.** Depends on 2. Replay property test
   (§7).
4. **App manifest, loader refusal, client announcement.** Depends on 1.
5. **Presentation assets, bundles.** Depends on 4.
6. **Seam conformance with pass 2.** The §4.2 envelope round-trips through the
   fleet socket and this REST side byte-identical. Depends on 3 and pass-2
   slice 5.

## 7. Verification plan

- **Argv parity (oracle: the CLI itself).** For each subcommand, run the CLI
  and the REST run with the same argv and input; compare stdout, stderr, exit
  code.
- **Replay invariant (property test).** Oracle: the full log. For random logs
  and split points `k`, `since=0..k` followed by `since=k..end` equals the
  full log. Also: truncation marker present whenever the cursor is below the
  retained floor.
- **Input ordering (model-based).** Oracle: a model that accepts frames in
  order and rejects gaps and conflicting duplicates. Random interleavings of
  input posts and reconnects must match the model.
- **Loader refusal (example test).** A run missing a required capability is
  refused with the capability named; it never starts.
- **Envelope round-trip (golden).** Oracle: the pass-2 §4.5 framing bytes.

## 8. Open questions

1. Whether `session` input should also accept a WebSocket-only upgrade for
   clients that cannot POST frames. Default: not in slice 3; add when a client
   needs it.
2. Whether bundle hashes should be published in the manifest. Default: yes,
   in `client_side` entries.

## 9. Record and acceptance

- Acceptance for this design pass: the invocation, input, session, replay,
  loader, presentation, and auth sections each state one decision and one
  default; the pass-2 seam is referenced, not restated; every verification
  item names its oracle.
- Build issues are filed from §6 once this record is merged.
