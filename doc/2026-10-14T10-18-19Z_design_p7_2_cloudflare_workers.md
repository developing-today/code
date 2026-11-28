# P7.2 design: Cloudflare Workers pkarr option

- Date: 2026-10-14
- Status: **design complete; implementation deferred**. This is a design-only decision record. No Worker, account, DNS zone, or Cloudflare resource was created or accessed.
- Roadmap: [Phase 7 connectivity](./2026-10-12T21-22-53Z_phase_7_connectivity/2026-10-12T21-22-53Z_phase_7_connectivity.md), [phased plan P7.2](./2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md).
- Evidence baseline: [Cloudflare Workers round-2 research](./2026-10-11T11-08-42Z_design_remaining_work/research-round2/cloudflare-workers/RESULTS.md), read 2026-10-11. Its labels **[verified]**, **[inferred]**, and **[not checked]** are retained below. Cloudflare limits and prices are time-sensitive; re-check before implementation.

## Decision and recommendation

Design the option as an **opt-in, standalone pkarr HTTP relay plus a direct DoH endpoint**, implemented as a Worker front end with one SQLite-backed Durable Object per public key. It stores signed pkarr packets; it is not a Mainline DHT node, a general DNS server, or an iroh relay. Support only:

1. `PUT /pkarr/<z32>` and `GET /pkarr/<z32>` for signed pkarr relay payloads.
2. `POST /dns-query` using `application/dns-message` for DNS-over-HTTPS responses to `_iroh.<z32>.<configured-origin>` TXT lookups.

Do not serve UDP/TCP DNS, Mainline DHT, WebSockets, iroh relay traffic, arbitrary HTTP proxying, or normal recursive DNS/NS delegation. Return explicit `404`/`405` for other paths and methods. Keep n0 as the product default; opting into this Worker replaces the endpoint's pkarr publisher/resolver and DNS lookup origin as a matched configuration. Do not silently publish to one service and resolve from another.

