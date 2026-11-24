# PM stack: full migration to Cloudflare hosting — design of record

- **Date:** 2026-11-24
- **Status:** design of record for [#14107](https://github.com/developing-today/code/issues/14107). Design only — no slice in §11 is queued yet; slices are packetised from this record after it lands.
- **Direction (Drew, 2026-11-23 23:51 CST):** "migrate fully into cloudflare fallback to cf tunnel then fallback to tailscale dns."

## 0. The direction, restated as architecture

Every human-facing surface of the PM stack gets one canonical name and a three-tier serving chain, in this order:

1. **Cloudflare hosting** — Workers/Pages serving from the hosted data plane. The normal path.
2. **Cloudflare Tunnel** — the same names, served by the laptop edition through a PM cloudflared tunnel. The first fallback.
3. **Tailscale DNS** — `amd-6.raptor-pumpkinseed.ts.net`, the laptop edition at its tailnet name. The final fallback.

Two honesty rules govern everything below:

- **A tier is only a fallback if a request can actually reach it.** Tier 1→2 can be mechanised at the front door (§5). Tier 3 is a different DNS name that no Cloudflare machinery can fail over to — it is a *bookmark the human picks*, and this design says so wherever it matters instead of dressing it up as failover.
- **A tier serves real data or says it cannot.** No tier, at any point in the chain, renders invented data to fill a gap. That is the #14065 rule, and the migration is where it is easiest to break — three serving paths for one page is three chances to fabricate. Every tier transition in §5 is specified against it.

## 1. Premise check — what is actually true today

Verified 2026-11-24 from the review record (`doc/2026-11-24T03-51-11Z_review_pm-stack-pages/`), the pm-stack sources on main, the estate worker registry (`pm-stack/platform/workers.json`, checked 2026-11-23), and the #9079 design (`doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition/`). Anything not verified live is marked.

**Serving today.** Every page a human opens is served from the laptop (`amd`): the aggregator (Bun/TypeScript, loopback :4870 + tailnet), the console origin (:4872), dispatch (:4879), projector (:7801), and the Docker apps Kaneo (:5173), Pad (:7777), Grist (:8484), ready-for-agent (:6056) — all NixOS units/containers on the one machine. If the laptop is off, every one of those pages is gone, not degraded. The deployed aggregator build is commit `7d3dcebe` (2026-11-15), **4,687 commits behind main** (#14059); the console has the same disease (#13833).

**A two-tier shape already exists for the console — it is the seed, not the exception.** `pm.security.cab` is fronted by the `pm-console-edge` Worker (in service, deployed by the `pm-console-edge-deploy` GitHub Actions workflow): a route is put on the Worker by `scripts/expose.py` once it answers its health check, and taking the route away is the rollback. Its `PmEdge` Durable Object holds a copy of what the host last reported, items held for the host, push subscriptions and keys, and linked phones; an opt-in `EDGE_DIRECT` mode lets the Worker read/write GitHub and the queue store itself while the laptop is away. Behind it, a tunnel sends `pm.security.cab` traffic to the console origin, which passes the dashboard's paths to the aggregator for a signed-in owner (`pm-stack/PORTS.md`). So the console already lives the pattern this design generalises: hosted front door, tunnel behind it, route-flip as the control.

**The hosted data plane is real but partial.** Live today: the decision-events Worker (D1 + per-set Durable Objects, webhook intake, in service); the Turso alerts and decision ledgers the aggregator reads; the console edge's KV/DO state. Designed but **not deployed**: the pm-notify relay Worker (#8569 phase 2 — the dead-man's switch sender). The fleet feed is a file written by a collector on the laptop and read by the aggregator — a single point of failure by construction.

**The hosted edition is already designed and in flight — this design builds on it, it does not restart it.** The pm-service Workers edition (#9079, component A of #9078) re-implements the aggregator's capabilities on Workers against the **same data model** (#9060) in the **tenant Turso plane** (#9083 component E): board data lives in the tenant's Turso DB, full stop; D1 is rejected there for board data (a second SQL plane would be a divergence factory). Its filed slices: 1 tenant store (#12825, landed — `pm-stack/pm-service` is in the repo), 2 hosted collector on Cron Triggers (#12826), 3 read-API parity, 4 dashboard surfaces (#12829), 5 `BoardDO` live state, 6 `ReconcileDO` + writer-of-record lease (#12831), 7 identity wiring (the hosted edition consumes the estate sign-in, #9077/H, directly), 8 dual-edition conformance (#12833). None of it is the serving path yet.

**Where #14107 widens #9079 — stated plainly.** #9079's V1 parity list covers the board/roadmap/releases/quota core and deliberately defers the aggregator's accretions (push, AA explorer, search, sessions, digest…) and the beside-products (Kaneo, Pad, Grist are separate products, "out of scope in both directions" there). Drew's direction is *fully* — all 38 reviewed surfaces get a target in §3, including the accretions and the beside-products. Where a surface fits pm-service, pm-service is the vehicle; where #9079 explicitly placed a product outside pm-service, the surface still gets a tier target (tunnel-served under the front door), owned by its own programme for anything more.

**Composition, not forking.** Three neighbouring designs land in the same hosted home and this design does not re-decide any of them: push unification (#14084 — one registry in the fleet-platform Turso store, one sender, the console at pm.security.cab as the one preferences surface, aggregator preferences page redirecting after its S4); answer write-back (#14043 — an applier applies recorded answers to the queue broker; slices 21653–21657 queued; edge answer intake is #13469); the writer-of-record lease (#9079 §5 — exactly one writer of record per board, whichever edition serves).

**Estate Cloudflare facts relied on.** Two Cloudflare accounts; the `security.cab` zone; an Access team domain already in use by the estate's other doors (commander, mcpx); the sanctioned `cf` binary from this repo is the only Cloudflare tool; credentials are purpose-scoped tokens held in Bitwarden by name (`doc/living/cloudflare-credential-architecture.md`) — the account-wide admin token is never placed in a Worker or runtime env. A lab Cloudflare Tunnel already exists (`lab.security.cab`) as tunnel precedent; the PM tunnel is a separate, PM-owned tunnel.

## 2. Target architecture

**One front door.** All human surfaces live under one canonical name — recommended: **`pm.security.cab`**, the name Drew and the Android app already use, and the name #14084 already chose for the one preferences surface. A front-door Worker (the `pm-console-edge` generalised, or a sibling Worker in the same deploy pattern) terminates every request for that name and routes by path:

- to the **hosted service** for the surface (pm-service or a sibling Worker) — tier 1;
- to the **tunnel origin** when the surface's route state says so (§5) — tier 2;
- machine paths (webhooks, intakes, mirrors) to their dedicated Workers/hostnames, never through the human door's policies.

The console origin already passes dashboard paths to the aggregator today, so path-unity is the estate's existing shape, not a new invention. The alternative — a second canonical name for the aggregator surfaces — is open decision D1 (§12); nothing else in this design depends on which way it goes, because the front door routes by path either way.

**The tiers, as components:**

| Tier | Serving components | Data read | Deployed by |
|---|---|---|---|
| 1 — Hosted | Front-door Worker; pm-service (board-family surfaces); decision-events Worker (decision surfaces); the unified push sender + registry reads (#14084); sibling Workers for accretion surfaces as they port | Tenant Turso DB (board family, #9060 model); fleet-platform Turso (alerts, push registry, unified sends ledger); D1 (decision events); KV/DO derived state (rebuildable only — never a data home, #9079 invariant 6) | GitHub Actions on merge to main (§7) |
| 2 — Tunnel | cloudflared (PM tunnel, config in repo, NixOS-declared) → console origin (:4872) + aggregator (:4870) on the laptop; beside-products at their laptop ports | The laptop editions, reading the same stores + the laptop collector's feed file | Laptop NixOS automation tracking main (§7) |
| 3 — Tailnet | The same laptop services at `amd-6.raptor-pumpkinseed.ts.net` (aggregator :4870 paths; console) | Same as tier 2 | Same as tier 2 |

**What deliberately does not move.** The queue broker and the other fleet brokers stay where the fleet programme puts them — the console's *views* of the broker migrate (as projections, §4), the broker does not. Kaneo/Pad/Grist data stores stay on their own programmes (#6816 stores-to-Supabase + laptop-off drill; #3142 Supabase migration). Board *write authority* is governed by the writer-of-record lease in every tier — serving tier and writer of record are separate facts, and no fallback in §5 ever silently moves write authority.

## 3. Per-surface targets — all reviewed surfaces

The review captured 37 screenshots and reviewed one further surface (the projector, API-only) in text: 38 rows. "Tier-1 home" is where the surface is served from in the target state; every row's tier 2 is the laptop edition via the PM tunnel and tier 3 the same at the tailnet name, unless the row says otherwise. Waves refer to the slice map in §11.

| # | Surface | Tier-1 home | Reason |
|---|---|---|---|
| 1 | `/` Overview | Hosted — pm-service via front door (wave M5) | The front door of the stack; reads feed projections + needs-you ledger, both hosted planes by then |
| 2 | `/today` | Hosted — pm-service (M5) | Same projections as Overview |
| 3 | `/digest` | Hosted — pm-service (M5) | Rollup over hosted projections; generation can move to a scheduled Worker later without changing the target |
| 4 | `/attention` | Hosted — pm-service (M6, interactive) | Needs-you is the stack's reason to exist; its answer path composes with #14043/#13469 (§8) |
| 5 | `/decisions` (index) | Hosted — decision-events side (M4) | The data is already hosted (Turso ledger + decision-events Worker); the half-migrated model, completed |
| 6 | Decision-set page | Hosted — decision-events side (M4) | Same pipeline as the index; serving moves to the Worker family that already takes the events |
| 7 | `/reports` | Hosted — pm-service (M5) | Reads the report store; the review found the store empty, not the page misplaced — placement is unaffected |
| 8 | `/board` | Hosted — pm-service (M5; #9079 slice 4) | The core #9079 parity surface: tenant-DB projections, `BoardDO` live state |
| 9 | `/roadmap` | Hosted — pm-service (M5; #9079 slice 4) | Tenant-DB roadmap rows; the Grist drift read stays a read of two hosted-side sources (§4) |
| 10 | `/conflicts` | Hosted — pm-service, read side (M7) | Serving can move; *resolution* stays read-only until #1282 slice 2's trust rulings — placement does not change that |
| 11 | `/quota` | Hosted — pm-service (M5; #9079 parity list) | Quota data already lands in the Turso estate (#6815); gatherers are the laptop part (§4) |
| 12 | `/quota/service/<id>` | Hosted — pm-service (M5) | Same read as /quota |
| 13 | `/dns` | Hosted — pm-service (M4) | The model page; a small static-ish read over hosted DNS/portfolio data |
| 14 | `/aa` Cost Explorer | Hosted — sibling Worker or pm-service (M5) | #9079 deferred it as an accretion; #14107 includes it. Snapshot data is already file/store-shaped and ports as a projection read |
| 15 | `/sessions` | Hosted — pm-service (M7) | 28k-record registry: needs the indexed projection read of M7, not a port of the laptop's cold-hang read |
| 16 | `/search` | Hosted — pm-service (M7) | Same: the target is a search over hosted indexes; re-creating the 33 s read at the edge is explicitly not the target (#14060, §9) |
| 17 | `/events` | Hosted — pm-service (M5) | Event stream over hosted projections |
| 18 | `/comments` | Hosted — pm-service (M4) | Small queue read; the review found it healthy — move it early and carefully |
| 19 | `/program` | Hosted — pm-service (M5) | Programme rollups over the same pool as /board |
| 20 | `/program/<id>` | Hosted — pm-service (M5) | Same read as /program |
| 21 | `/queue` | Hosted — pm-service (M5) | Board+pool reconciliation view over tenant-DB projections (not the broker's queue — the page must keep saying so) |
| 22 | `/health` | Hosted — front door (M1, then extended per wave) | The one surface that must exist before the others move: it renders per-surface serving tier, deployed commit + age per tier, and data-plane freshness — the migration's own instrument |
| 23 | `/alerts` | Hosted — front door over the alerts ledger (M4) | Data already hosted (fleet-platform Turso); pages laptop-bound today — the decision-sets move, again |
| 24 | `/alerts/preferences` | Hosted — redirects to the one preferences surface (M4) | Per #14084: the preferences surface lives on the console at pm.security.cab; this page redirects there after #14084 S4 |
| 25 | `/notifications` | Hosted — front door over the unified sends ledger (M4) | Reads #14084's unified ledger once S1/S2 land; until then it reads the legacy ledger wherever it is served |
| 26 | `/pages` | Hosted — front door (M1) | The directory must describe the *target* manifest, generated from the route table — a directory that reflects a stale build is how #14059 hid |
| 27 | `/item/<ref>` | Hosted — pm-service (M7) | Genealogy read model (#9079 slice 3 parity: `/api/item` ancestors + release) — pure derivation over the pool |
| 28 | `/discuss/<ref>` | Hosted — pm-service (M7) | Rides the item read; its comment write follows the write rules of §5 |
| 29 | `/projects` (redirect) | Hosted — front door (M1) | A redirect is front-door configuration, not an app page |
| 30 | `/chat` | Hosted — pm-service (M5) | Exists in main's manifest today; it 404s live only because the deployment is stale — M2 makes that class impossible before M5 serves it hosted |
| 31 | The 404 | Hosted — front door (M1) | The unknown-route page is front-door behaviour: app chrome, nav, and the serving tier named — never a bare browser default again |
| 32 | pm-console (Now / queue views) | Hosted — front-door surfaces over broker projections (M5/M6) | The broker does not move (fleet programme); its *views* read the same hosted projections as the dashboard, which also ends the two-stacks-two-numbers serving divergence (§9, #14063) |
| 33 | pm-dispatch (intake API) | Tunnel-tier now; hosted intake is open decision D6 | An intake API whose backend is the laptop broker; #13469's edge-intake pattern shows the hosted shape, but the call is the queue programme's as much as this stack's |
| 34 | pm-projector | Hosted twin exists by design — `ReconcileDO` (#12831) | The projector's hosted counterpart is already designed in #9079 slice 6; the laptop instance remains the tier-2 form |
| 35 | Kaneo | **Tunnel-tier is its target home in this migration** (M8) | A separate product (#9079 §4) with its own store programme (#6816/#3142); this migration gives it Access + the front door's name discipline, not a port |
| 36 | Pad | **Tunnel-tier** (M8) | Same as Kaneo; not reviewed past its login wall, and its own auth stays its own (§6) |
| 37 | Grist | **Tunnel-tier** (M8) | Same; its roadmap-source role is read by /roadmap from the hosted side once the Grist→projection publish exists (§4) |
| 38 | ready-for-agent | **Tunnel-tier** (M8) | A laptop app with no hosted counterpart designed; the review found it near-idle — whether it earns a hosted form is its owner's call, not this design's |

**The fleet feed** is not a page and gets no row: it is a data-plane artifact, placed in §4.

## 4. Data-plane placement

The rule is the decision-sets model, generalised: **data hosted, in the plane its owning design chose; pages read projections; anything derived is rebuildable and labelled with its age.**

| Data | Home (target) | Who writes | Notes |
|---|---|---|---|
| Board family: registry, item pool, links, Metadata copies, roadmap, releases, reconcile findings | Tenant Turso DB (#9060 model, #9083 tenant plane) | Hosted collector (#12826, Cron Triggers) once the writer-of-record lease flips per board; laptop collector until then | #9079 §3, full stop. D1 is not a board data home. `BoardDO` state is derived + rebuildable |
| Fleet feed (as a product) | Dissolves into projections: collector snapshots land in the tenant DB / fleet-platform stores; the feed *file* remains a tier-2/3 artifact of the laptop edition | Hosted collector primary; laptop collector fallback | The review's channel-dead vs collector-dead ambiguity becomes representable: snapshot age per section is a stored fact, rendered by /health |
| Alerts ledger, needs-you ledger, decision ledgers | Fleet-platform Turso (as today) | Aggregator/edge producers; the #14043 applier reads recorded answers from the ledger | Already hosted; the pages were the laptop-bound part |
| Push registry + unified sends ledger | Fleet-platform Turso (#14084 S1/S2) | The one sender; producers submit candidates | #14084 governs; this design only gives its surfaces a tier (§3 rows 24–25) |
| Decision-set events | D1 `decision-events` + per-set DOs (as today) | decision-events Worker webhook | Already hosted; the set *pages* join it in M4 |
| Console/broker views (Now, queue counts) | Projections published by the laptop broker-side exporter into the fleet-platform/tenant stores | The laptop (the broker is the source; it publishes snapshots the way the host already reports state to `PmEdge`) | Freshness is displayed, never hidden: when the laptop is off these views show last-known with its age — the honest shape the review asked for |
| Quota/usage | Turso estate (as today, #6815) | Gatherers: laptop today; hosted collector absorbs the API-shaped gatherers in M5; laptop-only gatherers keep publishing from the laptop | A gatherer that can only run on the laptop stays there — as a *writer to a hosted store*, not as a serving dependency |
| Kaneo / Pad / Grist stores | Unchanged (laptop Postgres/files) until #6816/#3142 move them | The apps themselves | This migration changes how a human *reaches* them (tunnel + Access), not where their data lives |
| Search + sessions registries | Indexed projections in the tenant/fleet-platform stores (M7) | Collectors/exporters | The index is derived and rebuildable; the 6.7 MB registry read does not move to the edge (§9, #14060) |
| Secrets for all of the above | Bitwarden, by name only | — | Webhook/intake/VAPID/Access credentials are referenced by name in every config; the account-wide CF admin token never enters a Worker (§1) |

## 5. Fallback mechanics — how a request actually moves down the chain

### 5.1 Tier 1 → tier 2 (hosted → tunnel): mechanised at the front door

The front-door Worker holds **route state per surface**: `hosted | tunnel`, stored in the same KV/DO config the edge already uses, flippable two ways:

- **Human flip** — the expose.py pattern, generalised: an operator (or a deploy pipeline, §7) sets a surface's route state; the flip is the rollback for a bad hosted deploy and the deliberate switch during each migration wave. Route state is data, visible on /health per surface — never a silent config buried in a dashboard.
- **Automatic fall (reads only)** — on a hosted read failure (the hosted service errors, times out, or its data-plane read fails), the front door re-issues the request against the tunnel origin and serves the laptop edition's response, marking the surface *degraded* (banner state + /health). Automatic fall is for idempotent GETs only, and it falls **to real laptop data or to an honest error** — if the tunnel is also down, the surface returns its unavailable page (app chrome, what failed, when it last worked), never a fixture (§0 rule 2; #14065).

**Health signals that feed the fall:** the hosted service's `/healthz` (checked at deploy time and continuously by the front door's scheduled probe); per-request error rate per surface; data-plane freshness (collector snapshot age per section, a stored fact per §4 — a hosted page reading a snapshot older than its surface's threshold renders the age, and past a second, harder threshold counts as a read failure for fall purposes). A surface that auto-falls repeatedly raises a stack-health incident through the alerts path — the fall is a symptom with an owner, not a silent new normal.

**Writes never auto-fall.** An answer, a decision submission, a preference change: these go to the hosted write path — the ledger/intake at the edge (#13469's answer intake; the `PmEdge` held-items pattern: the edge *holds* what waits for a machine) — and are applied by their applier (#14043) exactly once. If the hosted write path itself is down, the surface says so and keeps the user's input in front of them; it does not silently re-route a write to a different authority. Board-data writes additionally follow the writer-of-record lease (#9079 §5) in every tier: a tier may *serve* a board it may not *write*, and moving write authority is the lease flip procedure, never a side effect of a serving fallback.

### 5.2 Tier 2 → tier 3 (tunnel → tailnet): a bookmark, honestly

There is no mechanism — DNS, routing, or otherwise — by which a request to `pm.security.cab` can be failed over to `amd-6.raptor-pumpkinseed.ts.net` by the serving stack: they are different names in different DNS worlds, and tier 3 exists precisely for the case where the Cloudflare world (DNS, Access, tunnel connectors) is the thing that is down. So:

- Tier 3 is reached by **the human choosing the tailnet name** — from the tailnet (Drew's phone and machines are on it), the aggregator and console answer at the tailnet name exactly as they do today.
- The design's job for tier 3 is to make the bookmark *findable and trustworthy*: the hosted surfaces carry a standing "if this page is unreachable" note pointing at the tailnet name (in the app chrome, not in a runbook nobody can reach during the outage); /health on tier 3 renders the same deployed-commit + age as every tier, so a human on the fallback can tell whether the fallback itself is current.
- The M9 drills (§11) prove tier 3 by actually using it with Cloudflare paths disabled — a fallback that has never been exercised is a hypothesis.

### 5.3 What each outage shape looks like, end to end

| Outage | What Drew sees |
|---|---|
| Hosted surface broken (bad deploy, Worker error) | Deploy pipeline's health gate refuses the route flip, or the surface auto-falls to the tunnel edition; /health names the surface, the tier serving it, and the incident |
| Hosted data plane stale (collector down) | Pages render with visible snapshot age; past the hard threshold, reads fall to the tunnel edition; the staleness is an incident, not a banner that lives forever |
| Laptop off | Tier 1 serves everything hosted; laptop-sourced views (console projections, beside-products) show last-known data **with its age**, or unavailable — never invented. Push's dead-man's switch (pm-notify, once deployed) fires as designed |
| Cloudflare down (DNS/Access/Workers) | `pm.security.cab` unreachable; Drew uses the tailnet name (tier 3) and gets the full laptop stack. This is the one shape where the bookmark *is* the failover, and the design says so |
| Tunnel down, laptop up, CF up | Tier 1 unaffected. Tier-2-only surfaces (beside-products) are unavailable at the canonical name; tier 3 reaches them. Tunnel connector health is a /health row with its own incident |

## 6. Auth — Cloudflare Access posture per surface

The perimeter is **Cloudflare Access** on the PM names (the estate's Access team domain, already fronting commander/mcpx), and the app-level identity inside is the estate sign-in the hosted edition already consumes (#9079 slice 7: H/#9077 session → `binding_id`, role checks on every route, fail-closed). Access decides *who reaches the door*; the H session decides *who the app thinks this is*; neither replaces the other.

| Surface class | Posture |
|---|---|
| Human pages (all §3 hosted + tunnel-served pages) | Access application, owner-only policy (Drew's identity provider identity). No PM page is public. The workers.dev addresses of PM Workers answer health checks and public assets only — the console edge's existing rule, generalised to every PM Worker |
| The console origin's loopback exposure | Ends as a serving fact: the review found the console serving its full UI at loopback with no sign-in. Under this design the console origin is reached only through the tunnel (Access in front) or the tailnet door; loopback remains a local debugging path, not a surface |
| Beside-products (Kaneo, Pad, Grist) | Access in front of their tunnel hostnames; each app's own login stays as the second door (Pad/Grist were not reviewed past their login walls, and this design does not weaken or replace their auth) |
| Machine paths (decision-events webhook, alerts intake, notify-relay mirror/heartbeat, collector writes) | Not behind the browser policy: dedicated hostnames/paths with Access **service tokens** or the existing shared-secret-by-name auth each path already uses (names only: the decision-events webhook secret, the notify-relay shared secret, the alerts intake token, the VAPID private key). A machine caller must never need a browser login; a browser session must never satisfy a machine path |
| Decision links | The tailnet-hardcoded, bearer-token-in-URL link shape (review finding 13) is retired by the M4 slice that moves decision pages: links use the canonical name and the Access + H session; no credential travels in a URL again |
| Android app (TWA) | Wraps `pm.security.cab` as today; Access sign-in inside the TWA is the same owner flow. The app's push registration is #14084's endpoint story, unchanged by this design |

Access application granularity (one app for all PM names vs per-surface apps) is open decision D4 — the posture above holds either way.

## 7. Deployment pipeline — hosted == main, and the laptop tier follows

#14059's ask, made structural: **no PM surface may silently run a build that is not main.** Three mechanisms, one per place a build can hide:

**Hosted (tier 1).** Every PM Worker deploys the way `pm-console-edge` already does, generalised from one Worker to the family (front door, pm-service, decision-events, pm-notify when it deploys, and any sibling Worker a later wave adds):

1. GitHub Actions workflow per Worker (or one workflow, per-Worker jobs), triggered on push to main touching that Worker's paths — CI runs on main in this estate, and deploys ride the same event.
2. Build → deploy as a new version → the Worker's `/healthz` must answer and report the deployed commit == the merge commit → only then does the route flip (the expose.py pattern) put traffic on it. A failed health gate leaves the previous version serving and raises an incident; rollback is route removal, already the estate's shape.
3. The "deploy by hand" entries in `pm-stack/platform/workers.json` (decision-events, message-bridge class) are brought under this pattern for the PM-owned Workers; `workers.json` gains each Worker's last-deployed commit, kept current by the pipeline, not by hand.

**Laptop (tiers 2–3).** The fallback tier must not become the stale tier — a fallback to a nine-days-old build is a second outage wearing a rescue's clothes. The laptop editions get #14059's scheduled shape: a NixOS timer on `amd` fetches main, rebuilds the PM units, flips on success (the aggregator/console restart discipline the #13902 fix already exercised), and alarms through the alerts path on failure — and a **drift alarm**: a scheduled check compares each running unit's deployed commit against main and raises a stack-health incident when the gap exceeds a threshold (age or commit count), whether or not a rebuild is in flight.

**Rendered everywhere.** Every tier's /health (and the aggregator's `/api/health`, which already carries a `version` field) renders **deployed commit + age** permanently — #14059's second ask. The front door's /health (§3 row 22) aggregates: per surface — tier serving it, hosted commit, laptop commit, snapshot ages. A reader can answer "am I looking at current code?" from any tier in one glance; that question being unanswerable is what let 4,687 commits accumulate unnoticed.

## 8. Composition with the neighbouring designs

This migration is the serving home for three programmes already designed; it re-decides none of them.

- **Push unification (#14084).** Its registry and unified sends ledger live in the fleet-platform Turso store; its one preferences surface lives on the console at pm.security.cab; its sender consolidation retires the console's and edge's separate senders. This design's part: the surfaces get tiers (§3 rows 23–25), the preferences surface is a front-door surface at the canonical name, and #14084's S4 (edge cutover + app endpoint) lands *into* the front-door shape of §2 rather than into the console edge as a one-off. #14084's two open questions for Drew (the `needs`/`needs-drew` taxonomy; the preferences-surface home — Archer has ratified the console) are untouched by this design.
- **Answer write-back (#14043) + edge answer intake (#13469).** The write-back's applier consumes recorded answers from the ledger and applies them to the queue broker, exactly once, with failures raised as alerts. Placement consequence, and the only one: the applier and the broker are laptop-side today, so in a laptop-off world answers are *recorded* at the edge (intake, held) and *applied* when the broker path returns — the design of §5.1 already assumes exactly this split, and #14043's acceptance (a live tap applied with no seat in the loop) is unaffected. The write-back build slices (21653–21657) proceed on their own record; nothing here re-specifies them.
- **The #9079 hosted edition.** pm-service is the tier-1 vehicle for the board family (§3), its tenant-store/collector/API/dashboard/DO/conformance slices are this migration's M5 substrate, and its invariants bind every wave: same #9060 world, one writer of record per board, tenant DB is the record, DO state derived and rebuildable, no tokens in any DB. Where #9079 deferred a surface (push, AA, search, sessions), #14107's "fully" governs the *target* — the surface gets a hosted home — but the port is its own slice in §11, sequenced after the parity core, not a silent expansion of #9079's V1.
- **#3989** (move stateful services off the laptop + prove host failover) remains the umbrella programme for the estate-wide question; this design is the PM stack's serving answer under it, and M9's drills are the PM stack's contribution to its proof.
- **#6816 / #3142** own Kaneo/Pad/Grist store migration (Supabase). This design neither accelerates nor blocks them; if they land, §3 rows 35–37 are revisited by a new record, not by editing this one.

## 9. What this migration does and does not do to #14060–#14066

Per #14107's scope rule: the migration changes where pages are served from. It must not be used to close any of these by association.

| Issue | Affected? | Plainly |
|---|---|---|
| #14060 — search 33 s; registry ships 6.7 MB | **Not fixed.** Sequencing constraint only | The defect is read shape, not placement. M7 is forbidden from porting the unbounded read to the edge; the hosted search is built over indexed projections, which is #14060's fix shape arriving as M7's prerequisite — the issue closes on its own evidence |
| #14061 — send-ledger rows lack usable timestamps | **Not fixed** | The writer-stamp fix is #14084 S2's / #14061's own. The migration moves the /notifications *page* (M4) onto the unified ledger when it exists; the legacy ledger's repair stays #14061's |
| #14062 — /health contradicts itself | **Not fixed.** One input changes | The summary math and thresholds are code, wherever served. What changes: per-tier serving state, deployed commit + age, and per-section snapshot age become first-class health facts (§5.1, §7), and the collector-dead vs channel-dead distinction the review flagged becomes representable — the fix remains #14062's |
| #14063 — aggregator vs console counts contradict | **Partially — the serving half only** | One front door reading one projection set ends the *two serving stacks, two numbers* class (console views join the dashboard's projections, §3 row 32). Where the underlying *sources* disagree (board-derived vs broker truth), that reconciliation is #14063's and survives the migration untouched |
| #14064 — decision sets never age out | **Not affected** | An aging policy is a policy, not a placement. The hosted index inherits the same 89 pending sets until #14064's policy lands |
| #14065 — fixture fallbacks fabricate data | **Not fixed — but the requirement is set here** | Removing fixture fallbacks from the laptop edition is #14065's code work. This design makes fail-visibly a hard acceptance rule for every tier and every fall (§0, §5.1): no migration slice is accepted if any tier it touches can render invented data. If a slice finds fixtures in its path, it files/fixes under #14065, not silently |
| #14066 — /aa version picker duplicates | **Not affected** | Dataset-version duplication lives in the snapshot layer; /aa's move to a hosted read (M5) neither causes nor cures it |

## 10. PM book impact

The PM book (`pm-stack/book/`) documents the stack, including its serving shape — and today that shape is "everything on the laptop," which this migration exists to change. Per the book-live-update rule ("if the change, once merged, would make a sentence in a book false, the book is in scope for that change"), the book updates **in the slices that change serving facts** — M1 (front door + pipeline), M3 (tunnel tier), each wave that moves a surface, M9 (close-out) — in the same PRs as those changes. **This design record does not change the book**: at design time no serving fact has changed, and a book edited now would describe a stack that does not exist yet.

## 11. Slice map — named, ordered, NOT queued

Slices are packetised from this record after it lands, in the estate's usual way. Dependencies name both this map's slices and the neighbouring programmes'.

| Slice | Scope | Depends on |
|---|---|---|
| **M1 — Front door + deploy pipeline** | Generalise the console-edge pattern into the PM front door at the canonical name; every PM Worker under the Actions build→health-gate→route-flip pipeline; deployed-commit stamping; the /health tier/commit surface (§3 rows 22, 26, 29, 31 land here) | — |
| **M2 — Laptop edition follows main** | Scheduled rebuild+flip for the laptop aggregator/console; drift alarm vs main; deployed commit + age rendered on every tier's health | — (parallel with M1; together they close #14059's asks) |
| **M3 — Tunnel tier formalised** | The PM cloudflared tunnel as repo config + NixOS declaration; Access applications for the PM names; front-door route state + tunnel proxying live for not-yet-hosted surfaces | M1 |
| **M4 — Hosted surfaces, already-hosted data** | Decision index + set pages off the decision-events pipeline; /alerts + /notifications over the alerts/unified ledgers; the preferences redirect; /dns, /comments (§3 rows 5, 6, 13, 18, 23, 24, 25); decision links re-pointed to the canonical name (§6) | M1; #14084 S1/S2 (unified ledger) and S4 (preferences surface) for rows 24–25 |
| **M5 — pm-service read waves** | The feed-derived dashboard over tenant-DB projections: board, roadmap, program, queue, quota, events, overview, today, digest, reports, console views, /aa, /chat (§3 rows 1–3, 7–9, 11, 12, 14, 17, 19–21, 30, 32) | #9079 slices 2–4 (hosted collector, read API, dashboard surfaces); M4 |
| **M6 — Interactive surfaces** | /attention and the answer flows end to end at the canonical name: edge intake + held queue + applier, lease-respecting writes (§3 row 4; row 32's write side) | M3, M5; #13469 (edge intake), #14043 build slices (applier) |
| **M7 — Heavy surfaces** | /search, /sessions, /item, /discuss, /conflicts read side (§3 rows 10, 15, 16, 27, 28) over indexed projections | M5; #14060's read-shape work as prerequisite or first in-slice step |
| **M8 — Beside-products + APIs at the tunnel tier** | Kaneo, Pad, Grist, ready-for-agent behind Access at the front door's name discipline; dispatch's hosted-intake call taken (D6); projector ↔ `ReconcileDO` alignment (§3 rows 33–38) | M3; hosted moves only as #6816/#3142/#12831 land |
| **M9 — Tier-3 formalisation, drills, close-out** | Tailnet bookmark in the app chrome; laptop-off drill and Cloudflare-off drill executed and recorded; PM book serving chapters verified end to end; this record's close-out note | M4–M8 |

## 12. Open decisions — named, with deciders

| # | Decision | Decider | This design's recommendation |
|---|---|---|---|
| D1 | Canonical names: one front door at `pm.security.cab` for all human surfaces, or a second name for the aggregator family? | **Drew** (it is his daily surface and the Android app's home) | One door: pm.security.cab. The app, the preferences surface (#14084), and the console's existing path-passing already live there |
| D2 | Tier 1→2 posture for reads: automatic fall at the front door (as §5.1), or human-flipped route state only? | **Drew** (risk posture: a silent fall can mask a broken hosted tier behind a healthy laptop) | Automatic for reads, with the fall always visible on /health and incident-raising when repeated; writes never auto-fall (design rule, not part of this decision) |
| D3 | Store boundary for new projections: tenant plane (#9079) vs fleet-platform store (#14084) — which plane each new projection (console views, search index, session registry) lives in | **Archer**, at M4/M5 slice design | Follow the owning design per data family (board family → tenant; alerts/push → fleet-platform); name any new family's plane in its slice, never by drift |
| D4 | Access granularity: one Access application for the PM names vs per-surface applications | **Archer** (security posture), following the estate's existing Access patterns | One application per name class (human door, machine paths, beside-products) — coarse enough to audit, fine enough to revoke separately |
| D5 | The laptop edition's long-term status: maintained dual edition, or frozen fallback once hosted parity + conformance (#12833) pass? | **Drew**, after M5 lands | Decide with conformance evidence in hand; until then both editions are maintained, and M2 keeps the laptop one honest |
| D6 | pm-dispatch: hosted intake endpoint (edge-queued, #13469 pattern) or tunnel-only for the foreseeable future? | **Archer**, with the queue programme, at M8 | Tunnel-only until the queue programme wants an edge intake; the broker's placement is not this stack's to move |

## 13. Evidence and sources

- Direction + scope: #14107 (Drew, 2026-11-23).
- Surface inventory, defects, laptop-reliance map: the PM pages review, `doc/2026-11-24T03-51-11Z_review_pm-stack-pages/` (PR #14068; summary on #13900, comment 7121679379).
- Staleness root + the follow-main ask: #14059; console sibling #13833; deploy-target machinery #12768.
- Hosted edition design (data homes, parity list, writer lease, slices): `doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition/` (#9079, under #9078; data model #9060; tenant plane #9083).
- Push unification: #14084 + `doc/2026-11-24T04-45-43Z_design_pm-push-unification/`. Answer write-back: #14043 (design PR #14046); edge answer intake #13469.
- Current serving facts: `pm-stack/PORTS.md` (ports + the tunnel's path-passing), `pm-stack/console/edge/wrangler.jsonc` (front-door pattern, `PmEdge`, `EDGE_DIRECT`), `pm-stack/decision-events-worker/wrangler.toml` (D1 + DOs), `pm-stack/notify-relay/wrangler.toml` (designed, undeployed), `pm-stack/platform/workers.json` (estate Worker registry, checked 2026-11-23).
- Cloudflare credential discipline: `doc/living/cloudflare-credential-architecture.md` (purpose-scoped tokens by name; admin token never in a Worker).
- Umbrella + neighbours: #3989 (stateful services off the laptop), #6816 / #3142 (beside-product stores), #1282 slice 2 (conflict trust rulings).

*Secrets and credentials are referenced by name only throughout this record. No slice in §11 is queued; packetisation follows this record's landing, per #14107.*
