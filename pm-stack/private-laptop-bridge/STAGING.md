# Private staging and Aster pilot — approval packet

Prepared 2026-11-22. **Not deployed, enrolled, enabled, or replacement-ready.**
This is a bounded one-hour proposal, not authorization. Resource identifiers,
verified owner email, exact runtime/store paths and the UTC pilot window must be
filled from read-only inventory and approved before execution. No secrets belong
in this document. Desktop Commander remains installed and usable.

## Selected ingress and the remaining feasibility gate

Use **Cloudflare Gateway Forward**, not a public Worker URL. The official guide
supports a Worker with no public route, `workers_dev:false`,
`preview_urls:false`, invoked through `GatewayMiddleware.handle()`; HTTPS needs
Gateway HTTP filtering, TLS decryption and a trusted root. It can return a
synthetic response without contacting an origin. This capability is in public
beta on all Zero Trust plans. WebSocket and CONNECT traffic bypass this mechanism;
this bridge uses only ordinary HTTPS POSTs. Invocation failures fail closed.
[Cloudflare Forward guide](https://developers.cloudflare.com/cloudflare-one/traffic-controls/integrate-workers/forward-traffic/)

Proposed path:

```text
Aster's approved isolated caller runner                  AMD non-root client
  independently held caller signing key                  device signing key
             |                                             |
             +---- outbound HTTPS through scoped Gateway ---+
                                      |
                        GatewayMiddleware RPC entrypoint
                        (no next(), no origin forwarding)
                                      |
                         existing bridge Durable Object
                         independent request signature checks
```

Workers VPC connects Workers **to** private services. It does not itself let the
laptop reach a private Worker. Service bindings likewise do not supply a laptop
transport. No Workers VPC binding, Tunnel, public route, or generic network proxy
is proposed. [Workers VPC services](https://developers.cloudflare.com/workers-vpc/configuration/vpc-services/)

**DNS/on-ramp gate is unresolved, not papered over.** Proposed host is
`private-bridge-staging.security.cab`, subject to confirming owner control and
absence of an existing record/wildcard route. Do not point it at `pm.security.cab`
or any real application. The intended arrangement has no public A/AAAA/CNAME and
an exact-host Gateway DNS override to the non-serving documentation address
`192.0.2.1` (proposed, not yet tested), with a 60-second DNS TTL where supported.
No server or tunnel is attached to that address; confirm no local route repurposes it.
Gateway documents custom DNS override answers; it does not establish that a
reserved sink address passes every pre-HTTP proxy check. The combined path needs
a credential-free live smoke before any real bridge keys or Fleet data are used.
[DNS override](https://developers.cloudflare.com/cloudflare-one/traffic-controls/dns-policies/)

The smoke must show the Forward entrypoint returns a fixed synthetic response
without an origin connection. Then remove/disable the Forward rule and test
again, plus bypass Gateway entirely: both paths must fail without reaching an
application or exposing a request body. Test A and AAAA behavior, cached answers,
wrong hostname, alternate port, CONNECT and WebSocket. If the sink arrangement
is rejected or cannot prove that property, **stop the pilot** and review a new
transport; do not substitute a public authenticated management endpoint. The
current code can be reviewed/deployed disabled, but is not yet an approved,
operational private transport.

There is no evidence that AMD or the current Aster Work executor is already
Gateway-enrolled. The Work executor must not be assumed to support a persistent
VPN client or root certificate installation. A separate owner-controlled isolated
caller runner is a prerequisite if the existing executor cannot provide the
approved on-ramp. Cloudflare's clientless proxy alternatives are not a proven
shortcut here: identity proxy setup uses interactive authorization, while
source-IP proxy endpoints require Enterprise. They also need review for broader
private-network reach; neither is selected for this pilot.
[Proxy endpoints](https://developers.cloudflare.com/cloudflare-one/networks/resolvers-and-proxies/proxy-endpoints/)

A lower-priority HTTP Block is **not** a fallback for Forward: Gateway evaluates
request Blocks before Forward, so a matching Block prevents invocation. Do not
add an always-matching Block and expect ordering to fix it. A missing Forward
policy is different from a Worker error: traffic can proceed normally, making
the non-serving sink and bypass smoke essential.

## Exact proposed resources and authority

| Resource | Proposed permission / constraint |
| --- | --- |
| Cloudflare account | Existing owner account, ID verified before approval; no account-wide policy changes |
| Worker | `private-laptop-bridge-staging`, no routes/custom domains/public previews; default disabled |
| Storage | One new SQLite-backed `PrivateLaptopBridge` Durable Object namespace; only bounded RPC state/audit |
| Gateway rules | DNS rule `private-laptop-bridge-pilot-dns` and HTTP rule `private-laptop-bridge-one-hour-pilot`, initially disabled; owner-email and approved device-posture restriction; inspect only pilot host traffic |
| Network enrollment | Only AMD and a separately identified caller runner; dedicated test profile; preserve Tailscale and all existing routing |
| TLS trust | Only the two dedicated bridge Node processes; approved Gateway root public certificate, fingerprint verified; no global OS/browser trust installation |
| Device principal | `amd-pilot`, role `device`, device `amd`, public key only at Worker |
| Caller principal | `aster-pilot`, role `caller`, device `amd`, a different public key |
| Signature audience | `fleet-private-bridge-staging-v1`, unique to this staging instance |
| Bridge scopes | `fleet.queue.list`, `fleet.queue.health`, `laptop.health`, `laptop.diagnostics` only |
| Lifetime | Both principals issued at pilot start and expire no later than start + 3,600,000 ms; no renewal |
| AMD local reads | Fixed sanitized queue snapshot and fixed credential-free health adapter inputs only; no queue/secrets token |
| AMD process | New manually started user unit `private-laptop-bridge-pilot.service`, no enablement/linger; `RuntimeMaxSec=3600`, `Restart=no` |
| Local configuration | AMD `%h/.config/private-laptop-bridge/device.json`; caller `%h/.config/private-laptop-bridge/aster.json`; owner-only files, independently provisioned keys |
| Public CA file | `%h/.config/private-laptop-bridge/gateway-ca.pem` on each approved runner, fingerprint checked; per-process use only |
| Data destination | This staging Worker/DO and authenticated caller only; no log-body capture, external telemetry export or shared artifacts |

The Gateway entrypoint should pin account ID, Forward policy ID, hostname, and
verified owner email, denying unavailable identity. The context reference exposes
`accountId`, `policyId`, `host`, `uri`, and an async `identity()` returning an
optional email; it does **not** expose a documented raw device ID. A required
posture check is matched by UUID in `devicePosture().checks.passed`; absent or
failed lookup must deny. Gateway identity is an additional gate, never a
replacement for the independently pinned caller/device signing keys.
[Available context](https://developers.cloudflare.com/cloudflare-one/traffic-controls/integrate-workers/available-context/)

Deploy using the owner's existing approved short-lived administrative mechanism.
Runtime principals need **no Cloudflare API permissions**. The deployment actor
needs Workers script deployment permissions for the chosen account; an owner
administrator separately applies the exact reviewed Gateway rule changes. The
Gateway rule API lists `Zero Trust Write` as an accepted permission: this is broad
administrative power, not a credential to install on AMD or the caller. Prefer
owner-applied dashboard changes or verified resource-scoped administration over
minting a general automation token. No DNS-zone write permission is needed for
the proposed Gateway-only override. No Access service token or Tunnel token is
needed. [Gateway rule API](https://developers.cloudflare.com/api/go/resources/zero_trust/subresources/gateway/subresources/rules/methods/create/)

Review-only Forward request shape (not executed; placeholders must be resolved,
precedence compared against existing rules, then the exact diff approved):

```json
{
  "name": "private-laptop-bridge-one-hour-pilot",
  "enabled": false,
  "precedence": 1000,
  "action": "forward",
  "filters": ["http"],
  "traffic": "http.request.host == \"private-bridge-staging.security.cab\" and http.request.method == \"POST\" and http.request.uri.path in {\"/v1/call\" \"/v1/read\" \"/v1/poll\" \"/v1/result\"}",
  "identity": "identity.email == \"VERIFIED_OWNER_EMAIL\"",
  "device_posture": "any(device_posture.checks.passed[*] in {\"APPROVED_PILOT_POSTURE_UUID\"})",
  "rule_settings": {"forward": {"worker_id": "ACTUAL_32_HEX_WORKER_ID"}}
}
```

Validate expression syntax using the account's policy builder/API before
activation. This skeleton is not proof of a saved rule. An HTTP rule does not
support the DNS rule `expiration` facility; use principal expiry and client
process lifetime for the hard stop, then explicitly remove pilot routing.
[HTTP selectors](https://developers.cloudflare.com/cloudflare-one/traffic-controls/http-policies/)

## TLS and laptop install boundaries

`NODE_EXTRA_CA_CERTS` can extend trust for a single launched Node process with a
PEM certificate file; it is read on startup. This does not restrict the trusted
CA to one hostname. Therefore use a bridge-only process, pinned origin, no generic
fetch tool and no subprocesses. Do not set it globally or use
`NODE_TLS_REJECT_UNAUTHORIZED=0`. Confirm behavior with the pinned Nix Node build
and certificate fingerprint. [Node CLI certificate settings](https://nodejs.org/api/cli.html#node_extra_ca_certsfile)

Gateway profile enrollment/routing remains a separate network/security change
requiring approval even with per-process trust. A new dedicated two-device pilot profile that only routes
and decrypts this host must be proven before AMD participation. Do not install a
tenant-wide catch-all Do Not Inspect exception or disable existing inspection
policies. Review the existing on-ramp, device-profile assignment, DNS forwarding,
HTTP inspection and Tailscale coexistence together; exact-host HTTP rules alone
do not prove that other traffic is excluded from TLS interception. If the available
on-ramp requires broad TLS interception or changing existing Tailscale paths,
that exceeds this proposal and is a blocker.

After approval, install only the reviewed commit's client/protocol/adapters and a
pinned Node runtime; review the exact Nix store paths. Owner provisions separate
signing material locally on each host using a separately approved procedure;
private halves never go to Cloudflare, the other host, chat, or the repository.
The existing client accepts a private owner-only config file; no auto-key-creation
or pairing flow exists. Review and fill the disabled unit example, add a
one-hour `RuntimeMaxSec=3600`, keep `Restart=no`, and start manually without enable,
linger, cron, timer, or an `[Install]` section. The optional extra-CA file is public
certificate data, not a signing key, and must also be explicitly allowed by any
mount isolation.

Separately approve both `PM_AGG_BRIDGE_SNAPSHOT=1` and
`PM_AGG_BRIDGE_REFRESH=1` for the existing aggregator. The new, default-disabled
process interval reuses its existing collector: immediate read, then 65 seconds
after completion, never overlapping, at most 57 starts, with an independent
one-hour stop. It is registered as proposed/disabled in the Fleet schedule
registry. An in-flight collector may complete and publish its actual observation
after that stop, but cannot start another refresh. A restart with both flags set
starts another pilot, so remove both flags afterward. No external timer is added.
Verify actual home/path permissions and that the client reads the same file.
Only explicitly verified broker data can publish a successful snapshot; malformed
envelopes or missing counters cannot become zero/idle. Known failures publish an
unavailable marker; failed publication can preserve the prior observation only
until its original five-minute TTL. It never refreshes that observation's timestamp.

## Aster's concrete caller path

For the first pilot, Aster uses an **already authorized executor** on the approved
isolated runner to invoke `aster.mjs` with its own `aster-pilot` identity.
That executor's existence, owner and Gateway capability must be verified; current
Work access alone establishes none of them. The SDK can submit an allowlisted
operation and wait/read its accepted handle; it cannot enroll, dispatch, or run a
host command. The wrapper accepts only these forms (after approval, with the
reviewed Node/runtime paths and per-process CA environment):

```sh
node aster.mjs --config /APPROVED_HOME/.config/private-laptop-bridge/aster.json queue-list --limit 10 --status blocked
node aster.mjs --config /APPROVED_HOME/.config/private-laptop-bridge/aster.json queue-health
node aster.mjs --config /APPROVED_HOME/.config/private-laptop-bridge/aster.json laptop-health
node aster.mjs --config /APPROVED_HOME/.config/private-laptop-bridge/aster.json laptop-diagnostics
```

It emits bounded `accepted` then `complete` JSON records or a fixed error code.
Use the SDK to resume a known accepted handle and preserve
uncertain submission outcomes rather than automatically re-submitting.

If Aster cannot reach that runner with an existing authorized execution surface,
implementation is blocked pending a specific integration decision. A private
custom MCP connector could expose only these four SDK operations, keep the caller
key on the runner, and validate the same input/output schemas. That connector
requires its own reviewed authentication, hosting and installation approval; it is
not installed or implemented here. Do not install a generic shell connector,
reuse Desktop Commander as the new caller path, or put a key in a public MCP URL.

## One-hour acceptance and rollback

1. Before the clock starts: approve exact commit, account, host, identity/public
   keys, network/profile/CA changes, local paths, resource diff and zero-incremental
   budget. Complete the credential-free DNS/Forward smoke above.
2. Minute 0–10: enable staging registry and routing, start caller and AMD client,
   observe fresh snapshot and one successful result for each scope. Record only
   operation, timestamp, outcome and bounded scalar verification evidence.
3. Minute 10–30: Aster independently repeats each read; compare counters/task IDs
   to the approved local source. Verify partial lists say partial. Record which
   actual Commander workflow each covers and which remains native Codex work.
4. Minute 30–45: wrong key/role/device/audience, denied operation, stale snapshot,
   replay, quota, reconnect/expired lease, missing Gateway identity and disabled
   Forward path all fail as expected; no origin receives payloads.
5. Minute 45–55: revoke caller then device independently and verify stored result
   retrieval, submit, poll and completion denial. Observe active configuration,
   not just deployment acknowledgement. Demonstrate rollback.
6. By minute 60: expiry closes all authorized requests, clients stop; owner disables
   the bridge and both pilot policies, removes only new enrollment/profile/trust
   changes, restores aggregator setting and stops the new user unit. Remove
   Forward policy before deleting its Worker; remove the DNS override, wait its
   observed TTL and flush only pilot-client DNS caches; preserve the minimal audit receipt.
   Dispose of pilot private keys locally under the approved process.

No automatic cutover follows a successful pilot. Retiring Commander requires a
separate owner-approved parity record and native Codex verification.

## Cost and capability assumptions (fetched 2026-11-22)

Proposed incremental budget: **$0; no plan upgrade**. Confirm remaining shared
account allowances before approval. Cloudflare lists Zero Trust Free at $0 for
50 users; paid PAYG is $7/user/month. Two pilot devices do not establish that the
account has available seats. [Cloudflare pricing](https://www.cloudflare.com/plans/)

Workers Free includes 100,000 requests/day with a 10 ms CPU limit per invocation;
Workers Paid has a $5 monthly subscription. Do not upgrade automatically if the
signature path hits CPU limits. [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)

SQLite Durable Objects are available on Free: 100,000 requests/day, 13,000 GB-s/day,
5 million row reads/day, 100,000 row writes/day and 5 GB stored. Exceeding free
limits can make requests fail; the pilot must not treat that as an auth bypass.
[Durable Objects pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/)

One client polling every two seconds produces at most 1,800 idle polls/hour plus
result and caller traffic. Limit the pilot to 60 submitted reads. With prompt
completion this is roughly 2,100 ingress requests, 2,100 DO requests, plus alarm
invocations. Reserve at least 10,000 requests of remaining allowance **in each**
Worker and DO category; they are separately metered, and alarm invocations also
count. Slow/disconnected callers may consume more polling requests. Verify actual
storage operations/CPU in staging rather than equating an HTTP request with one
SQL row operation. This estimate is workload arithmetic, not a bill or
quota reservation. Stop if existing usage leaves insufficient headroom. No
storage/Logpush/paid retention add-on is proposed.

The optional refresh pilot starts at most 57 existing collector passes per hour.
Each pass performs one existing broker health read plus one bounded list read per
reported queue; this is local broker/Turso workload, not a new direct Cloudflare
or Turso credential. Account/API allowances and the actual number of queues must
be checked before enabling it. Free-plan limits are capacity limits, not a
guaranteed reserved budget; no $0 claim is valid if the account is already paid
and incurs incremental overages. The owner must approve the account-specific cap.

## Exact actions still requiring approval

The code/config is buildable offline; this packet does not authorize any of:
deploying the Worker/SQLite namespace; creating/enabling the two Gateway policies;
enrolling either device or changing its routing/DNS/TLS trust; provisioning keys
or registering their public halves; installing/starting the AMD unit; changing
aggregator flags; giving Aster a new executor/connector; removing pilot credentials
or infrastructure; or retiring Desktop Commander. Resolve real account/rule/posture
IDs, verified email, runner ownership, Nix paths, certificate fingerprint and
pilot UTC window into a final diff first. Seek approval for that exact diff,
starting with the credential-free network smoke; do not request blanket access.
