# Private laptop bridge — offline candidate

**Not deployed; not a replacement-ready cutover.** Desktop Commander remains the
fallback. [The staging approval packet](STAGING.md) specifies private Gateway
ingress, resources, identities, costs, installation and rollback. This package
implements bounded Fleet reads, not a shell. Aster still
needs an approved caller integration, a privately reachable control plane, and
verified enrollment on Drew's AMD NixOS laptop before relying on it.

## Permission boundary

The caller submits one named read. The laptop makes an outbound poll, executes a
fixed local adapter, and returns a projected result. The Cloudflare Durable
Object holds only short-lived RPC requests/results; the Fleet queue remains the
existing Turso-backed queue. No endpoint claims, dispatches, changes tasks,
restarts services, reads arbitrary files, fetches arbitrary URLs, executes a
command, or accepts scripts. Builds/tests are deliberately outside this version.

The Worker is default-disabled, has no routes, workers.dev or preview URLs, and
contains no private keys. Device and caller identities are independently pinned
public keys with separate roles, device binding, operation scopes, expiration and
revocation. Possession of a device key does not grant caller permissions. A
request signature covers its audience, endpoint, method, timestamp, nonce and
exact body. Durable replay checks, short leases, quotas and strict output schemas
apply before a result becomes visible to its requesting caller.

Only synthetic, memory-only test keys are created by tests. No credential is
created, transferred or enrolled by installation code. Runtime signing material
must be provisioned by an independently approved owner procedure. The existing
queue broker credential is never loaded by this bridge.

## Existing components and decisions

