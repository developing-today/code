# Worker evidence — the three WEAK-EVIDENCE rows (lane WORKER-EVIDENCE, programme #13900)

**Settled: 2026-11-23.** Method: laptop clone `~/git/code` (HEAD `44aaf731ad2`)
read via `~/tools/laptop-ssh.sh` — `home/common/default.nix`, the door Workers'
wrangler configs, a bounded wrangler inventory (`find pkgs pm-stack scripts
infra nixos -name 'wrangler.*'`), `pm-stack/aggregator/`, `pm-stack/console/`,
`pm-stack/systemd/`, `pm-stack/deploy.sh`, `pm-stack/PORTS.md`,
`pm-stack/platform/workers.json`, `nixos/services/pm-stack/default.nix`, the
#13844 audit doc in `doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/`,
and issues #13404/#13657/#13774 read via `fleet gh api`. No fresh clone.

**Headline: all three rows are NOT Workers.** The census's worker count drops
by three (22 → 19 censused, 19 → 16 worker gaps; PM-book worker gaps 12 → 9).
All three documentation gaps stay assigned to the PM book — what changes is
what the chapters must describe (laptop services and a tunnel door, not
Workers).

---

## 1. The exec door (`exec.security.cab`)

**Census text:** "Third security.cab door alongside commander/mcpx (issues
#13404/#13657/#13774 family) — no wrangler config found in repo census;
documented only in home config/issues."

**What it actually is:** a path-scoped **Cloudflare Tunnel ingress hostname**
fronting the laptop's fleet queue broker — the public rendezvous for `fleet
actions exec` (#13774), the mode where GitHub Actions runner agents dial out
and register with the broker. There is no Worker in the path at all: no OAuth
front door, no bridge door, no Access app.

**Where it runs:** nowhere as code of its own. The hostname terminates at the
remotely managed sandbox-link tunnel and is forwarded straight to the laptop
queue broker at `100.99.113.103:4801`.

**Evidence:**

- Only two security.cab door Workers exist, each with a wrangler config:
  `desktop-commander` (`pkgs/desktop-commander/worker/wrangler.toml`,
  `name = "desktop-commander"`, header cites #13404; public door
  `commander.security.cab`, bridge door `commander-link.security.cab`) and
  `mcpx` (`pkgs/mcpx/worker/wrangler.toml`, `name = "mcpx"`, header cites
  #13657; doors `mcpx.security.cab` / `mcpx-link.security.cab`). The bounded
  wrangler inventory finds 23 configs across `pkgs/ pm-stack/ scripts/ infra/
  nixos/` — none for an exec door.
- `home/common/default.nix` mentions `security.cab` only for Commander (the
  `desktop-commander-bridge` unit comment, ~lines 1077–1080: supergateway on
  `127.0.0.1:8931`, bridge door `commander-link.security.cab`, citing #13404).
  The exec door appears in home config **not at all** — the census's
  "documented in home config" is wrong for this row; its record is #13774.
- #13404 is the Desktop Commander door build and #13657 the mcpx door build;
  neither is an exec door. #13774 ("fleet actions exec: live short-lived exec
  sessions inside Actions runners, coordinator-handed-off") is the exec
  record, and its Slice 5 comment (ACTIONS-EXEC-2, 2026-11-23T22:10:06Z)
  records the door as deployed, first-hand:
  - the sandbox-link tunnel (`722f72ba-…`, remotely managed) at config
    version 6 gained an ingress rule — `exec.security.cab` with path
    `^/actions-exec-(register|poll|io|result|close)$` →
    `http://100.99.113.103:4801` (the queue broker) — inserted before the
    catch-all, all existing rules preserved;
  - DNS CNAME `exec.security.cab` → `722f72ba-….cfargotunnel.com` (proxied)
    created in the security.cab zone;
  - **no Access app in front, by design** — the runner job's GitHub OIDC
    token is the authentication, verified at the broker;
  - verified from outside (Muse sandbox): `POST
    https://exec.security.cab/actions-exec-poll` with no token returned the
    broker's own `401 {"error":"unauthorized"}`; `POST /claim` and `GET /`
    on the same host returned 404 — path scoping enforced at the tunnel.