**Recommendation:** proceed to implementation only when the owner wants an independent, private-to-this-service lookup namespace. The Worker adds no NAT traversal or relay capacity; it cannot replace n0 for QAD or relay traffic. The design does not require an owner decision to record these high-touch choices: this note chooses public read access to signed records, no account/auth layer, paid Workers/DO capacity, no DHT replication, and opt-in only. Revisit the choices listed under [Open decisions](#open-decisions) before deployment.

## Trust boundary and threat model

### Boundary

- **[Verified]** The pkarr payload is signed by the endpoint key. iroh checks returned payload signatures client-side. The relay can validate the signature and packet shape but cannot forge a valid endpoint update without the endpoint's private key. See research [§D, iroh-side pkarr](./2026-10-11T11-08-42Z_design_remaining_work/research-round2/cloudflare-workers/RESULTS.md#d-open-source-pkarr-relays-and-the-iroh-dns-server).
- **[Inferred]** Cloudflare terminates HTTPS and operates the Worker and Durable Object platform. Cloudflare can observe client IPs, requested z32 keys, update/lookup timing, DNS questions, and plaintext request/response metadata and content. Signed records are integrity-protected, not confidential. Treat Cloudflare and its account/operator boundary as trusted for availability and metadata handling, not as an authority that can change valid signed endpoint records undetectably.
- **[Verified]** A Durable Object is a strongly consistent storage boundary; it is not an independent replica or DHT. A per-key object serializes competing writes for that key. Platform/account compromise, service deletion, outage, quota exhaustion, or malicious withholding can make records unavailable or stale.
- **[Inferred]** Anyone who learns a public endpoint ID can query this service and read the signed address record. Address filtering should remain relay-only by default; publishing a direct address is a separate privacy decision and is not changed by this service.

### Threats and controls

| Threat                                                        | Design control                                                                                                                                                                                                                        | Residual risk                                                                                                                                        |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Forged or malformed packet, key/path mismatch, oversized body | Cap body at 1,072 bytes; validate z32 key, decode format, verify Ed25519 signature and embedded public key, and validate DNS packet bounds before storage. Clients still verify signatures.                                           | Parser/crypto implementation defects; WebCrypto Ed25519 runtime behavior and cost have not been exercised on Workers.                                |
| Replay or stale update replacing newer record                 | DO transaction compares signed packet timestamp/sequence with the stored value and atomically stores only a strictly newer value. Return `204` for a valid stale packet for iroh compatibility; do not let an older record overwrite. | A malicious endpoint can publish its own newer, valid record. Clock/sequence semantics and overflow need vector tests.                               |
| Write/read flooding, key churn, cost exhaustion               | Per-IP rate limiting at Worker edge; bounded body and route set; per-key DO; explicit request quotas/alerts and abuse response; cap storage/retention and reject overload cleanly.                                                    | IP limits can be evaded/distributed and may affect NAT-shared users. DO hot-key queues and plan/account quotas can still deny service or incur cost. |
| Poisoned cache or stale DNS answer                            | Use DO authoritative latest value; send conservative HTTP cache directives and honor conditional GET only if implemented safely. DoH responses use a bounded DNS TTL; clients verify signed packets.                                  | CDN/browser/intermediate caching may delay visibility; DNS TTL is not a consistency guarantee.                                                       |
| Data loss or service/region outage                            | DO durable storage and operational backups/export before any production reliance; keep n0 configuration available as rollback.                                                                                                        | No cross-provider replication in this design; DO durability is not a backup or availability SLA.                                                     |
| Privacy leakage                                               | Explain that the service sees IPs, queried IDs and timing; publish relay-only endpoint info; optional use is explicit.                                                                                                                | HTTPS does not conceal metadata from Cloudflare; DoH moves DNS query visibility from recursive resolvers to this operator/platform.                  |

## API and protocol contract

### PUT and GET

- Base URL is configured by the operator; iroh appends the z32 endpoint ID. Keep route shape compatible with the existing client seam: `PUT|GET <base>/<z32>`; the deployment may expose this under `/pkarr`.
- **PUT:** accept only `application/x-pkarr-signed-packet`; hard-limit request body to 1,072 bytes (signature 64 + timestamp 8 + DNS packet ≤1,000). Validate key encoding and that the packet key matches the path; parse and verify the signed bytes exactly as pkarr v3 (`3:seqi<ts>e1:v<len>:<packet>`); reject malformed, invalid-signature, invalid-DNS, and too-large inputs (400/413). Atomically store only a packet more recent than the current packet. Return `204 No Content` on new or valid stale packet. Return `429` on rate limit and `503`/`500` on storage failure; never claim success if a new packet was not durably accepted.
- **GET:** return the latest relay payload with `Content-Type: application/x-pkarr-signed-packet`; return `404` if absent/expired. Preserve the full signed payload. Conditional GET/`If-Modified-Since` may be added only after compatibility tests; do not rely on CDN caching for correctness.
- The 204 stale-write behavior is **[inferred]** to be safer for iroh 1.3.0: the reference iroh DNS server returns 204 for stale packets and the iroh publisher accepts only 2xx, while the pkarr relay spec describes 409. See research [§D](./2026-10-11T11-08-42Z_design_remaining_work/research-round2/cloudflare-workers/RESULTS.md#d-open-source-pkarr-relays-and-the-iroh-dns-server).
- Set an explicit retention policy. Recommended starting point: expire records after 7 days without refresh, matching the reference server's observed policy **[verified as source behavior]**; use a DO alarm or lazy expiration, not per-request scans. The 5-minute client republish interval means an active endpoint refreshes well within this window **[verified]**.

### DoH only

- `POST /dns-query`, `Content-Type: application/dns-message`; parse bounded DNS wire messages and answer only the intended TXT question under the configured origin (`_iroh.<z32>.<origin>`). Return `application/dns-message` with the DNS response; reject unsupported query types/names with a valid negative answer, and malformed/oversized messages with 400. No general recursive forwarding.
- **[Verified]** The native iroh client supports HTTPS DoH via `NameserverConfig::https(...).with_tls_server_name(...)` and POSTs `application/dns-message` to `/dns-query`; this is compiled for native targets. See research [§D, DoH client](./2026-10-11T11-08-42Z_design_remaining_work/research-round2/cloudflare-workers/RESULTS.md#d-open-source-pkarr-relays-and-the-iroh-dns-server).
- **[Inferred]** Configure clients with the DoH endpoint directly. Do not claim the Worker hosts an ordinary DNS zone: it has no DNS-over-UDP/TCP listener or port-53 delegation path. DoH answers should carry a short TTL aligned with the record format/client expectations and must not expose unsigned address data as authoritative.

## Storage, consistency, caching, and abuse limits

- Use a SQLite-backed DO namespace, one DO ID derived deterministically from validated public key bytes. Store only current signed payload, timestamp, and expiry metadata. Transactionally compare-and-set so concurrent updates cannot regress the key's record.
- **[Verified]** Research read Cloudflare's 2026-10-11 public docs: SQLite-backed DOs support a 2 MiB key+value maximum, 10 GB per object on Paid, and a soft 1,000 requests/s per object; a full queue can return `overloaded`. KV permits one write/second/key and may be stale by 60 seconds or more, so it is unsuitable for the authoritative write path. Use DO, not KV. Re-check these limits before implementation.
- **[Inferred]** A signed record is at most 1,072 bytes, making value size small; the per-object request queue and global account quotas, not record bytes, are the meaningful capacity concerns. A per-key DO serializes hot updates but also creates a hot-key bottleneck.
- Disable shared cache for PUT and initially for GET/DoH (`Cache-Control: no-store`) to make DO state authoritative and avoid stale edge reads. Later, immutable/version-aware ETags and short public caching can be considered after multi-region cache tests. Never let cache updates bypass signature/sequence checks.
- Apply rate limiting to both PUT and GET/DoH by source IP and use stricter write limits; research confirms Cloudflare's rate-limit binding supports configured limits with 10- or 60-second periods. Add body/query size bounds, per-key write pacing, bounded retention, overload responses, and operational request/CPU/DO usage alarms. Keep limits configurable and document that IP controls are abuse friction, not authentication.
- Cost estimate is not a promise: research's **[inferred]** back-of-envelope estimate assumes 2 ms CPU and 288 idle PUTs/endpoint/day; it estimates roughly $5/month at 100 endpoints, $6.15 at 1,000, and $43.59 at 10,000 on the researched Paid pricing. Recalculate with measured PUT/GET/DoH and DO duration before a public rollout.

## Feasibility grounded in research

- **[Verified]** Workers implement HTTP `fetch` handlers, support WebCrypto Ed25519 import/verify, and support SQLite-backed Durable Objects. The pkarr HTTP routes and DoH request/response path do not require UDP, long-lived TCP, or local disk.
- **[Verified]** Workers have no functional UDP socket (`node:dgram` is documented as a non-functional stub) and no persistent local disk; `/tmp` is per-request/non-persistent. Therefore they cannot run DNS on UDP/TCP, join Mainline DHT, or run either existing relay server unmodified. The existing `iroh-dns-server` also needs DNS UDP/TCP and redb local disk; `pkarr-relay` starts a DHT node and uses LMDB.
- **[Verified]** Paid Workers were documented as $5/month minimum, 128 MiB memory, 30-second default CPU configurable up to five minutes, and 10 million Worker requests included; DO has separate request/duration meters. The free Worker CPU limit is 10 ms and free plan daily request allowance makes research estimate capacity about 347 endpoints at the five-minute republish cadence. Rates/limits dated 2026-10-11; reconfirm before build.
- **[Inferred]** A small custom JS/TS implementation using DO storage and WebCrypto is feasible and substantially smaller than porting the complete Rust servers. Research estimates 150–200 lines for relay and 150–250 more for DoH; treat that as a rough estimate, not a sizing guarantee.
- **[Not checked]** No Worker was built or deployed. Ed25519 verification speed, CPU usage, DO transactional behavior under concurrent requests, caching behavior, account pricing, and production-scale limits remain unverified.
- This is lookup-only. It is not a replacement for n0 relays or a QAD-enabled VPS; it offers no direct-path/NAT traversal capability and is not DHT-backed. Records written here are visible only to clients configured to use this service unless some separate replication is later designed.

## Protocol and endpoint integration seam

- **Current seam [verified in repository]:** `pkgs/id/src/connectivity.rs::public_builder` starts from `presets::N0`; comments identify N0's pkarr and DNS services. The research confirms `PkarrPublisher::builder(url)`, `PkarrResolver::builder(url)`, and direct HTTPS `NameserverConfig` are available in the pinned iroh stack.
- **Proposed seam [inferred]:** add an explicit lookup configuration, e.g. `LookupPolicy::{N0, Worker { pkarr_base_url, doh_url, tls_server_name, origin }}`, and have one builder compose publisher, resolver, and DoH DNS lookup from the same selected service. Keep `N0` as default and make custom Worker selection opt-in in server config/CLI, validating `https` URLs and preventing an accidental split between publication and resolution. Do not change relay map/QAD settings.
- Keep Worker implementation separate from Rust endpoint configuration. The endpoint talks the existing pkarr HTTP payload contract and HTTPS DNS-message transport; no protocol fork or new client library is needed. First implementation should add fixture-based interop tests against iroh-generated signed packets and DoH queries, then local Worker/DO tests.
- **Boundary to existing app:** no change to `RelayPolicy`, relay kit, relay server, ticket format, direct-address policy, DHT opt-in, or QAD. The endpoint's existing 45-second dial deadline continues to bound lookup waits.

## Rollout, rollback, and verification

1. **Before coding:** re-read linked official documentation and pkarr/iroh source at the current pinned versions; confirm exact storage transaction APIs, DNS question/answer semantics, and Cloudflare plan/pricing. Capture dependency/source versions in the implementation record.
2. **Local validation:** protocol golden vectors for valid/invalid signature, public-key/path mismatch, malformed DNS packet, 1,072/1,073-byte boundaries, equal/older/newer timestamps, concurrent writes, expiry, and GET after accepted/stale PUT. Verify client behavior for 204 stale writes. Add DoH TXT query vectors and verify DNS IDs, flags, bounds, TTL, negative answers, and unsupported types.
3. **Worker integration, before any public use:** run Wrangler local tests with SQLite DO binding; exercise cross-object isolation, one-key serialization, simulated storage errors/overload, all unsupported routes/methods, cache headers, rate limits, and strict no-forwarding behavior. Measure CPU and DO metering for signed PUT, GET and DoH workloads.
4. **Canary only after explicit deployment approval:** deploy to a non-production hostname/namespace with synthetic keys; exercise publisher, resolver and DoH from native iroh clients. Confirm public key record is valid, stale write never regresses, request limits and observability are working, and no unintended routes are exposed. Do not use real account or production data in design work.
5. **Opt-in rollout:** retain N0 default; select Worker only for an explicitly configured test cohort. Compare lookup success/latency and cost against n0; do not claim availability or privacy improvement without measurements.
6. **Rollback:** switch endpoint configuration back to N0 (publisher, resolver, and DoH as a single unit). Keep service data exportable before production reliance. If compromised, stop writes, disable the Worker route/binding, and revert clients to N0; changing DNS/HTTP base URL alone must not strand default clients.

No deployment, account access, domain change, or live-network verification is authorized by this design note.

## Risks

1. **Custom implementation correctness:** exact pkarr v3 signature bytes, payload framing, DNS parsing, key binding and monotonic comparison are security-sensitive.
2. **Single-provider availability and privacy:** Cloudflare can observe lookup metadata and can deny/withhold service; there is no DHT fallback or independent replica.
3. **Operational capacity/cost:** publish-on-change plus five-minute republish, reads, DoH, DO duration, hot keys, abuse, and current billing can exceed the rough research estimate.
4. **Client behavior:** custom pkarr + DoH composition must be validated at the pinned iroh version; changing only a relay URL is not enough to route DNS lookup through the Worker.
5. **Cache consistency:** CDN cache can defeat freshness even when DO itself is strongly consistent; initial no-store policy reduces but does not eliminate client/runtime caching concerns.
6. **Not a relay:** users may mistake pkarr lookup service for traffic relay, DHT participation, or NAT traversal. Product/help text must name its scope clearly.
7. **Version drift:** the source research used iroh 1.3.0 and Cloudflare docs as of 2026-10-11; validate against the actual implementation lock and current platform documentation.

## Open decisions (recorded, not blockers to this design)

- **Namespace and domain:** choose the Worker hostname, pkarr base path, and DoH zone/origin; this note assumes operator-configured HTTPS hostnames and no normal NS delegation.
- **Retention and DNS TTL:** 7 days is a reference-server-derived starting point; confirm whether the application wants shorter retention and the right TXT TTL.
- **Rate policy:** select per-IP PUT, GET and DoH thresholds, burst behavior and operator alert budget after expected endpoint count is known.
- **Cache policy:** begin `no-store`; decide if ETag/`If-Modified-Since` and short edge caching are worth the added stale-data complexity.
- **Auth/abuse operations:** design assumes public reads, signed writes, no account-level authentication, and rate limits; decide whether an abuse report/contact path or emergency key/namespace shutdown is required before public exposure.
- **Deployment tier and budget:** research points to Paid due to CPU/request headroom, but account entitlements and billing must be reconfirmed by the owner before deployment.
- **Replication/fallback:** no n0 mirroring or DHT is included. Adding replication changes the trust, privacy, conflict and consistency model and requires a separate design.

## Out of scope

- Creating or configuring Cloudflare accounts, Workers, Durable Objects, DNS records, billing or credentials.
- Deploying, publishing, production code, benchmarking on a live Worker, account access or real-network tests.
- DNS over UDP/TCP, DNS delegation, recursive DNS, Mainline DHT, pkarr DHT replication, iroh relay/WebSocket protocol, QAD, NAT traversal, or replacing n0 relays.
- Authentication, endpoint authorization, direct-address publishing policy changes, a general cache/CDN, multi-provider replication, or a new pkarr protocol.

## Sources

Repository research is the source index; it contains file/version evidence and date-read for official source pages:

- [Round-2 research RESULTS.md](./2026-10-11T11-08-42Z_design_remaining_work/research-round2/cloudflare-workers/RESULTS.md), particularly §§A (Workers limits/pricing/DO/KV/WebCrypto), D (pkarr, iroh client and server), E (cost arithmetic), and “Not verified”.
- [Official Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [pricing](https://developers.cloudflare.com/workers/platform/pricing/), [DO limits](https://developers.cloudflare.com/durable-objects/platform/limits/), [KV limits](https://developers.cloudflare.com/kv/platform/limits/), [KV consistency](https://developers.cloudflare.com/kv/concepts/how-kv-works/), [Web Crypto](https://developers.cloudflare.com/workers/runtime-apis/web-crypto/), and [rate-limit binding](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/). Research read 2026-10-11; re-check current docs.
- [pkarr relay specification](https://github.com/pubky/pkarr/blob/main/design/relays.md); iroh `iroh-dns`/`iroh-dns-server` 1.3.0 and `pkarr-relay` 2.1.0 sources, as cited with paths in research §D.
- In-repository integration points: [`pkgs/id/src/connectivity.rs`](../pkgs/id/src/connectivity.rs), [Task 08](./2026-10-11T11-08-42Z_design_remaining_work/task-08-connectivity-extensions.md), and [phase 7 record](./2026-10-12T21-22-53Z_phase_7_connectivity/2026-10-12T21-22-53Z_phase_7_connectivity.md).