Inspected main at `ad4e6967008968b2f28334ddd89d8de24af4ba73` (2026-11-22),
including newly merged [#12505](https://github.com/developing-today/code/pull/12505).

* `pm-stack/console/edge` demonstrates Workers/Durable Object packaging and an
  outbound host sync. Its tunnel fallback, laptop-driven authority bootstrap,
  persistent key auto-generation, and held write actions are unsuitable here.
  This bridge is separate and fails closed without a fallback proxy.
* `pkgs/fleet/crates/fleet-queue/src/queue_broker.rs` uses the same
  `X-Agent-Token` for all POST verbs on TCP **and** Unix sockets. The comment in
  `pm-stack/aggregator/src/queue.ts` describing a read-capable token does not
  establish a restricted credential. The client therefore never calls those
  authenticated endpoints or reads their token file.
* The existing aggregator already reads `/health` and `/list`. An opt-in
  sanitized snapshot producer supplies queue metadata to this bridge without
  adding another credential consumer. Publication follows the existing collector;
  a separately gated one-hour refresh pilot can maintain its cadence. Unrefreshed
  data expires, and raw broker provenance must be verified before zeros count as
  an observed idle queue.
* `pkgs/laptop-agent/src/laptop_agent.py` exposes general `/exec`, `/jobs` and
  `/file`. None is called or proxied here.
* `home/common/default.nix` declares `desktop-commander-remote`; this change does
  not modify it, its saved authorization, native Codex, or laptop networking.

Work inspected while open: [#12312](https://github.com/developing-today/code/pull/12312)
(mcpx laptop SSH), [#12226](https://github.com/developing-today/code/pull/12226)
(health hooks), [#12299](https://github.com/developing-today/code/pull/12299)
(OpenCode restart behavior), [#12500](https://github.com/developing-today/code/pull/12500)
(Fleet credential cache parsing). Those are not dependencies or deployed by this
package. #12500 subsequently landed and was included when updating the base to
`7bfcaff23`; no bridge credential flow reuses the cache daemon.

## Workflow coverage and native Codex

| Workflow | This bridge | Native Codex / remaining evidence |
| --- | --- | --- |
| Bounded queue task status | Sanitized snapshot, numeric IDs and statuses; honest partial/stale reporting | Detailed payloads, titles and bound sessions excluded |
| Queue counters | Sanitized existing collector counters | Live broker parity still needs laptop verification |
| Laptop health | Fixed credential-free local health observation | No service changes |
| Bounded diagnostics | Strict scalar/categorical projection | No environment, logs or arbitrary files |
| Repository exploration, editing, builds, test development, interactive debugging | No permission | Aster's separate native Codex setup |
| Arbitrary shell / filesystem operations | No permission | Native Codex with its own authorization; never add as a bridge fallback |
| Dispatch, enqueue, cancel, service restart, deployment | No permission | Separate explicit permission review required |

Repository sources establish Commander capability, **not an observed inventory of
Aster's actual Commander calls**. Available conversation history only describes
proposed Fleet reads and a prior reachability check. Do not call this workflow
parity or retire Commander based on synthetic tests. During the authorized pilot,
record actual operation names (not command output/secrets), map each to a bridge
read or native Codex, and verify unsupported essential workflows have a working
replacement.

## Rollout gates — nothing below has been applied

1. Review this exact commit, schemas, local data permissions and security tests.
   Keep all production deployment automation absent. Confirm snapshot contents
   expose only the requested metadata and truncation/freshness semantics.
2. Drew must approve the **specific** caller identity/scopes, device identity,
   signing-key provisioning and lifetime, private ingress design/hostname,
   Cloudflare account/resources/cost cap, permitted local paths/socket, and
   non-root service configuration. This package does not choose a public gateway.
   A service binding alone is not a laptop-accessible transport: the authorized
   private caller/device ingress remains a required integration decision.
3. In approved isolated staging, enroll separate short-lived identities through
   an owner-controlled channel. Never accept a device-reported caller key or an
   unauthenticated key-discovery endpoint. Prove wrong-role, wrong-device,
   wrong-audience, expired and revoked identities fail at every stage.
4. Explicitly approve enabling `PM_AGG_BRIDGE_SNAPSHOT=1` and, for the bounded
   refresh pilot, `PM_AGG_BRIDGE_REFRESH=1` on the existing
   aggregator and installing the **disabled example** non-root bridge unit.
   Confirm actual permissions, Nix Node path, fixed snapshot location and local
   health socket. Observe a fresh real snapshot; then simulate stale/missing
   snapshots and prove they cannot report healthy/empty success.
5. Start the client manually for the approved pilot window. Demonstrate Aster
   submitting and receiving each authorized read end to end on AMD. Disconnect
   the laptop, reconnect after lease loss, and prove the old lease cannot complete.
   Check bounded output, quota exhaustion and no secret data in audit records.
6. Revoke both identities independently and demonstrate denial of submit, poll,
   completion and stored-result retrieval. Registry changes via deployment are
   not instantaneous: confirm the active configuration and observed denials;
   do not claim revocation completed just because deployment was requested.
7. Confirm native Codex covers every required non-bridge operation. Only after
   owner acceptance of the observed parity and rollback rehearsal may a separate
   change propose retiring Desktop Commander. Its credentials/service must remain
   intact throughout this implementation and pilot.

## Rollback

Disable the bridge first (`ENABLED` not `true`), revoke the caller/device entries,
and stop the new user service. Remove private ingress/service bindings only under
the same approved change procedure. Remove both aggregator opt-in flags;
remaining snapshots age out and are never treated as fresh indefinitely. Short
RPC/result lifetimes bound recoverable data; explicit deletion of cloud resources
or credential material needs its own approved cleanup. Resume Fleet through the
unchanged Desktop Commander/native Codex paths. Never silently forward a failed
bridge request to either general-purpose agent.

## Runtime references

Cloudflare documents [Durable Object state](https://developers.cloudflare.com/durable-objects/api/state/)
and [explicit preview URL controls](https://developers.cloudflare.com/changelog/post/2025-09-17-update-preview-url-setting/).
Runtime tests must use the deployed Worker entry and real Durable Object storage,
not only a JavaScript storage double. Offline success does not establish private
network reachability, actual NixOS sandbox compatibility, caller connector support,
or production revocation propagation.

## Fixed interface and limits

| Operation | Arguments | Result data |
| --- | --- | --- |
| `fleet.queue.list` | `{limit: 1..20, status?: enum}` | At most `limit` task IDs/statuses, `complete:false`, `total:null`, `fetchedAt` |
| `fleet.queue.health` | `{}` | Eight numeric status counters and `fetchedAt` |
| `laptop.health` | `{}` | `queueBroker: "up"` or `"down"`; liveness only, not Turso readiness |
| `laptop.diagnostics` | `{}` | `uptimeSeconds`, `load1`, `memoryFreeBytes` (OS free memory, not Linux MemAvailable) |

Status enum: `queued`, `claimed`, `parked`, `handed_off`, `blocked`, `done`,
`cancelled`, `failed`. Extra fields are rejected. Unavailable/invalid/timed-out
local sources return a fixed error enum; raw exception text never crosses the
bridge. Both the device and control plane validate results. Queue source age is
checked at execution, completion and retrieval; a stale successful result cannot
be read as current. An unknown queue total is deliberately not fabricated from
counters collected in a separate query.

The signed POST routes are `/v1/call`, `/v1/poll`, `/v1/result`, `/v1/read`.
There is no administrative, enrollment or proxy route. The public-key registry
is deployment configuration, not writable by a caller/device. Each principal
has `id`, `role`, `device`, `publicKey` (raw Ed25519 base64url), `scopes`,
`issuedAt`, `expiresAt` (milliseconds), and `revoked`; the registry also has a
unique `audience`. A registry with shared keys, unsupported fields or enrollment
longer than one hour fails closed. The default configuration contains no registry
and `ENABLED=false`.

Request freshness is 30 seconds; bodies are capped at 8 KiB with a three-second
read deadline. There are at most eight retained RPCs, 256 live nonces, 60
accepted-signature requests per principal per minute, and 64 recent audit rows.
Requests expire after 60 seconds, leases after at most 15 seconds, and retained
RPC state after 120 seconds. Alarms purge expired storage; failed storage never
falls back to memory. Reconnect can repeat only an allowed read and receives a
new lease; the previous lease cannot commit. All accepted-signature attempts,
including replay and invalid arguments, consume quota. Unknown/invalid signatures
produce constant denial log records; they cannot create identity-specific state.

Accepted calls and leased jobs include `serverNow`. The clients convert the
advertised server-relative lifetime to a conservative local deadline starting at
request send time, so network transit consumes the budget and ±30-second clock
offset does not extend a lease. Caller handle `expiresAt` is therefore on the
caller runner's clock, not server UTC. Queue observation timestamps remain source
timestamps; a future source timestamp is refused by the caller rather than shown
as a successful current observation. Idle polling is two seconds, leaving room
within the device quota for result submissions.

Audit stores only timestamp, validated principal/role, fixed route and status.
It stores no bodies, results, paths, private keys or exception messages. Denial
console records contain fixed event/status/code fields. Cloudflare log retention,
ingress rate limits, deployment access and spending ceilings must be set during
the approved infrastructure review; application quotas do not prevent an
unauthenticated flood from incurring ingress/runtime costs. The authority boundary
assumes trusted reviewed Worker code, trusted non-root local account, and an
independently secured private ingress. A malicious same-UID process is outside
this boundary; the service example's read-only home is not full filesystem secrecy.

## Offline verification

Requires Node 24+. From this directory:

```sh
npm ci
npm run rehearse
npm run check:staging
```

`runtime.test.mjs` executes the actual Worker with Miniflare/workerd and SQLite
Durable Object storage, including persistence across restart. Outbound network
access from that Worker is rejected by the harness. Keys exist only in memory;
local temporary database fixtures are removed after tests. The unit/property and
state-machine tests complement this runtime evidence. No test deploys a Worker,
connects to AMD, reads live credentials or installs a service.

`check:staging` runs the pinned `cf` Build Output v1 build, Versions dry run, and effective payload validator, not a deployment. `cloudflare.config.ts` in `bridge` mode is the sole real staging lifecycle definition; `wrangler.staging.jsonc` and `wrangler.jsonc` are parity fixtures tested against it, never release inputs. The smoke Worker is owned by the separate smoke init/upload workflows on `main` and is not selectable in the bridge release workflow. The deployed
candidate's default HTTP handler always denies; only the Gateway RPC entrypoint
can reach the signed bridge, after trusted-context checks. The real Gateway DNS,
device enrollment and TLS-trust path still requires the approval packet's smoke.

No automatic PR CI or active schedule is added. The merged smoke init/upload
workflows are manual; this branch retains a separate manual bridge release path
that fails closed on missing owner proof. The optional process interval remains
proposed/disabled. The repository disables automatic PR CI, so local checks are
not hosted CI evidence. Full aggregator/Bun verification and NixOS service
sandbox validation remain separate from the focused snapshot tests.

## Caller integration checkpoint

`caller.mjs` exports `createCaller({origin, audience, principal, device, privateKey})`.
An approved caller supplies its own Ed25519 CryptoKey and fixed HTTPS origin; the
SDK does not load files, discover credentials, enroll identities, or install an
Aster connector. Its four methods return accepted handles. `read(handle)` checks
once; `wait(handle)` polls for at most the request lifetime. A failed read retains
the handle for explicit resumption. An uncertain submit is never retried
automatically because its acknowledgement may have been lost after acceptance.

```js
const handle = await fleet.fleetQueueList({ limit: 10, status: 'blocked' });
const observation = await fleet.wait(handle);
```

The device CLI accepts only `--config ABSOLUTE_PATH` or `--help`. The approved
configuration must be an owner-only regular file containing exactly `origin`,
`audience`, `principal`, and an externally provisioned Ed25519 private JWK
(`kty`, `crv`, `x`, `d`). No example contains key material. The CLI refuses root,
follows no redirects, stops on authorization denial, and reconnects with bounded
backoff after transport failure. The supplied service file is a review template
with placeholder executable paths and no installation target.

Queue publication alone remains demand-driven. The separately approved refresh
pilot requires **both** literal flags, reads immediately then 65 seconds after
completion (beyond the 60-second cache), never overlaps, and stops scheduling
after one hour. It uses a monotonic clock and unref'd timers. An already running
read may finish after the stop; it cannot schedule another. Removing both flags
prevents restart from beginning another pilot. No successful read means no new
source timestamp: after five minutes the client reports unavailable, never idle.
Fresh explicit zero counters are an observed idle queue; an empty partial task
list still has `complete:false,total:null` and proves no global empty total.

`aster.mjs` supplies the four strictly parsed commands in STAGING.md for an
approved non-root caller runner. Its end-to-end test exercises actual CLI parsing,
signing, Worker/SQLite execution, the laptop client, and bounded JSON output. No
Aster connector or runner access is installed by this package.


The stateless smoke Worker and private Durable Object bridge have separate protected GitHub Actions targets using the pinned `cf` Build Output v1 and Versions API flow. Uploads create immutable versions; a separate exact-UUID promotion performs a 100% deployment. Each environment token must be scoped to its one pre-existing Worker. The workflow refuses bootstrap and never changes Gateway policies. See [DISCOVERY.md](DISCOVERY.md) for exact staging prerequisites and recovery.
