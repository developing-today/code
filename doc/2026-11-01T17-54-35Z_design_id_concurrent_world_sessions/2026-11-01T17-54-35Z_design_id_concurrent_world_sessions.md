# F13.8 concurrent world sessions: client coordinator

## Request and intent

Implement the first slice of F13.8 from [issue #3845](https://github.com/developing-today/code/issues/3845), following the decision on [#790](https://github.com/developing-today/code/issues/790): one client can hold live sessions to multiple worlds and servers at once. Preserve the client-only default, existing one-shot join behavior, and per-world capability checks. A second client joining a world must not mutate that world's durable journal.

The client-only command boundary is already documented and regression-tested by [#3846](https://github.com/developing-today/code/issues/3846), merged as PR #3862; this slice does not change that boundary or start listeners implicitly.

## Initial approach

Keep `WorldClient` as the transport/session primitive for one server endpoint. Add a client-side coordinator that owns/reuses clients by explicit server endpoint and retains independently opened session handles by world. A connection is never treated as spanning distinct servers. Every session is opened through the existing capability-bearing path.

Verify the design with integration coverage for (a) two simultaneously live worlds through one server, (b) sessions through distinct server endpoints, and (c) a second client joining without changing the durable journal. Preserve existing sequential `open_session` and one-shot CLI paths.

## Boundaries and rejected alternatives

- Do not start or host a server as a side effect of connecting or opening another session; hosting remains explicit via `id serve`.
- Do not change the journal model, guest ABI, or capability authority model.
- Do not make one transport connection represent multiple servers. Use an independent `WorldClient` per endpoint.
- Keep the change limited to coordination/session ownership and tests; retain the existing command surface.

## Verification

Run the targeted Rust integration tests for concurrent sessions, multi-server routing, journal non-mutation, and capability enforcement. Confirm the existing sequential and one-shot paths remain green.

## References

- [F13.8 implementation issue #3845](https://github.com/developing-today/code/issues/3845)
- [F13.8 decision record, #790](https://github.com/developing-today/code/issues/790#issuecomment-6561066133)
- [F13.8 client-only boundary, #3846](https://github.com/developing-today/code/issues/3846)
- `.opencode/plans/phase-13-client-shells.md`, section E