- In code, the door is a broker URL, not a Worker:
  `pkgs/fleet/crates/fleet/src/actions.rs:92` —
  `const DEFAULT_BROKER_URL: &str = "https://exec.security.cab";`
  A repo-wide grep for `exec.security|exec-door|exec_door` over
  nix/toml/json/jsonc/md/ts files returns exactly one further hit — a
  fleet-book reference line (`pkgs/fleet/book/reference/actions.md:87`,
  "OIDC through the public door at `exec.security.cab`…") — and no config
  of any kind.
- Why no repo config exists: the tunnel is remotely managed, so its ingress
  lives in Cloudflare, not in the repo. There is no Worker to configure, so
  there is nothing a wrangler config could say.

**VERDICT: NOT-A-WORKER.** It is a path-scoped tunnel ingress
(`exec.security.cab/actions-exec-*` → laptop queue broker :4801, OIDC-auth at
the broker). Row status changes: it leaves Table A's worker census; the
documentation gap stays **GAP→PM book**, now as a tunnel-door / actions-exec
chapter rather than a Worker chapter.

---

## 2. The pm-stack aggregator "worker"

**Census text:** "Audit (#13844) names the aggregator as a Cloudflare Worker
writing `fleet-platform` via the Turso HTTP API — no wrangler config in
`pm-stack/aggregator/` in repo census (deploy path unverified)."

**What it actually is:** the PM dashboard application itself — a **Bun /
TypeScript server on the laptop** (`pm-aggregator-ui`), a read-mostly
projection over the projector/link registry, Kaneo, Pad and Grist, with the
alert/push engine inside it. The audit got the store and the API right and
the runtime wrong: it does write `fleet-platform` through the Turso HTTP
API, but from the laptop, not from a Worker.

**Where it runs:** laptop host `amd`, as a systemd user service (now
NixOS-packaged), listening on port **4870** (loopback + tailnet) with a
freshness IPC port 4871. No public route of its own; the public face is the
console/edge surface in §3.

**Evidence:**

- `pm-stack/aggregator/` contains **no wrangler config** (only `bunfig.toml`
  among config files). `pm-stack/aggregator/package.json`: name
  `pm-aggregator-ui`, scripts `start`/`dev` = `bun run src/server.ts`. The
  bounded wrangler inventory confirms no config anywhere for an aggregator.
- Laptop service, two generations of the same fact:
  `pm-stack/systemd/pm-aggregator.service` — `WorkingDirectory=
  %h/git/code/pm-stack/aggregator`, `ExecStart=/run/current-system/sw/bin/bun
  run src/server.ts`, "Listens on port 4870", env from
  `%h/.local/state/pm-aggregator/.env`; and the current NixOS module
  `nixos/services/pm-stack/default.nix` — `pm-aggregator` unit running the
  packaged build (`pkgs/pm-aggregator/package.nix`, built from
  `pm-stack/aggregator` source), `aggregatorPort` default 4870, socket units
  on `127.0.0.1:4870` + tailnet:4870 and freshness 4871.
- `pm-stack/PORTS.md` (authoritative laptop port registry): "4870 |
  pm-aggregator (dashboard HTTP) | loopback + tailnet | … | live"; "4871 |
  pm-aggregator freshness IPC".
- `pm-stack/aggregator/install.sh` header: "Install/update the aggregator
  **on the laptop**… installs the repository-owned user systemd units… fails
  loudly unless /api/health reports the build version it just deployed."
  `pm-stack/deploy.sh` deploys the pm-stack's Docker side (Kaneo) and renders
  the tunnel token files on the host — it deploys no aggregator Worker.
