# Revision conflicts and spec-internal contradictions

WP6 output 1b. Everything where a message or behaviour legal in one revision is forbidden or required differently in another, plus places where one revision contradicts itself (page vs page, prose vs schema, changelog vs page). Per-area sections are the sweeps' own lists, lightly re-headed; ids refer to `requirements.md`.

## How mcpx chooses (#307)

Where a conflict below is not settled by the peer's stated revision, mcpx is
meant to decide it by `spec.precedence` (revisions in the order their rules
win; default newest first, `--mcp-spec <rev>` moves one to the front) and
`spec.lenient` (revisions not held strictly; default none), through
`internal/spec`. See [`../protocol.md`](../protocol.md) §1.1.

Decided by those settings today:

- **Progress `message` to a 2024-11-05 client** (2024-11-05's schema has no
  `message`; every later revision does). Sent by default; stripped only when
  2024-11-05 is first and strict. `internal/mcpserver/relay.go`.
- **Headline 2, server-initiated requests from a 2026-07-28 server.** Dropped
  while 2026-07-28 is strict (default); answered when it is in `spec.lenient`.
  `internal/mcpclient/client.go`.

Every headline item now carries a disposition in the index below
(Phase 1 of #2209): **peer-settled** (the peer's stated revision decides it,
in code), **precedence-governed** (decided by the settings above), a
**recorded reading** (mcpx follows one named reading of contradictory spec
texts), or **undisposed** (an open question is named instead of a silent
choice). Per-area items in sections A-E are dispositioned in place (Phase 1 of
#2209, complete): every item carries a disposition line of one of the
four kinds above, and undisposed items name the specific open question
instead of a silent choice.

## Disposition index — headline items

1. **Recorded reading** — on stdio send the cancel (MUST) and, for a graceful end, the result; on HTTP answer the listen request and close the stream. Reading recorded in the item and in the comment block at internal/mcpserver/listen.go.
2. **Precedence-governed** — `spec.Current().Governs("2026-07-28")` at internal/mcpclient/client.go (server-initiated requests dropped while 2026-07-28 is strict; MRTR carriage in section E). Also in the "Decided" list above.
3. **Peer-settled** — disconnect split by era at internal/mcpserver/server.go (disconnect SHOULD NOT cancel on legacy, MUST in 2026).
4. **Recorded reading** — the legacy MUST governs legacy `initialize`: `negotiate()` counter-offers a supported version; `-32022` applies to modern `_meta`-versioned requests only. See A/C5.
5. **Undisposed** — open question (#1481 decision): the client choke keeps `-32602` (internal/mcpclient/schemacheck.go, internal/daemon/callerr.go); should the serve seam era-split `isError` for >=2025-11-25 hosts while /v1 and the client API keep `-32602`?
6. **Peer-settled** — era-split codes in code: legacy `-32002` (internal/mcpserver/server.go, `codeResourceNotFoundLegacy`), 2026 `-32602`.
7. **Undisposed** — open question: `downgrade()` (internal/mcpserver/revisions.go) deletes `structuredContent` only for pre-2025-06-18; a non-object value to a 2025-06-18..2025-11-25 (object-only) host passes unchanged. Render into content, wrap, or error? Check the conformance rows first.
8. **Peer-settled** — `nextPage` (internal/mcpclient/client.go): a missing cursor ends results in every revision; in 2026 an empty string is a valid cursor, not the end.
9. **Undisposed** — open question: the logging gate. The client relay gates on a requested level (internal/mcpclient/relay.go, `moreVerbose`); if no path relays upstream logs to a host that requested no level, this item is moot in the relay direction and should be closed saying so, with the site.
10. **Peer-settled** — legacy session-404 re-initialisation is era-keyed in internal/mcpclient's legacy session recovery (internal/mcpclient/client.go:1138-1145): a session-carrying 404 re-initialises on the legacy path, a modern-era 404 surfaces; on the server side an unknown method on the modern path is answered 404 with `-32601` (internal/mcpserver/server.go:879). See section B item C7.
11. **Peer-settled** — `batchRefused` (internal/mcpserver/transport.go): batches are refused outside 2025-03-26, the one revision that mandates receiving them.
12. **Recorded reading** — the schema and caching page govern over the changelog's list: `server/discover` results are `CacheableResult`s. Implementation gap: the discover result (internal/mcpserver/server.go) emits `supportedVersions` but not yet `ttlMs`/`cacheScope`.
13. **Undisposed** — both halves stay open, with the actual behaviour now recorded for one: cancelled-task finality (core vs extension) has no verified site (section C item C11), and the missing-required-task code (`-32601` vs `-32600`) is not implemented either way — a non-augmented call executes ordinarily (section C item C10, internal/mcpserver/tasks.go:237-245).

## Headline items

1. **Subscription teardown, 2026-07-28 (three sources disagree).** `basic/patterns/cancellation`: a server MUST send `notifications/cancelled` referencing the `subscriptions/listen` id when it tears the stream down, and MUST NOT send `notifications/cancelled` for any other purpose. `basic/patterns/subscriptions`: the server SHOULD send a successful `subscriptions/listen` response to signal a graceful end, and never mentions a cancel. `schema.ts` `CancelledNotification`: *on stdio* the server sends it, solely to end a listen stream; the Streamable HTTP page says `notifications/cancelled` is used only on stdio. Reading that satisfies all three: on stdio, send the cancel (MUST) and, for a graceful end, the result; on HTTP, answer the listen request and close the stream. See C1 in section C.
2. **Server-initiated requests.** Legal on every legacy revision (sampling, roots, elicitation, ping); forbidden in 2026-07-28, where they travel as MRTR `inputRequests` in a result.
3. **Disconnect as cancellation (Streamable HTTP).** 2025-03-26..2025-11-25: a disconnect SHOULD NOT be treated as cancellation. 2026-07-28: MUST be.
4. **Legacy `initialize` with an unsupported version.** Every legacy revision: the server MUST answer with a version it supports. `-32022` exists only in 2026-07-28 and applies to `_meta`-versioned requests.
5. **Tool input validation errors.** Protocol error up to 2025-06-18; `isError` tool result from 2025-11-25.
6. **Resource not found.** `-32002` (SHOULD) up to 2025-11-25; `-32602` (MUST) in 2026-07-28.
7. **`structuredContent` / `outputSchema`.** Object-only in 2025-06-18..2025-11-25; any JSON value / any 2020-12 schema in 2026-07-28.
8. **Empty-string cursor.** Unspecified before; in 2026-07-28 MUST NOT be treated as end of results.
9. **Logging without a requested level.** MAY send before 2026-07-28; MUST NOT after. `logging/setLevel` is removed in 2026-07-28 while logging is only deprecated.
10. **HTTP 404.** Session ended (client MUST re-initialize) in 2025-03-26..2025-11-25; method not found in 2026-07-28.
11. **Batches.** Receiving JSON-RPC batches MUST be supported in 2025-03-26 only; removed in 2025-06-18.
12. **`server/discover` cache hints.** The 2026-07-28 changelog's list of results needing `ttlMs`/`cacheScope` omits `server/discover`; the caching page and the schema require them on `DiscoverResult`.
13. **Tasks.** 2025-11-25 core: a cancelled task stays `cancelled`; extension: may still end in another terminal status. Missing-required-task error code: `-32601` on the tool-level rule vs `-32600` in the error list. The changelog calls extension tasks unsolicited; the extension page forbids returning a task to a client that did not declare support.

## A. Lifecycle, versioning, messages, errors, _meta, ping

#### Across revisions (legal in one, forbidden or different in another)

**C1. JSON-RPC batches: silent → MUST receive → removed.**
- **Disposition:** Peer-settled — batches are answered only for a 2025-03-26 peer and refused for every other revision (`batchRefused` / `runBatch`, internal/mcpserver/transport.go:106-116); see headline 11. The `-32700` behaviour recorded below predates that split.
- 2024-11-05: messages must "follow the JSON-RPC 2.0 specification" (which defines batches) but the schema's `JSONRPCMessage` has no array form. https://modelcontextprotocol.io/specification/2024-11-05/basic/messages
- 2025-03-26: "MCP implementations **MAY** support sending JSON-RPC batches, but **MUST** support receiving JSON-RPC batches." https://modelcontextprotocol.io/specification/2025-03-26/basic/index#batching — and "The initialize request **MUST NOT** be part of a JSON-RPC batch … This also permits backwards compatibility with prior protocol versions that do not explicitly support JSON-RPC batches." https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#initialization
- 2025-06-18: changelog "Remove support for JSON-RPC **batching**" https://modelcontextprotocol.io/specification/2025-06-18/changelog#major-changes ; Batching section and `JSONRPCBatchRequest` deleted. Every later revision still says "**MUST** follow the JSON-RPC 2.0 specification" without excluding batches.
- Effect: a batch is mandatory-to-accept only for 2025-03-26. A server serving 2025-03-26 and 2025-06-18 cannot know the version before parsing, so it has to accept arrays on the wire anyway (the version is inside the elements). mcpx rejects arrays (-32700).

**C2. Request-ID uniqueness scope.**
- **Disposition:** Undisposed — open question: does mcpx's client reuse request IDs after completion on a 2026-07-28 connection, and can a stale `subscriptions/listen` notification then collide with a new listen reusing the ID? No site verified in Phase 1; check request-ID allocation in internal/mcpclient.
- 2024-11-05..2025-11-25: "The request ID **MUST NOT** have been previously used by the requestor within the same session." https://modelcontextprotocol.io/specification/2025-11-25/basic/index#requests
- 2026-07-28: "The request ID **MUST NOT** match the ID of any other request the sender has issued and not yet received a response for." https://modelcontextprotocol.io/specification/2026-07-28/basic/index#requests
- Reuse after completion is legal in 2026, illegal in legacy. Internal tension in 2026: changelog item 9 says a broken stream's request must be re-issued "as a new request with a new request ID" (https://modelcontextprotocol.io/specification/2026-07-28/changelog#major-changes), and `subscriptionId` is "the JSON-RPC ID of the `subscriptions/listen` request" (schema NotificationMetaObject) — with reuse legal once a listen ends, a stale notification can carry an ID now used by a different listen.

**C3. Server-initiated requests, including `ping`.**
- **Disposition:** Peer-settled — the peer's era decides: legacy peers receive wire requests; a modern connection never does (server-initiated requests are governed/dropped per headline 2, internal/mcpclient/client.go), and methods a revision removed are answered `-32601` (internal/mcpserver/revisions.go:139).
- 2024-11-05..2025-11-25: requests go "from the client to the server or vice versa"; ping: "Either the client or server can initiate a ping" and "The receiver **MUST** respond promptly with an empty response". https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/ping#behavior-requirements
- 2026-07-28: "Servers **MUST NOT** initiate JSON-RPC requests" https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/index ; changelog "Remove `ping`, `logging/setLevel`, and `notifications/roots/list_changed`." https://modelcontextprotocol.io/specification/2026-07-28/changelog#major-changes
- A 2026 client receiving `ping` has no defined behaviour; a 2026 server receiving `ping` would answer -32601 (unknown method), which a legacy client reads as a dead peer (the very bug mcpx fixed in its client).

**C4. Error-response `id` when the request could not be read.**
- **Disposition:** Recorded reading — mcpx omits `id` on a parse-error response, following the 2025-11-25+ schema reading over JSON-RPC's `null` and the legacy schema's required non-null `id`; the legacy-schema invalidity is accepted and recorded here.
- JSON-RPC 2.0 (imported by "MUST follow"): id "MUST be Null" when it could not be detected.
- 2024-11-05..2025-06-18 schema: `JSONRPCError { id: RequestId }` (required, non-null); prose: "Responses **MUST** include the same ID as the request they correspond to." GH(2025-06-18) / https://modelcontextprotocol.io/specification/2025-06-18/basic/index#responses — and "the ID **MUST NOT** be `null`" is stated for requests only.
- 2025-11-25..2026-07-28: `JSONRPCErrorResponse { id?: RequestId }`, "(except in error cases where the ID could not be read due a malformed request)" https://modelcontextprotocol.io/specification/2026-07-28/basic/index#error-responses
- So a parse-error response is unrepresentable in the legacy schema (neither `null` nor absent allowed), `null` per JSON-RPC, and *absent* per 2025-11-25+ schema (which still admits no `null`). mcpx omits `id` — right for 2025-11-25+, schema-invalid for 2025-03-26/2025-06-18.

**C5. Version negotiation: MUST counter-offer vs example error (lifecycle page, all legacy revisions).**
- **Disposition:** Recorded reading — the legacy MUST governs: `negotiate()` (internal/mcpserver/server.go) counter-offers a supported version; `-32022` applies to modern `_meta`-versioned requests only. See headline 4.
- Prose: "Otherwise, the server **MUST** respond with another protocol version it supports." https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#version-negotiation
- Same page, "Example initialization error": `"code": -32602, "message": "Unsupported protocol version", "data": {"supported": [...], "requested": "1.0.0"}` https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#error-handling
- The example is a server doing what the MUST forbids. 2026 then standardises that example's shape under -32022 for modern requests only. mcpx now follows the legacy MUST: `negotiate()` (internal/mcpserver/server.go) counter-offers a supported version (its latest when the request is unknown or modern); `-32022` is used only for modern `_meta`-versioned requests, as 2026 specifies. (When this item was recorded, mcpx returned -32022 on legacy `initialize` too.)

**C6. Client on an unsupported negotiated version: SHOULD vs MUST (prose vs schema, 2024-11-05..2025-11-25).**
- **Disposition:** Undisposed — open question: when a server negotiates a version mcpx's client cannot support, does the client disconnect (prose SHOULD) or treat disconnect as mandatory (schema MUST)? No site verified in Phase 1; check the client negotiation path in internal/mcpclient.
- Prose: "If the client does not support the version in the server's response, it **SHOULD** disconnect." https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#version-negotiation
- Schema `InitializeResult.protocolVersion`: "If the client cannot support this version, it MUST disconnect." https://modelcontextprotocol.io/specification/2025-11-25/schema#initializeresult
- Unchanged in all four legacy revisions.

**C7. Using un-negotiated capabilities: SHOULD → MUST (2025-06-18).**
- **Disposition:** Undisposed — open question: does mcpx's server refuse a capability-gated method the peer never negotiated on a 2026-07-28 request (the MethodNotFoundError comment's reading), or does its accept-liberally rule apply across eras? No site verified in Phase 1.
- 2024-11-05, 2025-03-26: "Both parties **SHOULD**: … Only use capabilities that were successfully negotiated" https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#operation
- 2025-06-18, 2025-11-25: "Both parties **MUST**: …" https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#operation ; changelog "Change **SHOULD** to **MUST** in Lifecycle Operation".
- A server *accepting* an un-negotiated method (mcpx's accept-liberally rule) is not clearly "using" a capability; a *client* calling one clearly is. The 2026 MethodNotFoundError comment ("or one gated behind a server capability the server did not advertise") says the server should refuse such calls — https://modelcontextprotocol.io/specification/2026-07-28/schema#methodnotfounderror — contradicting accept-liberally for 2026.

**C9. Reserved `_meta` prefixes changed incompatibly (2025-06-18 → 2025-11-25).**
- **Disposition:** Undisposed — open question: does mcpx validate or generate `_meta` prefixes per era, and would a 2025-06-18 peer's legal `io.modelcontextprotocol/` data be misread as protocol data? No site verified in Phase 1.
- 2025-06-18: "Any prefix beginning with zero or more valid labels, followed by `modelcontextprotocol` or `mcp`, followed by any valid label, is **reserved** … `modelcontextprotocol.io/`, `mcp.dev/`, `api.modelcontextprotocol.org/`, and `tools.mcp.com/` are all reserved." https://modelcontextprotocol.io/specification/2025-06-18/basic/index#meta
- 2025-11-25+: "Any prefix where the second label is `modelcontextprotocol` or `mcp` is **reserved** … `io.modelcontextprotocol/` … However, `com.example.mcp/` is NOT reserved" https://modelcontextprotocol.io/specification/2026-07-28/basic/index#meta
- `io.modelcontextprotocol/` — the prefix of every 2026 protocol key — is **not** reserved under the 2025-06-18 rule (nothing follows `modelcontextprotocol`), so a 2025-06-18 implementation could legally have used it for its own data. Conversely `modelcontextprotocol.io/` and `tools.mcp.com/` are reserved in 2025-06-18 and free in 2025-11-25.

**C11. Error codes -32002 and -32042.**
- **Disposition:** Peer-settled — dual-era code choice is in code: legacy `-32002` (`codeResourceNotFoundLegacy`, internal/mcpserver/server.go) vs 2026 `-32602` (headline 6), and `-32042` belongs to the 2025-11-25 era only — the client propagates it untouched (internal/mcpclient/client.go:630); the server-side return gap is recorded at section E, spec-internal item 2.
- 2025-11-25 (and earlier for -32002): resource-not-found is -32002; `URL_ELICITATION_REQUIRED = -32042` under "Implementation-specific JSON-RPC error codes [-32000, -32099]". https://modelcontextprotocol.io/specification/2025-11-25/schema#urlelicitationrequirederror
- 2026-07-28: "Implementations of this protocol version **MUST NOT** emit these codes: `-32002` … `-32042`" https://modelcontextprotocol.io/specification/2026-07-28/basic/index#error-codes
- 2025-11-25 called -32000..-32099 wholly implementation-specific; 2026 carves -32020..-32099 out as spec-only. Any legacy SDK code in -32020..-32099 other than -32042 becomes forbidden to emit under 2026. A dual-era server must choose the code by the era of the request.

**C15. Lifecycle requirement itself.**
- **Disposition:** Peer-settled — the dual-era server keys lifecycle off the request's era: legacy `initialize` is served on the legacy path, while the modern path takes `_meta`-versioned requests and answers unknown/removed methods `-32601` (internal/mcpserver/server.go:879, :916).
- 2024-11-05..2025-11-25: "All implementations **MUST** support the base protocol and lifecycle management components." https://modelcontextprotocol.io/specification/2025-11-25/basic/index
- 2026-07-28: "All implementations **MUST** support the base protocol, versioning, and the message patterns." — and `initialize` is an unknown method to a modern-only server. https://modelcontextprotocol.io/specification/2026-07-28/basic/index

**C16. Where capabilities live, and whether they persist.**
- **Disposition:** Undisposed — open question: does any modern-path handler read capabilities stored from an earlier request on the same connection, contrary to the 2026 statelessness rule? Legacy session state is era-correct; no modern-path site verified in Phase 1.
- Legacy: capabilities "establish which optional protocol features will be available during the session" https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#capability-negotiation
- 2026: "Servers **MUST NOT** rely on prior requests … (e.g., capabilities, protocol version, client identity)" and schema "Servers MUST NOT infer capabilities from prior requests." https://modelcontextprotocol.io/specification/2026-07-28/basic/index#statelessness
- Remembering caps is required in one era and forbidden in the other; a dual-era server must key the rule off the request, not the connection.

**C17. `serverInfo` placement and `instructions`.**
- **Disposition:** Recorded reading — mcpx follows the 2026 placement (see the item text): `supportedVersions` in the discover result, `serverInfo` in `_meta`. The remaining `ttlMs`/`cacheScope` gap is headline 12.
- Legacy: `InitializeResult.serverInfo` (required), `instructions?` top-level.
- 2026: `serverInfo` only in `_meta["io.modelcontextprotocol/serverInfo"]` (SHOULD, every result); `DiscoverResult.instructions` top-level. https://modelcontextprotocol.io/specification/2026-07-28/server/discover#discoverresult
- mcpx's discover answer now uses the 2026 placement: `supportedVersions` in the result and `serverInfo` in `_meta` (added by the envelope to every modern result). Remaining gap: the discover result carries no `ttlMs`/`cacheScope` (headline 12).

**C18. Timeouts relocated and reworded.**
- **Disposition:** Undisposed — open question: on a 2026-07-28 HTTP request, does mcpx's sender cancel a timed-out request by closing the stream rather than sending `notifications/cancelled`? The relocation makes the two eras' timeout behaviour differ; no timeout-path site verified in Phase 1.
- 2025-03-26..2025-11-25 lifecycle: "the sender **SHOULD** issue a cancellation notification for that request" https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#timeouts
- 2026-07-28 cancellation page: "the sender **SHOULD** cancel the request and stop …" https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation#timeouts (cancellation owner has the details; the change matters because on HTTP 2026 cancellation is closing the stream, not a notification).

#### Within one revision

**C8. "requests other than pings and logging" (2024-11-05..2025-11-25).** "The server **SHOULD NOT** send requests other than pings and logging before receiving the `initialized` notification." https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle#initialization — logging is `notifications/message`, a notification, so the exception is to a rule about requests. Read literally, notifications are unrestricted before `initialized`; read as intended, "messages" was meant and progress/list_changed notifications are discouraged.
- **Disposition:** Recorded reading — the literal reading is recorded: the sentence restricts requests, so notifications before `initialized` are not restricted by it, whatever the drafters intended by "logging".

**C10. -32000..-32019: "legacy" (prose) vs "implementation-defined" (schema), 2026-07-28.**
- **Disposition:** Undisposed — open question: does mcpx emit any code in `-32000..-32019` of its own on a 2026-07-28 request (prose: SHOULD NOT; schema/changelog: grandfathered)? No emissions audit done in Phase 1.
- Prose: "**`-32000` to `-32019` — legacy.** … new implementations **SHOULD NOT** use codes from this sub-range at all." https://modelcontextprotocol.io/specification/2026-07-28/basic/index#error-codes
- Schema comment: "`-32000` to `-32019`: implementation-defined. Existing SDKs and implementations use codes here for their own purposes; the specification will never define codes in this sub-range" — no SHOULD NOT. Changelog item 12: "`-32000` to `-32019` remains implementation-defined (existing SDK usage is grandfathered)". https://modelcontextprotocol.io/specification/2026-07-28/changelog#minor-changes
- Also prose "Apart from `-32002` … receivers **MUST NOT** assume any specific meaning" — but -32002 is in the legacy sub-range, and the same page tells clients to accept it as resource-not-found: a single spec-assigned meaning inside a range the schema says "the specification will never define codes in".

**C12. Request `params` optional (prose/type) vs required `_meta` (schema), 2026-07-28.**
- **Disposition:** Undisposed — open question: does mcpx's client ever send a params-less 2026-07-28 request, and does its server reject a 2026 request whose `_meta` lacks the required fields? No site verified in Phase 1.
- basic/index shows `params?: { [key: string]: unknown }` for requests. https://modelcontextprotocol.io/specification/2026-07-28/basic/index#requests
- Schema `RequestParams { _meta: RequestMetaObject }` with required `protocolVersion`/`clientCapabilities`; "fields marked as required **MUST** be included on every request". So every 2026 request has `params`, including ones legacy defined without params. The generic `Request.params?` is kept "to allow unofficial extensions".

**C13. Error for a legacy `initialize` at a modern-only server, 2026-07-28.**
- **Disposition:** Recorded reading — mcpx answers an unknown method on the modern path with `-32601` (internal/mcpserver/server.go:879), i.e. the versioning matrix's unknown-method reading, not the `-32602` the malformed-request MUST picks.
- basic/index: "A request missing any required field is malformed; the server **MUST** reject it with JSON-RPC error code `-32602` (Invalid params). On HTTP, the response status **MUST** be `400 Bad Request`." https://modelcontextprotocol.io/specification/2026-07-28/basic/index#meta
- versioning matrix (Legacy/Modern): "the server rejects `initialize` with a JSON-RPC error; the exact code is implementation-defined (`initialize` is an unknown method and the request also lacks the required `_meta` fields)." https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning#compatibility-matrix
- stdio page: legacy servers answer pre-initialize probes "commonly `-32601` or `-32602`". Unknown-method (-32601) and malformed (-32602) are both reachable; the MUST picks one, the matrix says either.

**C14. HTTP era detection: `400` vs `4xx`, 2026-07-28.**
- **Disposition:** Undisposed — open question: which statuses trigger mcpx client era fallback — only `400` (binding page) or any `4xx` without a recognised modern error body (matrix)? See also section B item C9.
- Streamable HTTP: "On `400 Bad Request`, the client **SHOULD** inspect the response body before falling back" https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#backward-compatibility
- Versioning matrix (Dual-era/Legacy): "the modern request returns a `4xx` without a recognized modern error body, and the client falls back" https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning#compatibility-matrix
- A legacy 2025-03-26..2025-11-25 server answering a session-less non-initialize POST commonly returns 400 or 404 (session required / unknown session); the binding page only covers 400.

**C19. "Extensions … negotiated during initialization" in a revision with no initialization, 2026-07-28.**
- **Disposition:** Recorded reading — the versioning page governs over the index page's sentence: extensions are negotiated per request via capabilities and `server/discover`, there being no initialization in 2026-07-28.
- index.mdx: "Extensions are always opt-in and require explicit support from both client and server, negotiated during initialization." https://modelcontextprotocol.io/specification/2026-07-28/index#extensions
- versioning: extensions are advertised in capabilities, i.e. per request (`clientCapabilities.extensions`) and via `server/discover`. https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning#extension-negotiation

**C20. Architecture page vs statelessness, 2026-07-28.**
- **Disposition:** Recorded reading — the architecture page does not govern: servers declare features on `server/discover`, per-request declaration is the client's, and sampling's deprecation status is as in deprecated.mdx.
- architecture: "servers declare their supported features on each request" https://modelcontextprotocol.io/specification/2026-07-28/architecture/index#capability-negotiation — servers declare on `server/discover` only; per-request declaration is the client's.
- architecture keeps "Built on JSON-RPC, MCP provides a protocol focused on context exchange and sampling coordination" while Sampling is Deprecated (deprecated.mdx).
- architecture: "Receiving resource update notifications requires opening a `subscriptions/listen` stream" vs changelog: the server "acknowledges" — no server capability gate stated for update notifications, while `ServerCapabilities.resources.subscribe` still exists in schema.

**C21. Notifications direction vs server-initiated cancellation, 2026-07-28.**
- **Disposition:** Recorded reading — covered by the headline 1 reading: the server-sent `notifications/cancelled` that terminates a `subscriptions/listen` stream is the one exception to the patterns page's direction rule.
- patterns/index: "The **client** sends JSON-RPC _requests_ and _notifications_. The **server** answers each request with a JSON-RPC _response_ … optionally preceded by _notifications_ scoped to that request." https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/index
- schema CancelledNotification: "On stdio, the server also sends this notification, solely to terminate a `subscriptions/listen` stream" and CancelledNotificationParams.requestId "MUST correspond to the ID of a request the client previously issued" — a server-sent cancel that refers to the client's request, i.e. not "scoped to" a request it is answering but terminating it. (Cancellation/subscriptions owners hold the full version of this.)

**C22. `resultType` unknown-value rule vs open type, 2026-07-28.**
- **Disposition:** Undisposed — open question: how does mcpx's client treat an unrecognised `resultType`, and an absent one on a response to a request that carried `protocolVersion: 2026-07-28`? No site verified in Phase 1.
- basic/index: "A `resultType` of any value unrecognized by the client **MUST** be considered invalid." https://modelcontextprotocol.io/specification/2026-07-28/basic/index#resulttype
- schema: `type ResultType = "complete" | "input_required" | string;` and "Servers implementing this protocol version MUST include this field. For backward compatibility, when a client receives a result from a server implementing an earlier protocol version … the client MUST treat the absent field as `\"complete\"`." A client cannot tell a pre-2026 server from a 2026 server that forgot the field, and on a response to a request that carried `protocolVersion: 2026-07-28` the server by definition is not "implementing an earlier protocol version" — yet the only rule given is absent ⇒ complete.

**C23. `server/discover` field names: versioning page vs mcpx (not a spec-internal conflict; recorded because it breaks interop).** Spec: `supportedVersions` (schema DiscoverResult, discover.mdx, versioning page text "learn the server's supported versions"). When recorded, mcpx server and client both used `protocolVersions` and top-level `serverInfo`. Fixed: the server emits `supportedVersions` (internal/mcpserver/server.go, `server/discover` case) with `serverInfo` in `_meta`, and the client parses `supportedVersions` (internal/mcpclient/probe.go). No longer an interop break; kept as the record of one.
- **Disposition:** Recorded reading — resolved by fixing mcpx to the spec's names (see the item text): the server emits `supportedVersions` (internal/mcpserver/server.go) and the client parses it (internal/mcpclient/probe.go). Kept as the record of the break.

## B. Transports and authorization

C1. **Server-to-client requests / client-to-server responses: legal ≤2025-11-25, forbidden 2026-07-28.**
- **Disposition:** Peer-settled — frame legality is keyed to the connection's era in code: a modern connection carries no server-initiated wire requests (see section A item C3), so the forbidden frames cannot arise on it; legacy connections permit them.
- 2025-11-25: "The server **MAY** send JSON-RPC _requests_ and _notifications_ before sending the JSON-RPC _response_." — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#sending-messages-to-the-server ; POST body "**MUST** be a single JSON-RPC _request_, _notification_, or _response_" (same anchor).
- 2026-07-28: "The server **MUST NOT** send independent JSON-RPC _requests_ on this stream." — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#receiving-messages ; "The client **MUST NOT** send JSON-RPC _responses_." — #sending-messages ; stdio: "The server **MUST NOT** write JSON-RPC _requests_ to `stdout`." / "The client **MUST NOT** write JSON-RPC _responses_." — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio#receiving-messages
- Consequence: on one mcpx endpoint/process, whether a frame is legal depends on the connection's era, not the endpoint.

C2. **JSON-RPC batching: required-to-accept in 2025-03-26, removed 2025-06-18.**
- **Disposition:** Peer-settled — same site as section A item C1: `batchRefused` / `runBatch` (internal/mcpserver/transport.go:106-116); batches are received for 2025-03-26 only.
- 2025-03-26 stdio: "Messages may be ... a JSON-RPC batch" — https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#stdio ; HTTP: "An array batching one or more _requests and/or notifications_" — #sending-messages-to-the-server
- 2025-06-18: "Messages are individual JSON-RPC requests, notifications, or responses." ; "**MUST** be a single JSON-RPC _request_, _notification_, or _response_." — https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#stdio . Changelog: "Remove support for JSON-RPC batching (PR #416)".
- 2024-11-05 transports page is silent on batching (JSON-RPC 2.0 allows it).

C3. **Relatedness of messages on a POST's SSE stream: SHOULD → MUST.**
- **Disposition:** Undisposed — open question: does mcpx's server scope every notification on a POST's SSE stream to the originating request on a 2026-07-28 connection (MUST), and only by default on legacy (SHOULD)? No site verified in Phase 1.
- 2025-11-25: "These messages **SHOULD** relate to the originating client _request_." (#sending-messages-to-the-server)
- 2026-07-28: "These notifications **MUST** relate to the originating client request." (#receiving-messages). Unrelated server messages have no home except `subscriptions/listen`.

C4. **Closing a POST SSE stream before the response.**
- **Disposition:** Undisposed — open question: per era, when may mcpx's server close a POST SSE stream before the response — is the 2025-11-25 connection/stream distinction implemented, and is early close on 2026 treated as losing the request? No site verified in Phase 1.
- 2025-03-26..2025-06-18: "The server **SHOULD NOT** close the SSE stream before sending the JSON-RPC _response_ ... unless the session expires." — https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#sending-messages-to-the-server
- 2025-11-25: "the server **MAY** close the _connection_ (without terminating the _SSE stream_) at any time" (after an event id) — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#sending-messages-to-the-server . Legal only via the connection/stream distinction, which requires resumability.
- 2026-07-28: no resumability, and closing the stream **is** cancellation (C6), so a server can no longer close early without losing the request.

C5. **Resumability / `Last-Event-ID` / event IDs: optional 2025-03-26..2025-11-25 (with priming SHOULD in 2025-11-25), removed 2026.**
- **Disposition:** Undisposed — legacy resumability is implemented on the client (GET resume with `Last-Event-ID`, internal/mcpclient/http.go:332, :407); open question: does the modern path ignore `Last-Event-ID` as the 2026 backcompat text prescribes? No modern-path site verified in Phase 1.
- 2025-11-25: "The server **SHOULD** immediately send an SSE event consisting of an event ID and an empty `data` field" ; "Resumption is always via HTTP GET with `Last-Event-ID`." — #resumability-and-redelivery
- 2026-07-28: "Resumable SSE streams via `Last-Event-ID` are not supported." (#receiving-messages); modern-only server "SHOULD" ignore `Last-Event-ID` (#earlier-streamable-http-revisions).

C6. **Disconnect semantics inverted.**
- **Disposition:** Peer-settled — the era split is in code (headline 3): in a session a dropped connection is not a cancellation (legacy SHOULD NOT; the client cancels explicitly), while a sessionless disconnect is treated as the cancellation signal (internal/mcpserver/server.go, session disconnect block).
- 2025-03-26..2025-11-25: "Disconnection **SHOULD NOT** be interpreted as the client cancelling its request. To cancel, the client **SHOULD** explicitly send an MCP `CancelledNotification`." — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#sending-messages-to-the-server
- 2026-07-28: "Closing the SSE response stream **MUST** be treated by the server as cancellation of that request." — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#cancellation ; "on Streamable HTTP, closing the SSE response stream is itself the cancellation signal and no `notifications/cancelled` message is expected" (#sending-messages Note); cancellation page: "The server **MUST** treat a client disconnect as cancellation of that request." — https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation#transport-specific-cancellation
- Spec-internal (2026): the streamable-http page ties cancellation to "Closing the SSE response stream"; the cancellation page to "a client disconnect" — the latter also covers `application/json` responses, which have no SSE stream. A dual-era server must key the disconnect rule to the request's era.

C7. **HTTP 404 meaning.**
- **Disposition:** Peer-settled — era-keyed on both sides: the client recovers a session-carrying 404 by re-initialising only on the legacy path, and surfaces a modern-era 404 (internal/mcpclient/client.go:1138-1145); the server answers an unknown method on the modern path with 404 + `-32601` (internal/mcpserver/server.go:879). See headline 10.
- 2025-03-26..2025-11-25: 404 = session terminated; "When a client receives HTTP 404 in response to a request containing an `Mcp-Session-Id`, it **MUST** start a new session" — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#session-management ; backcompat: 404 on POST ⇒ try HTTP+SSE GET (#backwards-compatibility).
- 2026-07-28: 404 = unknown method: "it **MUST** respond with `404 Not Found` and a JSON-RPC error with code `-32601`" — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#protocol-version-header . A 2025-11-25 client that still holds a session id and gets this 404 is required to re-initialize; a 2025-11-25-rule client probing with POST falls to HTTP+SSE. Only the 2026 rule ("and the response body is not a recognized modern JSON-RPC error") disambiguates.
- Also a change against JSON-RPC-over-HTTP practice in 2025-*, where a method-not-found error was an ordinary response (status not specified; in practice 200).

C8. **Missing `MCP-Protocol-Version` header.**
- **Disposition:** Undisposed — open question: does mcpx assume `2025-03-26` for a header-less legacy request (2025-06-18..2025-11-25 SHOULD) and reject a header-less 2026 request as a header mismatch (`-32020`)? No site verified in Phase 1.
- 2025-06-18..2025-11-25: server "**SHOULD** assume protocol version `2025-03-26`" — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#protocol-version-header
- 2026-07-28: "**MAY** treat a request that omits the header as protocol version `2025-03-26`. A server that does not support such clients **MUST** reject a request without the header" (#protocol-version-header); Server Validation lists a missing `MCP-Protocol-Version` as a HeaderMismatch failure (#server-validation) → 400/-32020. Also 2025-11-25: "invalid or unsupported" header → 400 with no body format; 2026: unsupported → 400 + `UnsupportedProtocolVersionError`, mismatch → 400 + `HeaderMismatch`.
- 2025-06-18 header rule applies to "all subsequent requests" (after initialize); 2026: "Every POST request".

C9. **HTTP+SSE fallback trigger narrowed twice.**
- **Disposition:** Undisposed — open question: which status/body combinations make mcpx's client fall back to HTTP+SSE against each era's rule, including the 2026 recognised-modern-error-body condition? No site verified in Phase 1.
- 2025-03-26..2025-06-18: "If it fails with an HTTP 4xx status code (e.g., 405 Method Not Allowed or 404 Not Found)" — https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#backwards-compatibility
- 2025-11-25: "\"400 Bad Request\", \"404 Not Found\" or \"405 Method Not Allowed\"" — https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#backwards-compatibility (401/403 no longer trigger it — matters for OAuth-protected servers).
- 2026-07-28: those codes "**and** the response body is not a recognized modern JSON-RPC error" — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#httpsse-transport-2024-11-05 . A 2025-11-25-rule client probing a modern-only 2026 server gets 400 (missing headers → HeaderMismatch) and then GETs, gets 405, and fails with a misleading diagnosis.

C10. **Re-issue-after-broken-stream MUST lives only in the changelog.**
- **Disposition:** Undisposed — open question: does mcpx's client re-issue a request lost to a broken modern stream under a new request ID, and does it guard non-idempotent `tools/call` re-issue? No site verified in Phase 1.
- Changelog major 9: "clients **MUST** re-issue it as a new request with a new request ID" — https://modelcontextprotocol.io/specification/2026-07-28/changelog
- Transport page only: "Resumable SSE streams via `Last-Event-ID` are not supported." — no MUST; stdio page: "in-flight requests are simply lost and the client can retry them" (no keyword) — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio#unexpected-termination . Also re-issuing a non-idempotent `tools/call` is not qualified anywhere.

C11. **Notification POSTs in 2026 have no defined header requirements, yet all POSTs need them.**
- **Disposition:** Undisposed — open question: what header requirements does mcpx apply to notification POSTs from a 2026-07-28 peer, given the revision defines none? No site verified in Phase 1.
- "The client **MUST** include the request metadata headers on each POST request." / "Every POST request to the MCP endpoint **MUST** include an `MCP-Protocol-Version` header." vs Note: "header requirements for notification POSTs are not defined by this revision." and `Mcp-Method` "Required For: All requests" — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#sending-messages . The same Note says no client→server notifications exist on HTTP in 2026 at all, while the rules for accepting them (202) remain.

C12. **Server-sent `notifications/cancelled` for subscription teardown: scope disagreement (2026-07-28).**
- **Disposition:** Recorded reading — the headline 1 reading governs: on stdio the server sends `notifications/cancelled` and, for a graceful end, the listen result; on HTTP it answers the listen request and closes the stream. See section C item C1.
- schema.ts `CancelledNotification`: "On stdio, the server also sends this notification, solely to terminate a subscriptions/listen stream" — https://modelcontextprotocol.io/specification/2026-07-28/schema#cancellednotification
- cancellation page: "A server **MUST** send `notifications/cancelled` referencing a `subscriptions/listen` request ID when it tears down that subscription stream" (no transport qualifier) — https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation
- schema.ts `SubscriptionsListenResult`: "signalling that the subscription has ended gracefully ... this result is sent only when the server tears the subscription down" — so graceful teardown has two mandated signals (a result response and a cancelled notification), and on stdio the notification is also the only server→client message the 2026 index's "server-sent _responses_ and _notifications_" permits beyond responses. The stdio page's list of "three kinds of messages" the server writes does not include a server-sent `notifications/cancelled`. (Detail belongs to the subscriptions/cancellation owner; recorded here because it is transport-scoped.)

C13. **`MCP-Protocol-Version` header in 2025-03-26 authorization before the transport defines it.**
- **Disposition:** Undisposed — open question (spec-internal/historical): does mcpx send `MCP-Protocol-Version` on authorization discovery requests while omitting it on 2025-03-26 MCP requests, as that era's own pages imply? mcpx's discovery surface was not audited in Phase 1.
- 2025-03-26 authorization: "MCP clients _SHOULD_ include the header `MCP-Protocol-Version: <protocol-version>` during Server Metadata Discovery" with example `MCP-Protocol-Version: 2024-11-05` — https://modelcontextprotocol.io/specification/2025-03-26/basic/authorization#server-metadata-discovery-headers
- 2025-03-26 transports page defines no such header; it arrives in 2025-06-18 (#protocol-version-header), whose backcompat rule assumes a header-less request is 2025-03-26 — i.e. a 2025-03-26 client following its own auth page sends a header on discovery but not on MCP requests.

C14. **Dynamic Client Registration: SHOULD → MAY → deprecated.**
- **Disposition:** Undisposed — open question: does mcpx implement Dynamic Client Registration at all, and if so under which era's strength? Its authorization surface was not audited in Phase 1.
- 2025-03-26..2025-06-18: "**SHOULD** support the OAuth 2.0 Dynamic Client Registration Protocol" — https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization#dynamic-client-registration
- 2025-11-25: "**MAY** support" — https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization#dynamic-client-registration
- 2026-07-28: "Dynamic Client Registration is deprecated." — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration#dynamic-client-registration . Yet 2026 adds a new MUST inside it (`application_type`).
- 2026 security-considerations still links the proxy-consent MUST to "dynamically registered client" — a MUST scoped to a deprecated mechanism.

C15. **Where the authorization server lives / discovery.**
- **Disposition:** Undisposed — open question: which discovery rule does mcpx's client follow per era — 2025-03-26 default paths or Protected Resource Metadata with path-inserted well-known URIs? Not audited in Phase 1.
- 2025-03-26: MCP server is (or fronts) the AS; "authorization base URL **MUST** be determined ... by discarding any existing `path` component"; default `/authorize`, `/token`, `/register` fallbacks **MUST** be used — https://modelcontextprotocol.io/specification/2025-03-26/basic/authorization#authorization-base-url
- 2025-06-18+: MCP server is a resource server only; clients "**MUST** use OAuth 2.0 Protected Resource Metadata for authorization server discovery" and no default-path fallback exists — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization#overview . 2025-11-25+ uses path-inserted well-known URIs (keeps the path the 2025-03-26 rule discarded).

C16. **Token acceptance subject.**
- **Disposition:** Undisposed — open question: where does mcpx validate token audience — authorization server or resource server — for each era? Not audited in Phase 1.
- 2025-06-18: "Authorization servers **MUST** only accept tokens that are valid for use with their own resources." — https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization#token-handling
- 2025-11-25+: "MCP servers **MUST** only accept tokens that are valid for use with their own resources." — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization#token-handling

C17. **`WWW-Authenticate` on 401.**
- **Disposition:** Undisposed — open question: does mcpx's server emit `WWW-Authenticate` on 401, publish Protected Resource Metadata at the well-known URI, or both? Not audited in Phase 1.
- 2025-06-18: "MCP servers **MUST** use the HTTP header `WWW-Authenticate` when returning a _401 Unauthorized_" — https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization#authorization-server-location
- 2025-11-25+: "MCP servers **MUST** implement one of the following discovery mechanisms" (header or well-known) — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery#protected-resource-metadata-discovery-requirements . A 2025-06-18 client that only parses the header fails against a well-known-only 2025-11-25 server.

C18. **Who accumulates scopes on step-up.**
- **Disposition:** Undisposed — open question: on an insufficient-scope step-up, does mcpx's client union previously granted scopes itself (2026 rule) or rely on the server including them (2025-11-25 recommendation)? Not audited in Phase 1.
- 2025-11-25 (server side): "**Recommended approach**: Include both existing relevant scopes and newly required scopes to prevent clients from losing previously granted permissions" ; client step 2 uses the Scope Selection Strategy — https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization#runtime-insufficient-scope-errors
- 2026-07-28: "servers are not required to include the client's previously granted scopes" ; "Scope accumulation across operations is a client-side responsibility" ; client step 2: "computing the union of the client's previously requested scope set and the scopes from the current challenge" — https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization#runtime-insufficient-scope-errors . A 2025-11-25 client against a 2026 server that emits per-operation scopes drops earlier grants.

C19. **Sessions: mintable 2025-03-26..2025-11-25, removed 2026 — while 2026 still scopes legacy state "to the session (HTTP)".**
- **Disposition:** Peer-settled — sessions exist only on the legacy path in code (session minting, session-scoped state, and the session disconnect rule, internal/mcpserver/server.go); the modern path is per-request and stateless.
- 2026 changelog: "Remove protocol-level sessions and the `Mcp-Session-Id` header" ; streamable-http: modern-only server "ignore it, and do not mint or echo session IDs" (#earlier-streamable-http-revisions).
- 2026 versioning: "An `initialize` request selects legacy semantics, scoped to the stdio process (stdio) or the session (HTTP)" — https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning#backward-compatibility-with-initialization-based-versions . Not a contradiction for dual-era servers (they implement the legacy page too), but the 2026 transport page defines no session, so "the session (HTTP)" is defined only by the 2025-11-25 page it links.

C20. **"Supports both POST and GET" satisfied by 405.** 2025-* says the endpoint "**MUST** ... support both POST and GET" (#streamable-http) and, two sections later, that the server may "return HTTP 405 Method Not Allowed, indicating that the server does not offer an SSE stream" (#listening-for-messages-from-the-server). Spec-internal; the practical reading is that 405 is a conforming answer to GET.
- **Disposition:** Recorded reading — as the item states, the practical reading is that 405 is a conforming answer to GET; "support both" is read through the listening section, not against it.

C21. **Header name casing drift.** 2025-03-26..2025-06-18 `Mcp-Session-Id`; 2025-11-25 `MCP-Session-Id`; 2026 backcompat text back to `Mcp-Session-Id`. Harmless (2026: header names "**MUST** use case-insensitive comparisons") but any implementation that string-compares header names breaks.
- **Disposition:** Recorded reading — header names are compared case-insensitively (the 2026 rule, and Go `net/http` header handling throughout mcpx), so the casing drift is harmless here.

## C. Cancellation, progress, pagination, logging, completion, tasks, caching, MRTR, subscriptions

**C1 — 2026-07-28 server-initiated subscription teardown: three texts, three answers (spec-internal).**
- **Disposition:** Recorded reading — headline 1: on stdio send `notifications/cancelled` (the MUST), then the listen result for a graceful end (the SHOULD); on HTTP send the listen result and close the stream. The reading is recorded in the comment block at internal/mcpserver/listen.go.
- Cancellation page, intro (no heading): "A server **MUST** send `notifications/cancelled` referencing a `subscriptions/listen` request ID when it tears down that subscription stream (see [Subscriptions][subscriptions]). Servers **MUST NOT** send `notifications/cancelled` for any other purpose." and Behavior Requirements: "Server-sent cancellation notifications **MUST** reference a `subscriptions/listen` request, to terminate that subscription stream" — https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation , …#behavior-requirements
- Subscriptions page, Cancellation: "The **server** tears it down (e.g., during shutdown) — it **SHOULD** send a successful `subscriptions/listen` response to signal a graceful end (see Graceful Closure), then close the stream." and Graceful Closure: "When the server ends a subscription on its own initiative (for example, during shutdown), it **SHOULD** respond to the original `subscriptions/listen` request with a completion result before closing the stream." It never mentions a server-sent `notifications/cancelled`, and ends "See [Cancellation][cancellation] for the full rules." — https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/subscriptions#cancellation , #graceful-closure
- Schema `CancelledNotification`: "On stdio, the server also sends this notification, solely to terminate a subscriptions/listen stream … Servers MUST NOT use this notification to cancel any other request." — scopes the server-sent cancel to **stdio**; the prose MUST has no transport qualifier. https://modelcontextprotocol.io/specification/2026-07-28/schema#cancellednotification
- Streamable HTTP note: "The only client-sent notification in the core protocol, `notifications/cancelled`, is used only on the stdio transport" — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#sending-messages-to-the-server (heading before "Receiving Messages"; anchor unverified).
- Unresolved questions: (a) MUST cancel, SHOULD respond, or both? Doing both means sending `notifications/cancelled` for request id N *and* a result for id N, while the same cancellation page tells a *receiver* of a cancellation that the canceller "SHOULD ignore any response … that arrives afterward" — here the canceller and responder are the same party. (b) On HTTP, is the server-sent cancel required (prose) or not sent at all (schema "On stdio")? (c) The `SubscriptionsListenResult` doc says it is "sent only when the server tears the subscription down", consistent with the subscriptions page, not with the cancellation page. Safest reading for an implementer: on stdio send `notifications/cancelled` (satisfies the MUST) **then** the listen result (satisfies the SHOULD, and the client can tell graceful from abrupt); on HTTP send the listen result and close the SSE stream.

**C2 — server-sent requests and server-sent cancellation: legal ≤2025-11-25 → forbidden 2026-07-28.**
- **Disposition:** Peer-settled — the server's own `notifications/cancelled` for a timed-out `elicitation/create` (internal/mcpserver/conn.go:396) can only fire on a legacy-era connection, because a modern connection never carries the wire request it would cancel (see the item text).
- ≤2025-11-25: "Either side can send a cancellation notification to indicate that a previously-issued request should be terminated." https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/cancellation
- 2026: "Servers **MUST NOT** send `notifications/cancelled` for any other purpose." (above), and "Servers **MUST NOT** initiate JSON-RPC requests" https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns ; "The previous pattern of server-initiated requests is no longer supported. This is a breaking change." https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr
- Consequence for a dual-era server (mcpx): `conn.go:265` (cancel own timed-out `elicitation/create`) must stay reachable only on legacy-era connections. It is today, because a modern connection never gets a wire request.

**C3 — 2026-07-28 client cancellation on HTTP (spec-internal).**
- **Disposition:** Recorded reading — mcpx follows the cancellation page's intro SHOULD: as a client it POSTs `notifications/cancelled` on HTTP too (see the item text), rather than treating stream close as the only signal.
- "A client **SHOULD** send a cancellation notification to indicate that a request it previously issued should be terminated." https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation
- vs "**Streamable HTTP**: Closing the SSE response stream is the cancellation signal. … No `notifications/cancelled` message is required or expected." …#transport-specific-cancellation, and the streamable-http note that notification POST header requirements "are not defined by this revision".
- The intro SHOULD is unqualified; the transport section contradicts it on HTTP. mcpx as client POSTs `notifications/cancelled` on HTTP too.

**C4 — HTTP disconnect meaning: "not cancellation" (SHOULD NOT) 2025-03-26..2025-11-25 → "is cancellation" (MUST) 2026-07-28.**
- **Disposition:** Peer-settled — same era split as section B item C6 / headline 3 (internal/mcpserver/server.go, session disconnect block).
- 2025-11-25: "Disconnection **SHOULD NOT** be interpreted as the client cancelling its request." / "To cancel, the client **SHOULD** explicitly send an MCP `CancelledNotification`." https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#sending-messages-to-the-server (same text 2025-03-26, 2025-06-18).
- 2026: "The server **MUST** treat a client disconnect as cancellation of that request." https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/cancellation#transport-specific-cancellation ; "Closing the SSE response stream **MUST** be treated by the server as cancellation" https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http#cancellation
- A dual-era HTTP server must pick behaviour by the request's era.

**C5 — empty cursor: allowed to mean "end" ≤2025-11-25 → forbidden 2026-07-28.**
- **Disposition:** Peer-settled — headline 8: `nextPage` (internal/mcpclient/client.go) treats a missing cursor as the end in every revision and an empty-string cursor as a valid cursor in 2026-07-28.
- ≤2025-11-25 only says "Treat a missing `nextCursor` as the end of results" (an empty-string cursor was unaddressed; common SDK practice treats `""` as end).
- 2026: "an empty string is a valid cursor and thus **MUST NOT** be treated as the end of results" https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/pagination#implementation-guidelines . Also 2026 drops "Don't persist cursors across sessions" (no sessions).

**C6 — who may send progress: either side ≤2025-11-25 → server only 2026-07-28.**
- **Disposition:** Peer-settled — follows section C item C2: a client has incoming requests to report progress on only in the legacy era; in 2026 progress is server-sent only.
- ≤2025-11-25: "Either side can send progress notifications" https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/progress
- 2026: "The server **MAY** send progress notifications to report the status of requests the client has issued." https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/progress . Follows from C2 (a client has no incoming requests to report on).

**C7 — progress monotonicity strength (prose vs schema, all revisions).**
- **Disposition:** Recorded reading — as the item states, the prose MUST governs over the schema's lowercase "should".
- Prose: "The `progress` value **MUST** increase with each notification" https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/progress#progress-flow
- Schema: "The progress thus far. This should increase every time progress is made, even if the total is unknown." https://modelcontextprotocol.io/specification/2026-07-28/schema#progressnotificationparams . Lowercase, not RFC — treat prose as governing.

**C8 — log messages without a requested level: MAY ≤2025-11-25 → MUST NOT 2026-07-28.**
- **Disposition:** Undisposed — headline 9: the client relay gates on a requested level (`moreVerbose`, internal/mcpclient/relay.go); whether any path relays upstream logs to a host that requested no level — which would settle this item as moot in the relay direction — is the open question named there.
- ≤2025-11-25 schema: "If no logging/setLevel request has been sent from the client, the server MAY decide which messages to send automatically." https://modelcontextprotocol.io/specification/2025-11-25/schema#loggingmessagenotification
- 2026: "The server **MUST NOT** emit `notifications/message` for a request that does not include this field." https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/logging#per-request-log-level . Also session-wide → per-request, and `logging/setLevel` removed (changelog 5). Invalid level: ≤2025-11-25 rejects the `setLevel` call; 2026 "SHOULD reject that request" — i.e. the `tools/call` (or whatever) carrying the bad `_meta`.
- mcpx `docs/protocol.md` §2.2 says logging is "removed in 2026-07-28"; the spec *deprecates* it (still in schema `ServerCapabilities.logging`, still has a MUST-declare rule). Only `logging/setLevel` is removed.

**C9 — `completions` capability: absent 2024-11-05 → MUST declare 2025-03-26+.** A 2024-11-05 server could answer `completion/complete` with no capability; from 2025-03-26 "Servers that support completions **MUST** declare the `completions` capability". A client gating on the capability (mcpx) will never ask a 2024-11-05 server. mcpx serves nothing older than 2025-03-26, so moot on its server side.
- **Disposition:** Peer-settled — the client gates `completion/complete` on the declared `completions` capability (internal/mcpclient/request.go:45-61); mcpx serves nothing older than 2025-03-26, so the un-gated 2024-11-05 case is moot on its server side.

**C10 — "task required but not used" error code (2025-11-25, spec-internal).**
- **Disposition:** Undisposed — the actual behaviour is recorded, and it follows neither text: a non-task-augmented legacy call executes ordinarily, because `maybeTask()` returns nil before any task-support check when the call does not request a task (internal/mcpserver/tasks.go:237-240); the `-32601` branch at :241-245 covers the opposite case, an augmented call to a tool whose support is "forbidden". Open question: should the missing-required-task case refuse with `-32601` (tool-level MUST) or `-32600` (error list), or is ordinary execution the intended reading? See headline 13.
- Tool-level: "If `execution.taskSupport` is `"required"` … Servers **MUST** return a `-32601` (Method not found) error if a client does not attempt to do so." https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks#tool-level-negotiation
- Protocol errors: "receivers **MAY** return …: Non-task-augmented request when receiver requires task augmentation for that request type: `-32600` (Invalid request)" …#protocol-errors (and the "Task augmentation required" example there uses -32600). For `tools/call` the MUST (-32601) should win; for other request types -32600.

**C11 — cancelled task finality: MUST stay cancelled (2025-11-25) → may end non-cancelled (2026 extension).**
- **Disposition:** Undisposed — headline 13, first half: whether mcpx's tasks implementation holds a cancelled task in `cancelled` (2025-11-25 core) or permits a later non-`cancelled` terminal status (extension) was not verified from internal/mcpserver/tasks.go in Phase 1.
- 2025-11-25: "…**MUST** transition the task to `cancelled` status before sending the response." "Once a task is cancelled, it **MUST** remain in `cancelled` status even if execution continues to completion or fails." …#task-cancellation ; and the `tasks/cancel` response is the Task.
- Extension: "Acknowledge cancellation requests with an empty result. Honor them when possible, but cancellation is cooperative — the task may still reach a non-`cancelled` terminal status." docs/extensions/tasks/overview.mdx#for-mcp-servers ; lifecycle table: "`cancelled` | The operation was cancelled (not always honored)."
- Also 2025-11-25 "Receivers **MUST** reject cancellation requests for tasks already in a terminal status … `-32602`" has no counterpart in the overview.

**C12 — `related-task` on task-operation messages (2025-11-25, spec-internal).**
- **Disposition:** Recorded reading — the carve-out reading in the item text: the MUST covers messages of the underlying work and the `tasks/result` response; task-management messages follow their SHOULD NOTs.
- "All requests, notifications, and responses related to a task **MUST** include the `io.modelcontextprotocol/related-task` key" …#associating-task-related-messages ; "All requests, responses, and notifications associated with a task **MUST** include…" …#related-task-metadata
- vs "Requestors **SHOULD NOT** include `io.modelcontextprotocol/related-task` metadata in these requests" (tasks/get, /result, /cancel), "receivers **SHOULD NOT** include … in the result messages" (tasks/get, /list, /cancel), and "the `notifications/tasks/status` notification **SHOULD NOT** include the `io.modelcontextprotocol/related-task` metadata". Read as: the MUST covers messages of the *underlying* work (e.g. the elicitation a task raises and its response), plus the `tasks/result` response (explicit MUST); task-management messages are carved out. The text does not say so.

**C13 — extension opt-in wording (changelog vs extension page).**
- **Disposition:** Recorded reading — "unsolicited" is read as "without a `task` param", not "without declaring the extension": a task is never returned to a client that did not declare support (see the item text).
- Changelog 2026-07-28 item 6: the extension "allows servers to return task handles unsolicited without per-request opt-in" https://modelcontextprotocol.io/specification/2026-07-28/changelog
- Overview: "Never return a task to a client that did not declare support." and "Clients opt in once via the extension capability … No per-tool warmup or per-request flag." — but the capability itself is sent in per-request `_meta` ("Include the extension in its per-request capabilities"). "Unsolicited" means "without a `task` param", not "without declaring the extension". Model change vs 2025-11-25: requestor-driven (`task` param + tool `taskSupport`) → server-driven.
- Also 2025-11-25 `tasks/result`, `tasks/list` and client-side tasks (task-augmented sampling/elicitation) do not exist in the extension; `notifications/tasks/status` → `notifications/tasks` via `subscriptions/listen`, yet core `SubscriptionFilter` has no field for it (presumably extension-defined; not verifiable without ext-tasks).

**C14 — which results must carry cache hints (changelog vs caching page).**
- **Disposition:** Recorded reading — headline 12: the caching page and the schema govern over the changelog's list; `server/discover` results are `CacheableResult`s, with the `ttlMs`/`cacheScope` emission gap recorded at headline 12.
- Caching page includes `server/discover` in the MUST list; changelog item 5 lists "`tools/list`, `prompts/list`, `resources/list`, `resources/read`, and `resources/templates/list`" only. Page governs. Also the page says `ttlMs` absent "should only occur in older server versions" while schema makes `ttlMs`/`cacheScope` required — consistent (absence ⇒ legacy server). And `CacheableResult` doc: "how long … the client MAY cache this response before re-fetching" vs page "how long … the client MAY consider the result fresh" — wording only.

**C15 — 2026-07-28 "stream stays open … until the client cancels it" vs server teardown.** Subscriptions intro: "the stream stays open and delivers notifications until the client cancels it", then lists server teardown and transport close as other endings. Descriptive inconsistency only.
- **Disposition:** Recorded reading — descriptive inconsistency only, as the item states; the enumerated endings (client cancel, server teardown, transport close) govern over the intro's shorthand.

## D. Tools, resources, prompts, discover

**C1 — tool input-validation errors: protocol error → execution error (cross-revision).**
- **Disposition:** Undisposed — headline 5: the client choke keeps `-32602` (internal/mcpclient/schemacheck.go, internal/daemon/callerr.go); whether the serve seam era-splits `isError` for >=2025-11-25 hosts is the open question named there.
2024-11-05…2025-06-18: "Protocol Errors: Standard JSON-RPC errors for issues like: … Invalid arguments" and execution errors are "Invalid input data" — https://modelcontextprotocol.io/specification/2025-06-18/server/tools#error-handling.
2025-11-25+: "Tool Execution Errors … Input validation errors (e.g., date in wrong format, value out of range)"; protocol errors are only "Malformed requests (requests that fail to satisfy CallToolRequest schema)" — https://modelcontextprotocol.io/specification/2025-11-25/server/tools#error-handling. Changelog: "Clarify that input validation errors should be returned as Tool Execution Errors rather than Protocol Errors" (SEP-1303). A server emitting `-32602` for a bad date is following 2025-06-18 and deviating from 2025-11-25. `isError:true` is legal in both, so the conservative choice is isError.

**C2 — resource-not-found code: `-32002` SHOULD → `-32602` MUST (cross-revision).**
- **Disposition:** Peer-settled — headline 6: the code is picked per request revision in code (legacy `-32002`, `codeResourceNotFoundLegacy`, internal/mcpserver/server.go; 2026 `-32602`).
≤2025-11-25: "Resource not found: `-32002`" (SHOULD) — https://modelcontextprotocol.io/specification/2025-11-25/server/resources#error-handling.
2026-07-28: "servers **MUST** return a JSON-RPC error with code `-32602`"; "clients **SHOULD** also accept `-32002`" — https://modelcontextprotocol.io/specification/2026-07-28/server/resources#error-handling. Note 2026 also forbids new use of `-32000..-32019` codes except grandfathered (basic/index#error-codes), and `-32002` is in that range. A dual-era server must pick the code per request revision.

**C3 — resources list_changed delivery: 2026 resources page vs 2026 subscriptions rules (spec-internal).**
- **Disposition:** Recorded reading — follow the schema, as the item states: `resources/list_changed` is sent only on a listen stream whose filter requested it; the resources page's unqualified SHOULD does not govern.
2026 resources: "servers that declared the `listChanged` capability **SHOULD** send a notification" (no stream qualifier) — https://modelcontextprotocol.io/specification/2026-07-28/server/resources#list-changed-notification.
2026 tools/prompts say "to clients that have opened a `subscriptions/listen` stream with `toolsListChanged: true`", and SubscriptionFilter says "the server **MUST NOT** send notification types the client has not explicitly requested here" — https://modelcontextprotocol.io/specification/2026-07-28/schema#subscriptionfilter. Read literally, the resources page permits an unsolicited push that the schema forbids; the resources page was not updated. Follow the schema: only on a listen stream with `resourcesListChanged: true`.

**C4 — `resources.subscribe` capability meaning (cross-revision).**
- **Disposition:** Undisposed — open question: is the legacy `resources/subscribe` RPC answered only on legacy connections, and is the same capability flag read as the listen-filter meaning on 2026-07-28? No site verified in Phase 1.
Legacy: client may call `resources/subscribe` (https://modelcontextprotocol.io/specification/2025-11-25/server/resources#capabilities). 2026: "whether the server supports resource-specific update notifications for resources requested through subscriptions/listen using the resourceSubscriptions filter" (https://modelcontextprotocol.io/specification/2026-07-28/server/resources#capabilities). `resources/subscribe`/`unsubscribe` are removed ("Replaces the former `resources/subscribe` RPC", schema#subscriptionfilter; changelog item 4). Same flag, different method behind it.

**C5 — `structuredContent` / `outputSchema` shape (cross-revision; prose vs schema in 2025-06-18/11-25).**
- **Disposition:** Undisposed — headline 7: `downgrade()` (internal/mcpserver/revisions.go:237) deletes `structuredContent` only for pre-2025-06-18; what happens to a non-object value headed for a 2025-06-18..2025-11-25 host is the open question named there.
2025-06-18/2025-11-25 schema: `structuredContent?: { [key: string]: unknown }`; outputSchema "Currently restricted to type: \"object\" at the root level." — https://modelcontextprotocol.io/specification/2025-11-25/schema#calltoolresult.
2026-07-28: "This can be any JSON value (object, array, string, number, boolean, or null)"; outputSchema "can be any valid JSON Schema 2020-12" — https://modelcontextprotocol.io/specification/2026-07-28/server/tools#structured-content. A 2026 upstream's array result is illegal to send to a 2025-11-25 client verbatim. (The 2025-06-18/11-25 prose says "a JSON object"; consistent with its schema.)

**C6 — list/changed delivery and per-connection variance (cross-revision).**
- **Disposition:** Undisposed — open question: does mcpx's server deliver list_changed per era (session-wide legacy vs opted-in listen streams in 2026), and can its served lists vary per connection against the 2026 MUST NOT? No site verified in Phase 1.
Legacy list_changed is sent on the session to any client; 2026 only to opted-in listen streams (tools/prompts pages). 2026 adds "**MUST NOT** vary per-connection" for tools/prompts/resources lists — https://modelcontextprotocol.io/specification/2026-07-28/server/tools#capabilities; legacy imposes nothing, so session-scoped tool sets were legal before.

**C7 — where capabilities are declared (page vs page, 2026).**
- **Disposition:** Undisposed — open question: does mcpx's `DiscoverResult` declare the `prompts` capability in the result itself, as the 2026 prompts page requires, and the `tools`/`resources` capabilities observably at all? No site verified in Phase 1.
2026 prompts: "**MUST** declare the `prompts` capability in their `DiscoverResult`" — https://modelcontextprotocol.io/specification/2026-07-28/server/prompts#capabilities. 2026 tools/resources: "**MUST** declare the `tools` capability" with no location — https://modelcontextprotocol.io/specification/2026-07-28/server/tools#capabilities. Meanwhile discover prose makes calling discover optional ("a client may invoke any RPC inline"). So a client that never calls discover never sees server capabilities, yet tools/resources/prompts listChanged/subscribe semantics depend on them. Not a contradiction, but the capability is only observable via discover (or the listen ack's echoed filter).

**C8 — embedded resource "MUST include … MIME type" vs schema (prose vs schema, all revs).**
- **Disposition:** Recorded reading — as the item states: senders include the MIME type; receivers accept its absence, the schema's optional field governing interop.
Prompts prose: "**MUST** include: A valid resource URI; The appropriate MIME type; …" — https://modelcontextprotocol.io/specification/2026-07-28/server/prompts#embedded-resources. Schema `ResourceContents.mimeType?` is optional in every revision. Senders should include it; receivers must accept its absence.

**C9 — resource_link in prompts (prose vs schema, 2025-06-18/2025-11-25).**
- **Disposition:** Recorded reading — the schema governs: `ResourceLink` is a legal `PromptMessage` content block from 2025-06-18, whether or not that era's page documents it (verified in the item text).
`PromptMessage.content: ContentBlock` includes `ResourceLink` in the 2025-06-18 and 2025-11-25 schemas, but only the 2026 prompts page documents "Prompt messages **MAY** include links to Resources" — https://modelcontextprotocol.io/specification/2026-07-28/server/prompts#resource-links. (Verified: 2025-06-18 schema `PromptMessage.content: ContentBlock`, and `ContentBlock` includes `ResourceLink`.)

**C10 — discover call: MAY vs SHOULD (page-internal, conditional).**
- **Disposition:** Recorded reading — MAY in general, SHOULD for dual-era clients; mcpx's client in fact probes `initialize` first by design (internal/mcpclient/probe.go), which is recorded here rather than silently deviating.
"Calling `server/discover` is optional for clients" / schema "Clients **MAY** call it" vs "A client that supports both modern… and legacy… servers **SHOULD** send `server/discover` first" — https://modelcontextprotocol.io/specification/2026-07-28/server/discover#when-to-call. Reconciles as: MAY in general, SHOULD for dual-era clients on stdio. mcpx (dual-era client) probes `initialize` first by design.

**C11 — CacheableResult coverage: changelog vs schema.**
- **Disposition:** Recorded reading — headline 12: the schema governs over the changelog; `server/discover` also requires `ttlMs`/`cacheScope`, with the emission gap recorded at headline 12.
Changelog: "Require `ttlMs` and `cacheScope` fields on results returned by `tools/list`, `prompts/list`, `resources/list`, `resources/read`, and `resources/templates/list`" — https://modelcontextprotocol.io/specification/2026-07-28/changelog. Schema: `DiscoverResult extends CacheableResult`, and discover.mdx "This operation supports caching." Changelog omits discover (and prompts/get). Schema governs: discover also requires both fields.

**C12 — serverInfo placement (cross-revision) and required-ness.**
- **Disposition:** Recorded reading — same disposition as section A item C17: mcpx follows the 2026 placement on the modern path and the required top-level field on the legacy path.
InitializeResult.serverInfo is a required top-level field in legacy; in 2026 it moves to `_meta['io.modelcontextprotocol/serverInfo']` and is optional SHOULD (https://modelcontextprotocol.io/specification/2026-07-28/server/discover#discoverresult). Same for `protocolVersion` (single) → `supportedVersions` (list).

**C13 — tool name guidance vs aggregator reality.**
- **Disposition:** Undisposed — open question: when mcpx prefixes upstream tool names for disambiguation, what happens to names that would exceed 128 chars or leave the charset? No site verified in Phase 1.
"Tool names **SHOULD** be unique within a server" + charset `[A-Za-z0-9_.-]` + ≤128 chars (2025-11-25+) vs the 2026 note that proxies "**SHOULD** implement a disambiguation strategy such as prefixing tool names with a server identifier" — prefixing makes the 128 limit and charset binding on the proxy's separator and on upstream names that are already long or non-conforming. Not contradictory; a squeeze.

**C14 — isError vs "tool not found" (schema vs mcpx, not spec-internal).** Recorded here because it is easy to misread: the CallToolResult comment keeps "errors in _finding_ the tool … should be reported as an MCP error response" in every revision; only *argument* validation moved (C1).
- **Disposition:** Recorded reading — as the item states: errors in finding the tool are MCP error responses in every revision; only argument validation moved (section D item C1).

## E. Roots, sampling, elicitation

#### Cross-revision changes (a message legal in one revision is illegal or meaningless in another)

1. **Carriage of all three requests.** 2024-11-05–2025-11-25: server sends a JSON-RPC request
- **Disposition:** Peer-settled — mcpx bridges the eras by translation: wire requests on legacy connections, `inputRequests`/`inputResponses` entries on modern ones (internal/mcpserver/ask.go, internal/mcpserver/conn.go).
   (`"To request information from a user, servers send an \`elicitation/create\` request."`,
   https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#elicitation-requests).
   2026-07-28: `"The previous pattern of server-initiated requests is no longer supported. This is a breaking change."`
   (https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr). `ClientResult` in the 2026 schema is
   `EmptyResult` only — there is no client→server response message type at all. A proxy bridging eras must translate
   wire request ⇄ `inputRequests` entry, and wire response ⇄ `inputResponses` entry.
2. **Capability declaration.** "during initialization" (≤2025-11-25) → "in `_meta.io.modelcontextprotocol/clientCapabilities`
- **Disposition:** Peer-settled — era-keyed in code: declared at `initialize` on legacy connections and in per-request `_meta` client capabilities on modern ones, for roots, sampling and elicitation alike.
   on each request" (2026-07-28), for roots, sampling and elicitation. Same URLs as rows `*-declare-capability-*`.
3. **`roots.listChanged` and `notifications/roots/list_changed`.** ≤2025-11-25: "When roots change, clients that support
- **Disposition:** Peer-settled — the notification exists only in the legacy era; methods a revision removed are answered `-32601` on the modern path (internal/mcpserver/revisions.go:139; internal/mcpserver/server.go:916).
   `listChanged` **MUST** send a notification" (https://modelcontextprotocol.io/specification/2025-11-25/client/roots#root-list-changes).
   2026-07-28: capability is `roots?: {}`; changelog item 5: "Remove `ping`, `logging/setLevel`, and
   `notifications/roots/list_changed`" (https://modelcontextprotocol.io/specification/2026-07-28/changelog).
4. **Roots semantics.** ≤2025-11-25: "Roots define the boundaries of where servers can operate within the filesystem".
- **Disposition:** Recorded reading — for modern peers mcpx follows the 2026 reading (roots are informational guidance, not an access-control boundary); the legacy boundary wording governs legacy peers only.
   2026-07-28: "They are informational guidance rather than an access-control mechanism. The protocol does not enforce
   that servers stay within roots." (https://modelcontextprotocol.io/specification/2026-07-28/client/roots).
5. **Roots, Sampling deprecated** (SEP-2577) in 2026-07-28: "New implementations **SHOULD NOT** adopt it"
- **Disposition:** Recorded reading — deprecation is read as "should not adopt anew": mcpx continues to implement roots and sampling because its legacy peers require them; it does not treat the features as removed.
   (https://modelcontextprotocol.io/specification/2026-07-28/deprecated). Elicitation is not deprecated.
6. **Sampling error path.** ≤2025-11-25: "Clients **SHOULD** return errors for common failure cases" with `-1` for a
- **Disposition:** Undisposed — open question: under MRTR there is no error channel (see the spec-internal item 6 below); how does mcpx's client signal a declined or failed sampling request to a 2026-07-28 server — absence of a retry only? No site verified in Phase 1.
   rejected request (https://modelcontextprotocol.io/specification/2025-11-25/client/sampling#error-handling). 2026-07-28:
   "If an error occurs or the user declines the sampling request, the client does not need to replay the initial call with
   an error message" (https://modelcontextprotocol.io/specification/2026-07-28/client/sampling#error-handling). Roots has
   the same change (`-32601`/`-32603` → no error path).
7. **Tools in sampling** exist only from 2025-11-25 (`tools`, `toolChoice`, `tool_use`/`tool_result` content, content
- **Disposition:** Undisposed — open question: does mcpx shape sampling messages per peer era (single-block content for pre-2025-11-25 peers, arrays and tool content after), as it shapes `structuredContent` in `downgrade()` (internal/mcpserver/revisions.go)? The sampling path was not verified in Phase 1.
   arrays, `stopReason: "toolUse"`). 2024-11-05 `SamplingMessage.content` is `TextContent | ImageContent` (single block);
   2025-03-26 adds audio; arrays only from 2025-11-25. A 2025-11-25+ result with array `content` is unparseable by an
   older peer.
8. **`includeContext` values.** Plain in ≤2025-06-18; "soft-deprecated. Servers **SHOULD** avoid using these values ..."
- **Disposition:** Recorded reading — as the item states, the two texts are consistent in substance; the soft-deprecation date (2025-11-25) is the operative one.
   (https://modelcontextprotocol.io/specification/2025-11-25/client/sampling#capabilities); "deprecated under the feature
   lifecycle policy (SEP-2596)" in 2026-07-28. Schema 2026 `@deprecated` says "as of protocol version 2025-11-25" while the
   2026 changelog says "Reclassify ... (soft-deprecated since protocol version `2025-11-25`) as Deprecated" — consistent
   in substance, differing in which date is "deprecated".
9. **Elicitation sensitive information.** 2025-06-18: "Servers **MUST NOT** use elicitation to request sensitive
- **Disposition:** Undisposed — open question: does mcpx gate elicitation modes per era — form mode barred from sensitive information from 2025-11-25, URL mode required for it, the 2025-06-18 all-modes ban before that? The elicitation path (internal/mcpserver/ask.go) was not audited for this in Phase 1.
   information." (all modes; https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation#user-interaction-model).
   2025-11-25+: only form mode is forbidden, and URL mode is **required** for it.
10. **Client UI obligations promoted SHOULD → MUST** in 2025-11-25: which-server indication, review/modify (form),
- **Disposition:** Undisposed — open question (host-side): mcpx's client surfaces elicitation to its host UI; whether the 2025-11-25 MUSTs (which-server indication, review/modify, decline/cancel) are all met by that surface was not audited in Phase 1.
    decline/cancel options. 2025-06-18 "Applications **SHOULD**: ..." vs 2025-11-25 "MCP clients **MUST**: ..."
    (https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#user-interaction-model).
11. **Elicitation request shape.** 2025-06-18 `params: {message, requestedSchema}` — no `mode`, no `default`, enum is
- **Disposition:** Undisposed — open question: are elicitation params shaped per peer era (`mode`/`elicitationId`/url mode withheld from a 2025-06-18 peer, `elicitationId` withheld from 2026-07-28)? No site verified in Phase 1.
    `{enum, enumNames}`. 2025-11-25 adds `mode`, url mode, `elicitationId`, defaults, `oneOf`/array enums,
    `LegacyTitledEnumSchema`. 2026-07-28 removes `elicitationId`. Sending `mode`/`elicitationId` to a 2025-06-18 peer is
    sending fields its schema lacks; sending url mode to it is sending a request it cannot represent.
12. **`elicitationId`, `notifications/elicitation/complete`, `URLElicitationRequiredError` (-32042)** exist only in
- **Disposition:** Peer-settled — the trio exists in the 2025-11-25 era only, and 2026-07-28 forbids emitting `-32042`; mcpx's client handles and propagates `-32042` on that era's path (internal/mcpclient/client.go:1549-1586), while the server-side return gap is recorded at the spec-internal item 2 below.
    2025-11-25. 2026-07-28 changelog item 11 removes the first two; basic/index: "Implementations of this protocol version
    **MUST NOT** emit these codes: ... `-32042`" (https://modelcontextprotocol.io/specification/2026-07-28/basic/index#error-codes).
13. **Elicitation statefulness.** 2025-11-25: "Most practical uses of elicitation require that the server maintain state"
- **Disposition:** Undisposed — open question: does mcpx's URL-mode elicitation keep server state keyed by anything other than a session ID alone (2025-11-25 rule), and does it rely on no state at all in 2026 form mode? Not audited in Phase 1.
    + "State **MUST NOT** be associated with session IDs alone"
    (https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#statefulness). 2026-07-28: "Elicitations
    do not require that the server maintain state ... However, if state is stored ..." and the session-ID rule is gone
    (sessions no longer exist) (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#statefulness).
14. **Elicitation error handling.** 2025-11-25 prescribes `-32042` (server) and `-32602` for an undeclared mode (client).
- **Disposition:** Undisposed — open question: which error surface does mcpx use per era — `-32042` / `-32602` in 2025-11-25 (but see the spec-internal item 2 below) vs decline/cancel handling with no codes in 2026-07-28? Cross-reference headline 5's serve-seam question.
    2026-07-28 replaces both with "Servers **SHOULD NOT** assume ... and **MUST** handle cases where the user declines or
    cancels ..." (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#error-handling).
15. **Client rate limiting** for elicitation: SHOULD in 2025-06-18 and 2025-11-25, dropped in 2026-07-28 (sampling keeps it).
- **Disposition:** Undisposed — open question: does mcpx's client rate-limit elicitation requests at all, and if so is the limit era-scoped now that 2026-07-28 drops the SHOULD? Not audited in Phase 1.
16. **New 2026-only sampling MUSTs**: "Sampling messages **MUST** contain a `role` ... and a `content` field", "The client
- **Disposition:** Undisposed — open question: does mcpx's client enforce the 2026 sampling MUSTs (`role` and `content` present, `maxTokens` respected, no retention between requests) on MRTR-carried sampling? Not audited in Phase 1.
    **MUST** respect the `maxTokens` parameter", "SHOULD NOT be retained between separate requests"
    (https://modelcontextprotocol.io/specification/2026-07-28/client/sampling#messages,
    https://modelcontextprotocol.io/specification/2026-07-28/client/sampling#sampling-parameters).
17. **Task-augmented sampling/elicitation** (`params.task`, `tasks.requests.sampling.createMessage`,
- **Disposition:** Undisposed — headline 13 area: task augmentation exists only in 2025-11-25 core; whether mcpx strips or rejects a `task` param headed for a 2026-07-28 peer was not verified in Phase 1.
    `tasks.requests.elicitation.create`) exist in 2025-11-25 core
    (https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks). The 2026-07-28 `CreateMessageRequestParams`
    and `ElicitRequest*Params` have no `task` field, and 2026 `ClientCapabilities` has no `tasks`.

#### Spec-internal contradictions

1. **`mode` "MUST include" yet optional** (2025-11-25, 2026-07-28): "All elicitation requests **MUST** include the following
- **Disposition:** Recorded reading — as the item states: only `message` is unconditionally required; `mode` defaults to `"form"` and may be omitted for form mode.
   parameters: ... `mode` ... Optional for form mode (defaults to `\"form\"` if omitted)." followed by "servers **MAY**
   omit the `mode` field" (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#elicitation-requests).
   Resolution by reading: only `message` is unconditionally required.
2. **`-32042`: MAY vs MUST** (2025-11-25). "the server **MAY** return a `URLElicitationRequiredError`"
- **Disposition:** Recorded reading (reading chosen; implementation gap recorded) — mcpx's reading is the MUST: a request that cannot be processed until an elicitation completes should return `-32042`. The server does not yet do it: a URL question the legacy client cannot answer stays with the broker while the server waits (internal/mcpserver/ask.go:192-207), and the gap is recorded in the conformance registry (`elicitation-server-must-return-32042`, internal/conformance/coverage_clientfeatures.go, #214). The client side does handle receiving `-32042` (internal/mcpclient/client.go:1549-1586).
   (https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#url-elicitation-required-error) vs
   "Servers **MUST** return standard JSON-RPC errors for common failure cases: - When a request cannot be processed until
   an elicitation is completed: `-32042`" (https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#error-handling).
   Also the schema marks `URL_ELICITATION_REQUIRED` and `URLElicitationRequiredError` `@internal`, and the 2025-11-25
   changelog links `#url-elicitation-requests`, an anchor that does not exist (heading is "URL Mode Elicitation Requests").
3. **Roots: guidance vs access control** (2026-07-28). "They are informational guidance rather than an access-control
- **Disposition:** Recorded reading — the roots page's main text governs (informational guidance) over the security-considerations boilerplate; see the cross-revision item 4 above.
   mechanism" (https://modelcontextprotocol.io/specification/2026-07-28/client/roots) vs "Clients **MUST**: ... Implement
   proper access controls" and servers "**SHOULD** ... Validate all paths against provided roots"
   (https://modelcontextprotocol.io/specification/2026-07-28/client/roots#security-considerations).
4. **Roots: change monitoring without a change signal** (2026-07-28). "Clients **SHOULD**: ... Monitor for root changes"
- **Disposition:** Undisposed — open question: in 2026-07-28 there is no signal to monitor with; does mcpx's client re-list roots per request instead of caching them, and if it caches, what invalidates the cache? Not audited in Phase 1.
   (https://modelcontextprotocol.io/specification/2026-07-28/client/roots#implementation-guidelines) survives, but there is
   no longer any way to tell the server; and "Cache root information appropriately" has no invalidation signal.
5. **Deprecated but MUST declare** (2026-07-28): roots and sampling pages say new implementations "**SHOULD NOT** adopt it"
- **Disposition:** Recorded reading — as the item states, the texts are consistent read as "if you implement it anyway": support entails the per-request declaration MUST; non-adoption is the SHOULD NOT's subject.
   while still "Clients that support roots **MUST** declare ... on each request" — consistent only if read as
   "if you implement it anyway".
6. **Tools-without-capability: client MUST error, but 2026 has no error channel.** Schema 2026: "The client MUST return an
- **Disposition:** Undisposed — open question (same gap as the cross-revision item 6 above): the only expressible client "error" under MRTR is not retrying, or a client-side failure; which one mcpx's client produces for an undeclared `sampling.tools` field was not verified in Phase 1.
   error if this field is provided but ClientCapabilities.sampling.tools is not declared."
   (https://modelcontextprotocol.io/specification/2026-07-28/schema#createmessagerequestparams). In 2026 the request
   arrives inside `InputRequiredResult.inputRequests`; `InputResponse` is `CreateMessageResult | ListRootsResult | ElicitResult`
   — no error member — and the sampling page says "the client does not need to replay the initial call with an error
   message". The only expressible "error" is not retrying, or a client-side failure.
7. **Elicitation "client fails to process" in 2026.** "**MUST** handle cases where ... the client fails to process the
- **Disposition:** Undisposed — open question: under MRTR the server sees only absence; does mcpx's server re-request missing elicitation input (the mrtr SHOULD) or fail the original request? Not audited in Phase 1.
   request" (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#error-handling) — under MRTR the
   server cannot observe that; it sees only absence of a retry, or a retry missing that key (mrtr: "SHOULD respond with a
   new `InputRequiredResult` requesting the missing information again").
8. **Missing capability: error vs. omit** (2026-07-28). basic/index: "the server **MUST** return a
- **Disposition:** Undisposed — open question: when mcpx's server requires elicitation a client has not declared, does it return `-32021` (MissingRequiredClientCapabilityError), and when it can degrade, does it degrade silently? Not audited in Phase 1.
   `MissingRequiredClientCapabilityError` (`-32021`)" (https://modelcontextprotocol.io/specification/2026-07-28/basic/index#meta)
   vs mrtr: "Servers **MUST NOT** send an `inputRequests` that the client has not declared support for" and elicitation:
   "Servers **SHOULD NOT** assume that elicitation requests will always succeed". Together they force: a server that
   *requires* elicitation errors `-32021`; one that can degrade must degrade silently. Neither says which a server that
   *could* use elicitation but has a fallback must pick.
9. **Elicitation page vs mrtr on where elicitation may occur.** "Servers **MAY** request information from a user during the
- **Disposition:** Undisposed — open question: which client requests may carry an `InputRequiredResult` from mcpx's server — any (elicitation page) or only `prompts/get`, `resources/read`, `tools/call` (mrtr)? Check the ask path in internal/mcpserver/ask.go.
   processing of a client request" (any request) vs mrtr "Servers **MUST NOT** send `InputRequiredResult` responses on any
   other client requests" than `prompts/get`, `resources/read`, `tools/call`
   (https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr#supported-requests).
10. **`_meta` preservation addressed to the wrong side.** Schema `ToolUseContent._meta` / `ToolResultContent._meta`:
- **Disposition:** Undisposed — open question: mcpx's server constructs subsequent sampling requests; does it preserve `ToolUseContent._meta` / `ToolResultContent._meta` when doing so, as the mis-addressed SHOULD intends? Not audited in Phase 1.
    "Clients SHOULD preserve this field when including tool uses in subsequent sampling requests"
    (https://modelcontextprotocol.io/specification/2026-07-28/schema#toolusecontent) — subsequent sampling requests are
    constructed by the *server*.
11. **`maxTokens`: MUST respect vs MAY sample fewer** (2026-07-28). Compatible (a ceiling), but "respect" is undefined
- **Disposition:** Recorded reading — as the item states, the texts are compatible: `maxTokens` is a ceiling; sampling fewer is respecting it.
    beyond not exceeding it (https://modelcontextprotocol.io/specification/2026-07-28/client/sampling#sampling-parameters vs
    schema `CreateMessageRequestParams.maxTokens`).
12. **2025-11-25 StringSchema example uses `pattern`** (`"pattern": "^[A-Za-z]+$"`,
- **Disposition:** Recorded reading — the type governs over the example: `pattern` is not a 2025-11-25 `StringSchema` keyword, whatever the example shows; the 2026 example's removal confirms the reading.
    https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation#requested-schema), a keyword the 2025-11-25
    `StringSchema` type does not define. Removed from the 2026 example.
13. **Unconditional user binding** (2025-11-25, 2026-07-28): "Servers **MUST** bind elicitation requests to the
- **Disposition:** Undisposed — open question: how does mcpx bind an elicitation request to a user identity on stdio or without authorization, where no identity exists? Not audited in Phase 1.
    client and user identity" while there is no user identity on stdio / without authorization; "when possible" appears
    only in the Statefulness bullet (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#security-considerations).
14. **Sampling "Result Fields" vs tool-result constraint** (2026-07-28): result `content` may be "multiple tool uses or
- **Disposition:** Recorded reading — as the item states: the user-message ONLY-tool-results rule is not applied to result content; a proxy must not apply it there.
    mixed content" (assistant side) — not a contradiction with "user message ... **MUST** contain ONLY tool results", but a
    proxy must not apply the user-message rule to results.
15. **URL-mode requestState and "stateless" claim** (2026-07-28): Statefulness says elicitation does not require server
- **Disposition:** Undisposed — open question (same area as the cross-revision item 13 above): token custody makes the OAuth server stateful regardless of elicitation's statelessness; whether mcpx's URL-mode flow stores `requestState` server-side was not audited in Phase 1.
    state, while the OAuth section still requires "The MCP server is responsible for tokens ... (in other words, the MCP
    server must be stateful)" (https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation#understanding-the-distinction).
