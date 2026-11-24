# Public Go client for /v1 (`pkg/client`)

`pkgs/mcpx/pkg/client` is the public Go client for the mcpx daemon's
/v1 API (#1877, rung 1). Its calls are derived at run time from the
daemon's operation table (the `Ops` function in `internal/api`) — method, path, and
parameter placement come from the table, never from hand-written
per-operation code — so the client cannot drift from the daemon the
way a hand-maintained client would.

## The API

One client, two call shapes, and the table itself for callers who
need to know what exists before calling. The signatures below are
generated from the Go source; the prose around them is the chapter:

<!-- BEGIN GENERATED: public-client-signatures -->

| signature                                                                                    | what it is                                                                                                                                                                                                                                         |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `type ArgError struct`                                                                       | ArgError reports an argument that does not fit the operation's declared parameters: an undeclared name, a missing required parameter, or a value outside a declared enum. It is a client-side refusal — no request was sent.                       |
| `type Client struct`                                                                         | Client talks to one mcpx daemon over HTTP at its /v1 base endpoint, for example http://127.0.0.1:7432 (the daemon's loopback endpoint, as reported by the health operation).                                                                       |
| `type HTTPError struct`                                                                      | HTTPError is a non-2xx answer from the daemon, with the daemon's error envelope decoded. Body keeps the raw answer for the routes whose failure payload is itself information.                                                                     |
| `type Option func(*Client)`                                                                  | Option configures a Client.                                                                                                                                                                                                                        |
| `Invoke[T any](ctx context.Context, c *Client, name string, args map[string]any) (T, error)` | Invoke is Call with the answer as a return value, for callers that prefer generics over an out parameter.                                                                                                                                          |
| `New(baseURL string, opts ...Option) *Client`                                                | New builds a Client for the daemon at baseURL. The trailing slash is optional; operation paths from the table are appended to it.                                                                                                                  |
| `Operation(name string) (api.Op, bool)`                                                      | Operation looks one operation up by its table name (for example "task_get"). The second result is false for a name the table does not declare.                                                                                                     |
| `Operations() []api.Op`                                                                      | Operations returns the daemon's /v1 operation table: every operation Call can reach, with its declared parameters. The list is the table itself, so it is current by construction.                                                                 |
| `WithHTTPClient(hc *http.Client) Option`                                                     | WithHTTPClient replaces the HTTP client, for callers that need a custom transport (a unix-socket dialler, a test round-tripper, TLS).                                                                                                              |
| `WithToken(token string) Option`                                                             | WithToken sets the bearer token sent on every request, matching the daemon's Authorization: Bearer convention. An empty token sends no header, which is the loopback daemon's usual case.                                                          |
| `(*ArgError).Error() string`                                                                 | Error reports the operation, parameter, and reason the call was refused for.                                                                                                                                                                       |
| `(*Client).Call(ctx context.Context, name string, args map[string]any, out any) error`       | Call performs the named operation with the given arguments and, when out is non-nil, decodes the JSON answer into it. Arguments are keyed by the parameter names the table declares; where each travels is the table's business, not the caller's. |
| `(*Client).CallRaw(ctx context.Context, name string, args map[string]any) ([]byte, error)`   | CallRaw performs the named operation and returns the raw answer body. A non-2xx answer is an *HTTPError.                                                                                                                                           |
| `(*HTTPError).Error() string`                                                                | Error reports the daemon's message for a non-2xx answer, or the raw body when it is not that shape.                                                                                                                                                |

<!-- END GENERATED: public-client-signatures -->

`ErrUnknownOperation` is the package-level sentinel `Call` returns
(wrapped) when it names an operation the table does not declare. It is
a variable, not a callable, so it stays in this prose rather than in
the generated list above.

## An example

The shape of real use, from the package's own tests
(`pkg/client/client_test.go`), which drive it against a test
daemon:

```go
c := client.New(srv.URL, client.WithToken("test-token"))

var task struct {
    Status string `json:"status"`
}
if err := c.Call(ctx, "task_get", map[string]any{"id": "t 1/2"}, &task); err != nil {
    // *client.ArgError names a bad argument, *client.HTTPError a
    // refused call, and an error wrapping client.ErrUnknownOperation
    // means the name is not in the table at all.
    return err
}
```

## Who calls it

Nothing in this repository does — yet. The CLI, the daemon and the
plugin each reach the operation table in-process; `pkg/client` is
the rung of #1877 that exists for callers outside the module:
embedders who want the daemon's API as a library surface instead
of a subprocess. That is also why the derivation rule matters more
than any one signature: an outside caller pinned to the table
inherits new operations as the daemon declares them.

## Stability promise

The package is **pre-1.0: public but unstable**. Within the pre-1.0
period the exported surface may change in any release — signatures
may be renamed, reshaped, or removed without a deprecation period,
and callers must expect to adjust on upgrade. The derivation rule is
the stable part: as long as an operation is declared in the table,
the client reaches it by its table name.

## Not in this rung

C ABI / c-shared / c-archive builds, WASM (wasip1), and the `unix://`
MCP transport are separate rungs of #1877 and are not part of this
package.