- The Turso writing is real, from this laptop process:
  `pm-stack/aggregator/src/turso-stores.ts` (store client, ref
  `turso:fleet-platform`), `src/decision-events-sync.ts` —
  `DECISION_EVENTS_TURSO_URL =
  "https://fleet-platform-dezren39.aws-us-east-1.turso.io"`, token resolved
  by name `turso.fleet-platform-rw-token`; alerts/incidents and push
  subscriptions live in Turso-backed `alerts.*` tables in the same process.
- PM-SURVEY's `survey/push.md` (2026-11-23, same clone): "the aggregator
  **on the laptop** is the only detector/decider for ordinary classes; it
  sends Web Push (VAPID) directly to subscribed browsers/phones" — the
  survey and this lane agree; the audit's "Worker" label is the outlier.
- `pm-stack/platform/workers.json` ("Every Cloudflare Worker this repository
  holds", checked 2026-11-23) lists the pm-stack Workers as `pm-console-edge`,
  `decision-events`, and `pm-notify` (state "not deployed"). No aggregator
  entry exists.
- The audit text itself (`doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/`,
  line 55): "`fleet-platform`… Its owners are Cloudflare Workers in
  `pm-stack/` (aggregator, dashboard, decision-events) writing through the
  Turso HTTP API." Of the three names, only `decision-events` is a Worker
  (`pm-stack/decision-events-worker/wrangler.toml`).

**VERDICT: NOT-A-WORKER.** It is the laptop Bun service `pm-aggregator`
(systemd/NixOS, port 4870). Row status changes: it leaves Table A's worker
census; the documentation gap stays **GAP→PM book** (the PM book's aggregator
chapters describe this laptop service).

---

## 3. The pm-stack dashboard "worker"

**Census text:** "Audit names the dashboard Worker as reader/writer of
`fleet-platform` ingest families — no wrangler config found in repo census."

**What it actually is:** not a separate thing at all. **"The dashboard" is
the aggregator's other name** — the same laptop service as §2. The audit
(§G2: the ingest families are "written by Worker collectors and read by the
dashboard Worker"; line 55's owner list) split one laptop application into
two Workers that do not exist.

**Where it runs:** the laptop, port 4870 — identical to §2.

**Evidence:**

- `pm-stack/aggregator/README.md` line 3: "The PM-stack **aggregator UI** —
  **the dashboard Drew lives in** while the fleet works in GitHub, Kaneo,
  and Pad underneath."
- `pm-stack/PORTS.md`: the 4870 row names the service "pm-aggregator
  (dashboard HTTP)" — one row, one service, both names.
- `pm-stack/console/src/server.ts` header: "pm-console: the write side of
  the PM stack. **The dashboard (pm-aggregator) shows the fleet; this
  service acts on it.**" `pm-stack/console/src/config.ts`:
  `dashboardUrl` defaults to `http://127.0.0.1:4870`
  (`PM_CONSOLE_DASHBOARD_URL`) — the console fronts the dashboard at that
  loopback address for a verified owner.
- No dashboard wrangler config exists anywhere in the bounded inventory, and
  `pm-stack/platform/workers.json` (checked 2026-11-23) has no dashboard
  entry.
- The only Worker anywhere in the dashboard's public surface is
  `pm-console-edge` (`pm-stack/console/edge/wrangler.jsonc`,
  `name = "pm-console-edge"`, route `pm.security.cab` attached by host
  `scripts/expose.py`) — already censused as its own Table A row, so it
  cannot also be this row. Its host half, `pm-console`, is itself a laptop
  Bun service (`pm-stack/systemd/pm-console.service`, port 4872 per
  PORTS.md) that passes dashboard paths through to 4870. So even under the
  loosest reading of "dashboard" (the console surface), there is no
  additional Worker: laptop service + the already-counted edge Worker.

**VERDICT: NOT-A-WORKER.** It is a duplicate row: "dashboard" = the
aggregator UI (laptop Bun service, port 4870). Row status changes: it leaves
Table A's worker census as a duplicate of the aggregator row; its
documentation gap is the aggregator's gap, **GAP→PM book**, not a separate
one.

---

— Lane WORKER-EVIDENCE, 2026-11-23. Coordinator: PM-BOOK (#13900).
