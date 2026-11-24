# Cloudflare Containers billing + Durable Objects real-world costs

Researched 2026-10-20 from official Cloudflare documentation. Every factual claim carries its
source URL. Anything not found on an official page is flagged **UNVERIFIED**.

Primary sources:

- Containers pricing: https://developers.cloudflare.com/containers/platform/pricing/
- Containers FAQ: https://developers.cloudflare.com/containers/faq/
- Container class API: https://developers.cloudflare.com/containers/api/container-class/
- Containers limits / instance types: https://developers.cloudflare.com/containers/platform/limits/
- Durable Objects pricing: https://developers.cloudflare.com/durable-objects/platform/pricing/
- Workers pricing (plan-level statements + DO section): https://developers.cloudflare.com/workers/platform/pricing/

---

## Part A — Containers billing

### A1. Included amounts and overage rates (Workers Paid)

Containers have **no Free-plan allocation** (Free column is "N/A"); the included amounts below
come with the $5/month Workers Paid plan. Billing is metered **every 10 ms** that an instance is
actively running.

| Meter | Included / month (Workers Paid) | Overage rate | Equivalent |
|---|---|---|---|
| Memory | 25 GiB-hours | $0.0000025 per GiB-second | $0.009 per GiB-hour |
| CPU (vCPU) | 375 vCPU-minutes | $0.000020 per vCPU-second | $0.072 per vCPU-hour |
| Disk | 200 GB-hours | $0.00000007 per GB-second | $0.000252 per GB-hour |

Source: https://developers.cloudflare.com/containers/platform/pricing/

Key mechanics from the same page:

- **Memory and disk are billed on provisioned resources** for the instance type selected —
  you pay for the size you picked, running idle or not, for as long as the instance is up.
- **CPU is billed on active usage only** — an idle-but-running instance accrues memory and
  disk charges but ~zero CPU charges.
- On top of the container meters, "incoming requests to your containers are handled by your
  Worker, and each container has its own Durable Object. You are billed for your usage of
  both Workers and Durable Objects." Container logs are billed at Workers Logs rates.

Source: https://developers.cloudflare.com/containers/platform/pricing/

### A2. Billing increment and what counts as billable time

- **Smallest billing increment: 10 ms.** "Containers are billed for every 10ms that they are
  actively running."
- **When the clock starts:** "charges start when a request is sent to the container or when
  it is manually started." The clock therefore starts at the start request, not when your
  process finishes booting — cold-start/provisioning time falls inside the billable window
  for the provisioned memory/disk meters (the pricing page does not itemise provisioning
  separately; this is the direct reading of the two statements above).
- **When the clock stops:** "Charges stop after the container instance goes to sleep, which
  can happen automatically after a timeout."
- **Idle time counts.** An instance that has started and is sitting idle is still "running":
  memory and disk (provisioned) keep accruing until the instance sleeps/stops. Only CPU
  drops to ~zero while idle, because CPU is usage-based. This is the entire reason
  `sleepAfter` is the main cost lever.

Source: https://developers.cloudflare.com/containers/platform/pricing/

### A3. Instance lifetime mechanics — `sleepAfter`

- `sleepAfter` (Container class property, `string | number`, **default `"10m"`**): "how long
  to keep the container alive without activity before shutting it down. Accepts a number of
  seconds or a duration string such as `"30s"`, `"5m"`, or `"1h"`. Activity resets the
  timer." It can also be reset manually via `renewActivityTimeout()`. When the timer expires,
  `onActivityExpired()` fires; the default implementation calls `stop()`.
  Source: https://developers.cloudflare.com/containers/api/container-class/
- With the lower-level Durable Object Container API the equivalent is
  `setInactivityTimeout()`; the runtime stops the container after the Durable Object becomes
  inactive. Source: https://developers.cloudflare.com/containers/concepts/architecture/
