# Public Go client for /v1 (`pkg/client`)

`pkgs/mcpx/pkg/client` is the public Go client for the mcpx daemon's
/v1 API (#1877, rung 1). Its calls are derived at run time from the
daemon's operation table (`internal/api.Ops`) — method, path, and
parameter placement come from the table, never from hand-written
per-operation code — so the client cannot drift from the daemon the
way a hand-maintained client would.

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
