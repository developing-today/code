# Secure public web router constructors

**Status:** Initial design, before implementation (2026-11-07).

## Request and intent

Issue [#5747](https://github.com/developing-today/code/issues/5747) reports
that public router builders can return routes without the host, origin, and
optional token guard used by `id serve`. The issue names `create_router` and
`create_router_for_app`; source inspection also found the public
`create_program_router` bypass. An embedder must not accidentally receive an
unguarded router from any of these public constructors.

## Initial design

- Require an explicit `WebSecurity` argument on each public `create_*_router`
  builder. The public builder applies the guard before returning its router.
- Keep the route-tree builders private. Do not re-export a raw builder as the
  default embedding API.
- Centralize guard application so each built router receives exactly one
  security layer. `finish_router` will continue setting world and cookie state,
  then call the corresponding guarded constructor rather than adding a second
  layer itself.
- Preserve the guard's existing host/origin/token response behavior and the
  `cmd_serve` listener-specific security configuration.
- Document the source-compatible migration: pass an explicit `WebSecurity`
  when calling the changed constructors. Callers should choose a policy
  appropriate to their bind and must not rely on `WebSecurity::default()` as an
  authentication policy.

## Verification plan

Add integration coverage through the public API for each exported constructor:
requests without credentials and with a wrong token must receive 401, while a
cross-origin request must receive 403. Cover the program-only router as well as
the explorer builders. Exercise the normal serve router with a valid bootstrap
request to ensure it keeps its existing redirect/cookie response and does not
apply the guard twice. Run the focused integration and security tests, then
review the actual PR checks under `pkgs/id/AGENTS.md` and `.github/ci-policy.md`.

## Change log

- 2026-11-07: Recorded the initial API and single-guard design before code
  changes.

---

## 2026-11-07T04-52-01Z Update: Guard the public constructors

Implementation follows the initial design. `create_router`,
`create_router_for_app`, and `create_program_router` now require an explicit
`WebSecurity`; the last constructor was added to scope after source inspection
found it was another public bypass. A shared `security::protect` helper wraps
each completed route tree. `finish_router` sets world/cookie state and delegates
to those guarded constructors without adding a second layer.

The migration is documented on the changed APIs: callers pass their listener's
`WebSecurity`, and `WebSecurity::default()` is explicitly noted as not requiring
a token or restricting hosts. The integration test covers all six public
router entry points, missing and incorrect credentials, cross-origin rejection,
the existing GET query-token redirect/cookie behavior, and one-cookie behavior
for a non-GET bootstrap. The focused integration test, 17 security unit tests,
and three program-router regression tests pass.