- **Minimum / maximum `sleepAfter`:** the Container class documentation publishes **no
  minimum and no maximum** — only the formats above. **UNVERIFIED** beyond that. (For
  comparison, the Sandbox docs — a different product built on the same Durable Object
  Container API — state the inactivity timeout works "up to 6 hours":
  https://developers.cloudflare.com/sandbox/concepts/lifetime/ . Do not assume the same cap
  is documented for Containers' `sleepAfter` property; it is not on the Containers pages.)
- **Minimum billable duration per start:** none published. There is no per-start or
  per-request fee for the container itself on the pricing page — billing is purely the
  10 ms-metered memory/CPU/disk above (plus the separate Worker + Durable Object charges
  for the fronting Worker and the container's own DO). Source (by inspection of the full
  rate card): https://developers.cloudflare.com/containers/platform/pricing/

### A4. Worked example — one `lite` instance

`lite` instance specs (official): **1/16 vCPU, 256 MiB memory, 2 GB disk**.
Source: https://developers.cloudflare.com/containers/platform/pricing/ and
https://developers.cloudflare.com/containers/platform/limits/
(Full table: lite 1/16 vCPU / 256 MiB / 2 GB · basic 1/4 / 1 GiB / 4 GB · standard-1 1/2 /
4 GiB / 8 GB · standard-2 1 / 6 GiB / 12 GB · standard-3 2 / 8 GiB / 16 GB · standard-4 4 /
12 GiB / 20 GB.)

**How many instance-hours does the included allowance buy for `lite`?**
All three meters bind at the same point, by design:

- Memory: 25 GiB-h ÷ 0.25 GiB = **100 instance-hours**
- Disk: 200 GB-h ÷ 2 GB = **100 instance-hours**
- CPU (only if pegged at the full 1/16 vCPU the whole time): 375 vCPU-min = 22,500 vCPU-s
  ÷ (1/16) = 360,000 s = **100 instance-hours**

→ The Workers Paid included allowance buys exactly **~100 lite-instance-hours per month**
(≈ 4.2 days of continuous running). After that you pay overage.

**One lite instance running continuously for a 30-day month (720 h = 2,592,000 s):**

| Meter | Usage | Included | Overage qty | Overage cost |
|---|---|---|---|---|
| Memory | 0.25 GiB × 720 h = 180 GiB-h | 25 GiB-h | 155 GiB-h = 558,000 GiB-s | 558,000 × $0.0000025 = **$1.40** |
| Disk | 2 GB × 720 h = 1,440 GB-h | 200 GB-h | 1,240 GB-h = 4,464,000 GB-s | 4,464,000 × $0.00000007 = **$0.31** |
| CPU (worst case: pegged at 1/16 vCPU 24/7) | 2,592,000 s × 1/16 = 162,000 vCPU-s | 22,500 vCPU-s | 139,500 vCPU-s | 139,500 × $0.00002 = **$2.79** |
| CPU (typical: mostly idle) | ~0 active vCPU-s | — | ~0 | **~$0** |

- Always-on lite, CPU pegged the whole month: **≈ $4.50/month in overage** on top of the
  $5 plan (plus the small Worker/DO charges for the container's own Durable Object).
- Always-on lite, CPU mostly idle: **≈ $1.71/month in overage** — memory + disk only.

**Container network egress** (separate meter, per month):

| Region | Included allotment | Price per GB after |
|---|---|---|
| North America & Europe | 1 TB | $0.025 |
| Oceania, Korea, Taiwan | 500 GB | $0.05 |
| Everywhere else | 500 GB | $0.04 |

Source: https://developers.cloudflare.com/containers/platform/pricing/
(Note the contrast: Workers/R2 egress is free; container egress is the exception — see B1.)

### A5. Maximum instance lifetime

- **There is no fixed maximum runtime.** "Cloudflare does not stop a container instance
  after a fixed maximum runtime." An instance runs until it is stopped (inactivity timeout,
  `stop()`/`destroy()`/`signal()`, rollout) or the platform stops it.
- **But no uptime is guaranteed either:** "a host server restart happens on an irregular
  cadence. Cloudflare does not guarantee that any container instance will run for a set
  period." On any platform stop, the main process gets **SIGTERM, up to 15 minutes to
  exit, then SIGKILL**. Disk is ephemeral — a restarted instance comes back with a fresh
  disk from the image (snapshots/FUSE-to-R2 are the persistence options).

Source: https://developers.cloudflare.com/containers/faq/

---

## Part B — Durable Objects costs in real use

### B1. Full rate card (Workers Paid)

Source for the whole table: https://developers.cloudflare.com/durable-objects/platform/pricing/
(page updated 2026-10-18; cross-checked against
https://developers.cloudflare.com/workers/platform/pricing/)

**Compute**

| Meter | Workers Free | Workers Paid — included | Workers Paid — overage |
|---|---|---|---|
| Requests | 100,000 / day | 1 million / month | **$0.15 / million** |
| Duration | 13,000 GB-s / day | 400,000 GB-s / month | **$12.50 / million GB-s** |

- "Requests" includes HTTP requests, RPC sessions, WebSocket messages, and **alarm
  invocations**. Each RPC method call on a stub is its own RPC session = 1 billed request
  (calls on a returned `RpcTarget` stub ride the same session and are not billed again).
- **Incoming WebSocket messages are billed at a 20:1 ratio** (100 incoming messages = 5
  billed requests). A request is charged to establish the WebSocket. **Outgoing WebSocket
  messages are free**, and incoming WebSocket protocol pings are not charged.
- **Duration is wall-clock time while the object is active, billed at a flat 128 MB
  allocation** regardless of actual memory use — even if several objects share one isolate's
  128 MB, each is billed as if allocated the full 128 MB.
- **Rounding gotcha:** billable usage above the included amount is **rounded up to the next
  billable unit** before pricing — the docs' own example rounds 500,000 billable GB-s up to
  1,000,000 GB-s. In practice, *any* duration overage costs a minimum of $12.50, and any
  request overage a minimum of $0.15, per month.

**Storage — SQLite backend (recommended; the only backend on Free)**

| Meter | Workers Free | Workers Paid — included | Workers Paid — overage |
|---|---|---|---|
| Rows read | 5 million / day | 25 billion / month | **$0.001 / million rows** |
| Rows written | 100,000 / day | 50 million / month | **$1.00 / million rows** |
| SQL stored data | 5 GB total | 5 GB-month | **$0.20 / GB-month** |

- Rates match D1. The key-value-style methods (`get`/`put`/`delete`/`list`) run against a
  hidden SQLite table and are billed as rows read/written.
- **Each `setAlarm()` is billed as one row written.** Deletes count as rows written.
- Writes are 1,000× the read price — the single most asymmetric meter in the stack.
- Stored data is billed until removed; even an empty SQLite database is ~12 KB, and
  internal metadata tables count toward billable storage.

**Storage — legacy key-value backend (Paid only)**

| Meter | Included | Overage |
|---|---|---|
| Read request units (4 KB = 1 unit) | 1 million | $0.20 / million |
| Write request units | 1 million | $1.00 / million |
| Delete requests | 1 million | $1.00 / million (deletes unmetered by size: one charge per delete call) |
| Stored data | 1 GB | $0.20 / GB-month |

List operations are billed as read units by data examined; cache hits and multi-key
`get`/`put`/`delete` are billed as if each key were an individual request. `setAlarm` =
1 write request unit.

**Data transfer:** there is **no inbound or outbound data-transfer meter for Durable
Objects**. The Workers Paid plan states: "There are no additional charges for data
transfer (egress) or throughput (bandwidth)" — Durable Objects are part of that plan, and
the DO pricing page lists no transfer line item. (Containers, above, are the exception.)
Source: https://developers.cloudflare.com/workers/platform/pricing/

### B2. Worked examples

**(a) One always-on, non-hibernating 128 MB Durable Object, 30-day month**

- Duration: 2,592,000 s × 0.125 GB = **324,000 GB-s** — inside the 400,000 GB-s included
  allowance → **$0 duration cost**. (31-day month: 334,800 GB-s, still inside.)
- Requests: only whatever traffic keeps it awake — e.g. a 1-request/minute heartbeat =
  43,200 requests, inside the 1M included → $0.
- **Total: $0 on top of the $5 plan minimum.** What dominates: duration is the only
  meaningful meter, and one always-on object consumes ~81% of the account's *entire*
  included duration pool. A **second** always-on object pushes the account to ~648,000
  GB-s → 248,000 billable → rounded up to 1,000,000 GB-s → **+$12.50/month**. Always-on
  objects are effectively "first one free, each additional ≈ $12.50" (before rounding
  effects at larger counts).

**(b) Hibernating WebSocket room: 100 clients, 1 message/minute each, 24/7**

Assumptions (matching Cloudflare's own Example 4 modelling): hibernatable WebSockets via
`state.acceptWebSocket()`, ~10 ms of handler execution per message, clients connect once
and stay connected all month through hibernation.

- Incoming messages: 100 × 60 × 24 × 30 = 4,320,000 → at the 20:1 ratio = **216,000 billed
  requests**; +100 connection-establishment requests = 216,100 — inside 1M included → $0.
- Outgoing broadcasts: free. Protocol pings: free.
- Duration: the object is only active while handling — 100 msgs/min × 10 ms = 1 s per
  minute → 43,200 s/month × 0.125 GB = **5,400 GB-s** — inside included → $0.
- **Total: ≈ $0 on top of the $5 plan.** What dominates: **requests** (216k of the 1M
  included pool) — hibernation collapses the duration meter to ~1.7% of the always-on
  figure for the same room (5,400 vs 324,000 GB-s).
- Contrast at scale, from Cloudflare's own worked examples (100 Durable Objects, 50–100
  clients each, 1 msg/min): **without** hibernation $142.95/month (duration $137.50 of it);
  **with** WebSocket Hibernation $20.65/month (duration $12.50, requests $3.15).
  Source: https://developers.cloudflare.com/durable-objects/platform/pricing/ (Examples 2
  and 4)

**(c) Coordination-only Durable Object that stores nothing**

Pass-through pattern: the Worker/DO receives a call and forwards it (e.g. to R2 or another
service) without touching DO storage. Cost = requests + the brief duration while your
handler executes. Example at volume: 10 million RPC calls/month, 5 ms handling each:

- Requests: 10M − 1M included = 9M × $0.15/M = **$1.35**
- Duration: 10,000,000 × 0.005 s = 50,000 s × 0.125 GB = 6,250 GB-s → inside included → $0
- Storage: **$0** — no rows written, nothing stored. (Caveats: if you call `setAlarm()` it
  counts as a row written; an object that never writes still avoids storage charges
  entirely.)
- **Total ≈ $1.35/month overage.** What dominates: **requests, by a mile.** A
  store-nothing coordinator is the cheapest DO pattern there is — duration only accrues
  for the milliseconds your code is actually running, and the artifact/data path
  (Worker → R2) never touches the DO's disk or storage meters at all.

### B3. Hibernation mechanics

Sources: https://developers.cloudflare.com/durable-objects/platform/pricing/ (Compute
billing + footnotes + FAQ) unless noted.

