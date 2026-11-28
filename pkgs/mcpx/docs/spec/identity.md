# Who an MCP request is from

mcpx's own MCP server passes every call on to an upstream server, and which
_instance_ of that server answers depends on its `scope`: one per session, per
cwd, per pid, per repository, or per call. So every request has to answer the
question "who is asking?", and the answer has to come from the transport, not
from the process.

## What went wrong

The backend used to ask the process. `mcpSession()` returned
`$MCPX_SESSION_ID` or `mcp-<pid>`, and `callContext()` filled in the process's
cwd and pid. That is right for `mcpx serve`, where one process serves one
client over stdio. It is wrong inside the daemon, which serves `/mcp` for
every HTTP client from one process: every client, every `Mcp-Session-Id`,
resolved to `session:mcp-<daemonpid>`, `cwd:<daemon's cwd>` and
`pid:<daemonpid>`. Two agents talking to one daemon shared one browser, one
REPL, one set of cookies, and saw each other's `mcpx_exec` artifacts in
`resources/list` (conflict #2 in the WP7 record). Over stdio the same test
gave separate instances, so the two transports disagreed about what `scope:
session` means.

## What mcpx does now

`mcpserver` resolves an `Identity` for each request from the connection it
arrived on, and puts it in the request's context
(`internal/mcpserver/identity.go`). The backend builds the daemon call context
from that (`mcpCaller` in `internal/cli/serve.go`):

| request arrives on | identity | session | cwd / pid |
| --- | --- | --- | --- |
| stdio (`mcpx serve`) | the process | `$MCPX_SESSION_ID`, else `mcp-<pid>` | the process's |
| a legacy Streamable HTTP session | its `Mcp-Session-Id` | `mcp-<session id>` | unknown |
| any request carrying `_meta["dev.mcpx/session"]` | that name | the name, verbatim | unknown |
| any request carrying `_meta["ai.opencode/sessionID"]` (and no `dev.mcpx/session`) | that id | the id, verbatim | unknown |
| a 2026-07-28 request without it, or a legacy POST outside a session | none | none: a fresh call id | unknown |

The rows are in precedence order except that an explicit session key
wins over the connection's own identity. When a request carries more
than one listed key, the first key in `transport.sessionMetaKeys` order
whose value is a non-empty string wins.

### Why cwd and pid are unknown over HTTP

The daemon's cwd and pid are not the client's, and the transport carries
neither. A scope that needs one (`cwd`, `repo`, `worktree`, `pid`) degrades to
per-call and logs the degradation once, which is the direction that can't
leak one client's state into another. A client's `roots` would be a better
source for cwd. That needs a `roots/list` round trip that mcpx doesn't make
yet.

### Why a modern request gets no identity

2026-07-28 has no sessions. The streamable-http page tells a server to ignore
`Mcp-Session-Id` and not to mint one, so no connection-level identity exists.
mcpx doesn't invent one: an unnamed request is its own scope, which for a
`session`-scoped upstream means a new instance per request.

### The session keys: `transport.sessionMetaKeys`

For a client that wants continuity, such as an agent host that keeps one
browser across its calls, mcpx reads a session name (a string) from any
request of any revision, under the keys listed in the
`transport.sessionMetaKeys` setting, in order. The default list is
`dev.mcpx/session` — mcpx's own key, which uses mcpx's prefix because the
specification reserves `io.modelcontextprotocol/` and defines no session
key of its own — followed by `ai.opencode/sessionID`, the key OpenCode v2
sends on every `tools/call`, so each OpenCode session is scoped as its
own session instead of sharing the daemon's. A deployment can replace
the list (dropping either key, or adding a host's own) in configuration;
an empty or non-string value is ignored, as if the key were absent. The value is used verbatim as the session id, so a client can join
the same session a CLI user names with `--session` or `$MCPX_SESSION_ID`.
That is deliberate. Anyone who can reach the daemon can already run any tool
as its user (see the warning the daemon prints for a non-loopback address), so
a session name is a label and grants no authority.

### `ai.opencode/sessionID` and `transport.sessionMetaKeys`

opencode v2 puts `_meta["ai.opencode/sessionID"]` on every `tools/call`,
including calls made from inside its code mode, so a client that never
injects `MCPX_SESSION_ID` can still get one stateful scope per opencode
session. mcpx reads a precedence list of `_meta` keys, `transport.sessionMetaKeys`
(default `dev.mcpx/session,ai.opencode/sessionID`): the first key present on a
request wins, and an empty value counts as absent. `dev.mcpx/session` is first
so a name chosen for mcpx beats the id a host attaches automatically; a client
that sends only the opencode key is keyed by it verbatim, exactly as it would
be by `dev.mcpx/session`. The setting is hot: a change applies to the next
request, not the next daemon start. Send a key outside the list and it is
ignored, which is the documented fallback rather than an error.

## What it touches

Identity reaches every backend path that picks an instance: `mcpx_call`,
`mcpx_exec` (its artifact session), `resources/read`, `prompts/get`,
`completion/complete`, the artifact listing in `resources/list`, and the ask
path (`/v1/ask` with the same context). A caller with no identity lists no
artifacts, because the daemon reads an empty session as "everyone's".

The tests are
`2025-11-25/transport/http-mcp-clients-do-not-share-scoped-instances`,
`2026-07-28/transport/client-named-session-is-kept-across-requests`,
`2026-07-28/transport/unnamed-requests-are-their-own-scope` (e2e, against a
session-scoped fakemcp), and the `TestEachConnectionHasItsOwnIdentity` cases
in `internal/mcpserver/wp11_test.go`.
