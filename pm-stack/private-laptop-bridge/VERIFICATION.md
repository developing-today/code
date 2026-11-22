# Offline verification checkpoints — 2026-11-22

## Follow-up review and private staging candidate

The next revision adds `npm run rehearse` (all bridge, Gateway, Aster, freshness,
publisher and bounded-refresh tests) and `npm run check:staging` (Wrangler bundle
dry run, never deployment). The PR records the final tested commit and results;
the original checkpoint below remains historical evidence for `0ba3dd75a`.

Concrete findings fixed in this follow-up:

* Raw malformed broker counters could normalize to zero and look idle. Bridge
  publication now requires explicit validated broker provenance and fresh,
  error-free observations. Eight integration cases prove idle versus unavailable,
  exact TTL boundaries, missing data, stale completed results and retained-file
  behavior after failed publication.
* Awaiting cancellation of a denied response could hang the device client.
  Cleanup is nonblocking; pending/rejecting cancellation regression tests pass.
* Slight clock differences rejected legitimate short leases. `serverNow` and
  conservative request-start-relative deadlines tolerate valid offsets while
  charging transport time against the lease. Regression coverage includes
  ±500 ms, ±29 seconds, delayed responses and inconsistent advertised lifetimes.
* Idle polling consumed the entire per-minute device quota. Two-second polling
  reserves capacity for results.

The disabled refresh pilot has eight deterministic scheduler tests proving
strict opt-in, cache-safe cadence, no overlap, monotonic one-hour stop, cancellation
and no rearming after a hung/late collector. The real Gateway RPC harness verifies
trusted context/identity/posture gating, direct HTTP denial, and zero upstream
continuation calls. Aster's four-command CLI has seven tests including actual
CLI → signed SDK → workerd/SQLite → laptop-client roundtrips.

Wrangler 4.166.0 successfully bundled the private staging configuration in a
local dry run. Bun 1.4.5 ran the existing queue suite: 18 passed, zero failed.
These checks do not prove real Gateway routing or AMD service behavior. The
DNS sink/on-ramp composition, exact account/device inventory and certificate
trust remain mandatory approval-gated live tests in [STAGING.md](STAGING.md).

## Original checkpoint

Implementation based on developing-today/code `ad4e69670`, refreshed by merge to
`7bfcaff23` before review. Executed as non-root UID 1000 in an isolated cloud
worktree, not on Drew's laptop. Node v24.19.0; Miniflare 4.20260730.0;
compatibility date 2026-07-30; SQLite Durable Object storage.

| Check | Result |
| --- | --- |
| Clean `npm ci` using checked-in lockfile | Passed |
| `npm test` from `pm-stack/private-laptop-bridge` | 39 passed, 0 failed (~33 s) |
| Control plane | 12 passing tests |
| Laptop client/adapters | 15 passing tests |
| Model-based authorization | 1 passing test, 50 seeds × 40 transitions |
| Real workerd acceptance + caller SDK | 11 passing tests |
| `node --test pm-stack/aggregator/test/bridge-snapshot.test.ts` | 7 passed, 0 failed |
| `git diff --check` | Passed |

The generated model achieved 150 accepted calls, 62 leases, 16 completions,
59 invalid/stale-lease denials, 72 replay denials, 4 completed reads and
231 non-owner read denials. The coverage assertions prevent vacuous success.

Real-runtime evidence includes all four named operations through the actual
producer, fixed-file adapter, client, Worker, SQLite object and caller SDK. A
synthetic Unix socket server stands in for the queue broker liveness endpoint;
its assertions permit only GET /healthz with no broker token. Synthetic keypairs
exist only in memory. Temporary snapshots/databases are removed after tests;
Worker outbound I/O is explicitly rejected by the harness. Restart tests use the
same temporary persistent SQLite store and reject an already accepted nonce.
The disconnect test waits for the actual 15-second lease boundary, obtains a
new lease, refuses the stale completion and accepts the new completion.

Review found and fixed an inherited-object identity-name edge case, explicit
lease deadline/ack validation, queue freshness at completion/retrieval, and audit
retention/alarm scheduling. The final independent security review found no
blocking issue within this offline scope. This is engineering test evidence,
not a security certification.

## Not verified / not performed

* No Cloudflare deployment, routing, public management endpoint or identity
  enrollment; no live key or secret creation/transfer.
* No AMD connection, NixOS service installation/network/security change, or
  Desktop Commander/native Codex modification.
* No actual Aster connector call, private ingress validation, live queue parity,
  observed workflow inventory, production revocation propagation or cutover.
* No full aggregator Bun suite/typecheck or real systemd sandbox validation.
* No automatic PR CI claim. Current repo policy disables it; focused local
  offline execution was explicitly requested for this task.

Follow the README's rollout/rollback gates. Keep the PR a draft and keep the
fallback until owner-approved end-to-end laptop acceptance is complete.
