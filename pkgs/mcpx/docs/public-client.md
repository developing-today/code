# Public Go client for /v1 (`pkg/client`)

`pkgs/mcpx/pkg/client` is the public Go client for the mcpx daemon's
/v1 API (#1877, rung 1). Its calls are derived at run time from the
daemon's operation table (the `Ops` function in `internal/api`) — method, path, and
parameter placement come from the table, never from hand-written
per-operation code — so the client cannot drift from the daemon the
way a hand-maintained client would.

## The API

One client, two call shapes, and the table itself for callers who
need to know what exists before calling:

| symbol                                                                       | what it is                                                                                                                                                                                                                                                      |
| ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `New(baseURL string, opts ...Option) *Client`                                | build a client for the daemon at `baseURL`; operation paths from the table are appended to it                                                                                                                                                                   |
| `(*Client).Call(ctx, name string, args map[string]any, out any) error`       | perform the named operation; when `out` is non-nil, decode the JSON answer into it. Arguments are keyed by the operation's declared parameter names                                                                                                             |
| `(*Client).CallRaw(ctx, name string, args map[string]any) ([]byte, error)`   | perform the named operation and return the raw answer body; a non-2xx answer is an `*HTTPError`                                                                                                                                                                 |
| `Invoke[T any](ctx, c *Client, name string, args map[string]any) (T, error)` | `Call` with the answer as a return value, for callers that prefer generics over an out parameter                                                                                                                                                                |
| `Operations() []api.Op`                                                      | the daemon's /v1 operation table: every operation `Call` can reach, with its declared parameters                                                                                                                                                                |
| `Operation(name string) (api.Op, bool)`                                      | look one operation up by its table name (for example `task_get`); the second result is false for a name the table does not declare                                                                                                                              |
| `Option`                                                                     | configures a `Client`; `WithHTTPClient(hc *http.Client)` replaces the HTTP client (a unix-socket dialler, a test round-tripper, TLS), and `WithToken(token string)` sets the bearer token sent on every request, matching the daemon's `Authorization` handling |
| `ErrUnknownOperation`                                                        | returned (wrapped) when `Call` names an operation the table does not declare                                                                                                                                                                                    |
| `*ArgError`                                                                  | an argument problem, naming the operation (`Op`) and the parameter (`Param`) at fault — a missing required argument, or one the operation does not declare                                                                                                      |
| `*HTTPError`                                                                 | a non-2xx answer, carrying the response `Status` and the daemon's error `Code`                                                                                                                                                                                  |

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