- **What stops the duration meter:** duration is billed for wall-clock time while the
  object is *actively executing JavaScript* (handling a request or running event handlers)
  **or** *idle in memory but unable to hibernate*. An idle object that is **eligible** for
  hibernation is **not billed for duration, even before the runtime actually hibernates
  it**. Inactive objects receiving no requests incur no duration charges at all.
- **The WebSocket trap:** calling `accept()` on a WebSocket "will incur duration charges
  for the entire time the WebSocket is connected." Use the **WebSocket Hibernation API**
  (`state.acceptWebSocket()`) so the object can hibernate while connections stay open;
  the runtime wakes it when a message arrives.
- **Auto-responses are free time:** messages handled by `state.setWebSocketAutoResponse()`
  (e.g. heartbeat ping/pong) "will not incur additional wall-clock time, and so they will
  not be charged" — the object doesn't even wake.
- **What wakes a hibernated object:** a request routed to it ("Requests to a Durable
  Object keep it active or create the object if it was inactive"), an incoming message on
  a hibernatable WebSocket, or an alarm firing. On wake, the constructor runs again —
  in-memory state is gone and must be rebuilt from storage.
- **What blocks hibernation / keeps the meter running:**
  - WebSockets accepted with `accept()` instead of the Hibernation API (billed for the
    whole connection lifetime).
  - **Pending I/O:** "Pending I/O operations keep an object in memory for up to 15
    minutes each. The object incurs duration charges during that time." (Corroborated by
    the compatibility-flags docs: pending service-binding requests, DO RPC calls,
    `container.monitor()`, and `ctx.waitUntil()` promises each prevent eviction until
    completion or a 15-minute cap:
    https://developers.cloudflare.com/workers/configuration/compatibility-flags/)
  - Actively executing handlers — duration is shared across all requests concurrently
    active on the object (it is wall-clock, not per-request summed).
- **Do alarms bill duration?** Partially, in three small pieces:
  - The alarm **invocation counts as a request** (requests meter includes alarm
    invocations).
  - Each **`setAlarm()` call is billed as one row written** (SQLite backend) or one write
    request unit (KV backend) — even if you store nothing else.
  - While the **alarm handler is executing**, the object is actively running JavaScript,
    so duration accrues for those milliseconds — same as any handler. Between alarms, an
    idle hibernation-eligible object accrues nothing. A short handler on a sparse alarm
    is effectively free; the meter to watch is a high-frequency alarm cadence eating the
    requests allowance (e.g. a 30-second keep-alive heartbeat = 86,400 requests/month).

---

## Headline numbers

- **Containers (Workers Paid):** 25 GiB-h memory, 375 vCPU-min, 200 GB-h disk included;
  overage $0.0000025/GiB-s, $0.000020/vCPU-s, $0.00000007/GB-s; billed per 10 ms; memory &
  disk on provisioned size (idle counts), CPU on active use only.
- Included container allowance = **exactly ~100 lite-instance-hours/month**; an always-on
  lite costs ≈ **$1.71/mo overage idle-CPU, ≈ $4.50/mo with CPU pegged**. No maximum
  instance runtime — but no uptime guarantee (SIGTERM + 15 min grace on host stops).
- **Durable Objects (Workers Paid):** 1M requests (+$0.15/M) and 400,000 GB-s duration
  (+$12.50/M GB-s) included; SQLite rows read 25B incl. (+$0.001/M), rows written 50M
  incl. (+$1.00/M), stored 5 GB-mo incl. (+$0.20/GB-mo); no egress charges.
- One always-on DO ≈ 324,000 GB-s/month = **free alone, ~$12.50/mo for each additional**
  (rounding makes any duration overage cost at least $12.50). Hibernation cuts a busy
  WebSocket room's duration by ~98%; a store-nothing coordination DO is billed almost
  entirely on requests ($0.15/M).
