# Design (component C): fleet tool, Workers/TypeScript equivalent — hosted Metadata stamping/query, session provenance, sync tools

Status: design record for issue #9081, component **C** of epic #9078 (hosted
editions program). Docs-only; no implementation in this record.
Author: fleet agent `felix` (kind muse), queue task 20284
(queue repo-health-code-pilot), 2026-11-22.
Written against `origin/main` at `cd9e579c13`.

Basis, and the inputs this record must answer rather than restate:

- Issue #9081 §What + its four open design questions (split of duties,
  stamping parity, auth/non-forgeability, queue semantics), and its
  acceptance (capability inventory vs the native fleet surface, hosted
  stamp set, auth model, queue answer; granular implementation issues
  filed under #9081).
- The epic doctrine (Drew amendment, 2026-11-12 17:03 CST, on #9078 and in
  the intake scaffold `doc/2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md`):
  Cloudflare+Turso and the Rust binaries are **both tier 1** — parity +
  deliberate divergence, never "port down"; a self-hosted server is a
  **peer participant kind**, not a special case. Statements S4, S14, S18,
  S19 of the scaffold's inventory are the ones this component carries.
- The native semantics that carry over, all specified in
  `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md`
  (the #9060 design): §12 the Metadata block and its `provenance` object,
  §13 `fleet gh` as a superset of `gh` (create-path stamping + DB insert),
  §14 session state, §15 the metadata database, §16 dual ingestion.
- Settled neighbours this record aligns with and does not override:
  component E's tenancy design
  (`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture/`) — central
  tenancy DB holds accounts/roles/credential metadata, **no user content,
  no private keys**; per-user data reached only through scoped, expiring
  Turso tokens minted by a tenancy Worker from a tenancy row + an
  authenticated session. Component H (#9086) owns the per-surface
  token/session flows and the agent credential model; #9077 owns identity
  itself. Neither is re-designed here.
- Design inputs re-checked at writing time (2026-11-22): #9073, #9074,
  #9075, #9076, #9077, #9016, #9079 (A), #9080 (B), #9085 (G) are all still
  **OPEN**. Their semantics are specified but unimplemented natively; this
  design therefore fixes the hosted shape against the *specifications*,
  and §8 names the points where a native implementation landing first
  would supersede a choice made here.

## 0. Shape in one paragraph

One hosted service — a single Cloudflare Worker, working title
**fleet-hosted** — is the hosted world's fleet front door and its **only
stamping authority**: it parses, stamps, and required-field-checks
Metadata server-side, performs the GitHub create through the fleet App's
installation-token rotation held server-side, and inserts the full record
into the metadata database itself. A **thin TypeScript CLI** holds no
authority: it resolves session context, assembles the caller's body and
caller-owned fields, calls the service over REST, and renders results.
The **queue is not re-implemented**: the queue core already lives in a
Turso database behind the broker protocol (#6741); the hosted world gets
a second, peer front door onto that same database and the same
`repo-task/v1` packet schema — the fleet-hosted Worker speaking the
broker protocol — rather than a second queue. Callers authenticate with
#9077 relying-party tokens (agents with #1253 agent-role credentials);
per-user Turso credentials from component E are a data-plane read
mechanism and can never mint provenance. Stamps are non-forgeable because
every fleet-owned provenance field is filled by the service from sources
the caller does not control (its own clock, the queue record, its session
registry, the validated token), and the resulting provenance object is
**signed** by the stamping authority and verifiable by the collector and
the query surface.

## 1. V1 — what exists today (measured, not assumed)

The capability inventory below is read off the shipped native surface
(`pkgs/fleet/docs/verbs.md`, the `fleet-registry` domains) and the code,
not off the design documents:

- **`fleet gh` today is step 1 only**: a passthrough to the installed
  `gh` with token rotation (personal token, then the developing.today
  fleet App's installation tokens, failover on rate-limit/permission
  refusals; #8887, PR #9014) plus a token cache and closure records. The
  Metadata-aware layer of #9060 §13 — parse, stamp, required fields, DB
  insert — is **not implemented**; it is #9073, still open. Likewise
  session state (#9075), the metadata DB + query surface (#9074), and
  dual ingestion (#9076) are specified-only. The hosted design is
  therefore not a port of running code; both editions implement the same
  §12–§16 semantics, and whichever lands second conforms to the first's
  fixtures (§6).
- **The queue core is already hosted data**: `fleet-queue`'s backend is
  a dedicated Turso database (libSQL) behind the broker protocol — tasks,
  leases, lanes, workers, checkpoints, the lane ledger, and the
  `agent.events` stream (#6741; `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs`).
  Claim atomicity is one conditional UPDATE under libSQL's single-writer
  serialization. The "laptop queue" is a *broker process location*, not
  where the data lives. Packets validate against
  `pkgs/fleet/repo-task-v1.schema.json` (`$id: repo-task/v1`).
- **A hosted fleet front door already exists as precedent**: the
  `bridge` domain (agent messaging: `hello`/`inbox`/`post`/`ack`) is
  served through a Cloudflare Worker location today — agents register
  and exchange messages without a laptop broker in the path. Component C
  is the second hosted front door, not the first.
- **Native stamp set** (#9060 §12, the `provenance` object of
  `pm_meta: 1`): `queue_id`, `session_id`, `lane`, `host`, `worktree`,
  `picked_up_at`, `work_started_at`, `created_at`, `model`,
  `lane_start_issues`. §13 adds what the create path knows: the queue
  item's **scheduled start**, the current time, and the caller's **pwd**.
  Caller-owned block fields are `direct_parent`, `links`, `lineage`;
  #9073's rule is that caller-supplied values are never overwritten.
- **Session state today** is a *specification* (#9060 §14 / #9075): each
  fleet start assigns a session id, carried in the environment of
  everything fleet launches and in a pwd-file at the worktree root;
  resolution order explicit argument > env > pwd-file. No crate in
  `pkgs/fleet/crates/` implements the pwd-file mechanism yet.
- **Sync tools** (#9016) are a triage item, not a build: step 2's
  acceptance is a sequenced step list (project sync, queue/GitHub sync,
  possibly a shared gh-domain crate). No steps are filed yet; §7.2
  consumes that triage rather than duplicating it.

## 2. Capability inventory vs the native fleet surface

Disposition of every native domain (per `pkgs/fleet/docs/verbs.md`) for
the hosted edition. "Hosted" = served by fleet-hosted and/or the TS CLI;
"native-only" is a deliberate divergence, named as such per the doctrine.

| Native domain | Hosted disposition | Notes |
| --- | --- | --- |
| `gh` — rotation passthrough | **Hosted, transformed** | The rotation point moves server-side: the service holds the fleet App installation tokens; callers never see a GitHub token. Plain passthrough is *not* offered hosted — every hosted GitHub write goes through a verb that can stamp (§3). Divergence: deliberate (a raw passthrough would bypass the stamping authority). |
| `gh` — Metadata create/query layer (#9073/#9074) | **Hosted core** | This design's centre: §3 (create/stamp), §7.1 (query). |
| `queue` — broker verbs (claim/heartbeat/complete/…) | **Hosted front door** | Same Turso queue DB, same `repo-task/v1` schema, broker protocol over hosted transports (§6). |
| `queue fallback` — GitHub-issue claim markers | **Hosted via service** | The service performs the identical marker-comment protocol on the caller's behalf; marker format unchanged so native and hosted claims interlock. |
| `agent` — front door (`next`, shortlist, doctor) | **Hosted (intake subset)** | `next` and claim are service calls (the intake ladder's queue + GitHub-priority rungs work hosted; the local-shortlist rung is a laptop-local file and stays native). `doctor` reports hosted-leg reachability. This is how agent workflows off the laptop — the primary consumers of C — take work. |
| Sync tools (#9016 scope) | **Hosted verbs** | Project sync and queue/GitHub sync as service endpoints with CLI verbs (§7.2). |
| `bridge` | Already hosted | Precedent, out of C's scope; no change. |
| `search` (Atlas) | Already a service | Out of scope. |
| `secrets` | **Native-only (divergence)** | The secrets broker is a local trust domain. Hosted callers use Workers secrets and tenancy-issued credentials (E). No hosted `secrets get`. |
| `shell` / `laptop-agent` / `queue dispatch` | **Native-only (divergence)** | Execution placement runs on native runners. A hosted caller *requests* execution only by enqueueing a task; it never gets a shell. |
| `proxy` | **Native-only (divergence)** | Tailnet/session plumbing has no Workers analogue. |
| `serve` (broker roles) | Transformed | The hosted service *is* the serve role for its front doors, as a Worker; the queue-broker role additionally gains the hosted endpoint (§6) rather than a second broker implementation. |

## 3. Question 1 — split of duties

**Answer: the service stamps and inserts; the CLI carries and renders;
direct Turso is read-only and never on the stamp path.**

### 3.1 The hosted service (fleet-hosted Worker) owns

- **Stamping authority.** Metadata parse, stamp, and required-field
  enforcement with #9073's semantics, executed server-side: the service
  fills every fleet-owned provenance field from its own sources (§5)
  and preserves caller-owned fields (`direct_parent`, `links`,
  `lineage`, and any caller-supplied provenance it is *not* authoritative
  for — none, by §5's table) untouched.
- **The GitHub write.** Issue/PR creation through the fleet App
  installation-token rotation, held as Worker secrets. This is the
  hosted rotation point: #9016's "one rotation point" property is
  preserved, relocated server-side.
- **The DB insert.** The create path inserts the full record (full
  Metadata JSON + extracted keys, #9074's schema) into the metadata
  database in the same request that creates the GitHub item. A failed
  insert never loses the GitHub item: ingestion (§7.3) heals it — the
  §16 contract, unchanged.
- **The session registry** (§4.2), **the query surface** (§7.1), **the
  hosted queue front door** (§6), **the ingestion receiver** (§7.3),
  and **the sync verbs** (§7.2).
- **Stamp signing** (§4.3).

One Worker with route modules, not several Workers: the stamping,
query, queue-door, and ingestion routes share the auth validation, the
tenancy lookup, and the DB clients, and component A's service-shape
question (#9079 Q1) is answered independently for A — C does not
prejudge it. Durable Objects are used only where coordination needs a
single writer: none of C's routes need one at V2 (the queue's atomicity
lives in libSQL, the metadata DB's in its own writes); if rate limiting
per tenant is added, it follows E's TenantDO pattern rather than
inventing a second one.

### 3.2 The thin TS CLI owns

- Session resolution and context carriage: explicit argument >
  environment > locally stored session state — the #9075 resolution
  order with the pwd-file replaced per §4.2. The CLI is where
  `FLEET_SESSION_ID`-equivalent state lives for hosted callers.
- Assembling create requests: the caller's body text, caller-owned
  Metadata fields, and *claims* the service may verify (repo, branch) —
  never timestamps, never queue fields, never the session's provenance:
  anything fleet-owned in a CLI request is ignored by the service, so a
  buggy or hostile CLI cannot mint provenance.
- Rendering: query results, claim results, and errors in the shapes
  agent workflows consume (JSON first, like the native CLI).
- Runs anywhere Node/Workers-compatible TS runs: a developer machine, a
  Codespace, an agent runtime (OpenCode session hosts), or another
  Worker (service-to-service, with a service token per §4.3).

### 3.3 Direct Turso access — C's prudence rules

Component G (#9085) owns the program-wide prudence statement; these are
the rules as they bind component C, stated so C's implementation issues
can be worked before G lands, and G's design supersedes them if it
rules differently:

1. Direct Turso access from a hosted caller is **read-only**, against
   the **metadata database only**, with a scoped, expiring token minted
   by the tenancy plane (E's model) for that account. It exists for
   high-volume query consumers (the collector, component A's dashboard)
   for whom REST-per-query is the wrong shape.
2. **Never direct**: the create/stamp path (stamping is the service's
   authority — a direct insert could forge provenance), the queue
   database (all queue access goes through a broker front door, native
   or hosted, so claim semantics and event writes stay in one code
   path per edition), any cross-tenant read, and any write at all.
3. A direct-Turso token is a *data-plane* credential. It is not an
   authentication to the service and confers no stamping or queue
   rights (§4.3).

## 4. Question 3 first (it constrains §5) — auth and non-forgeability

(Answered before stamping parity because the stamp set's new fields are
auth outputs.)

### 4.1 Who may call

- **People and agent runtimes** authenticate to fleet-hosted with
  **#9077 relying-party tokens**: the caller signs in through the
  devtoday OIDC provider (which itself accepts the linked GitHub /
  Google / Microsoft doors), and presents the resulting token to the
  service. The service validates it against the provider and maps the
  subject to a **tenancy account** (E) — stamps record the tenancy
  `binding_id`, never an email or a provider subject (E's keying rule).
- **Agents** authenticate as #1253's **agent role** with agent
  credentials issued and scoped per H (#9086 Q4): an agent credential
  names its account, its lane, and its session; the service treats the
  credential's claims as the ceiling of what the caller may assert.
- **Service-to-service callers** (component A's PM service, the
  ingestion receiver's internal calls) use **service tokens** recorded
  in the tenancy DB, one per caller, scoped to named routes.
- **Per-user DB credentials are not service authentication** (§3.3.3).
- The Drew-only gate of #9077's first phase is inherited, not
  re-implemented: the service accepts exactly the accounts the tenancy
  plane marks active; relaxing the gate is a tenancy change (H #9086
  Q3), not a fleet change.

Per-surface token *flows* (PKCE vs device flow for headless agent
hosts, session lifetimes) are H's design surface (#9086 Q1); this
record fixes only what C requires of them: a bearer token the service
can validate without a callback to the caller, carrying account, role,
and — for agents — lane/session claims.

### 4.2 Session provenance in the hosted world

Native session state is env + worktree pwd-file (§1). Hosted, there may
be no worktree and no persistent process environment, so the session
record moves server-side:

- A session is **opened against the service** (by the CLI, or by an
  agent runtime at lane start): the service mints the session id,
  records account, lane, declared model, open time, and the lane-start
  issue set when known, and returns the id. The record lives in the
  fleet metadata database's session table (the same DB #9074 builds;
  schema addition is an implementation issue under #9081).
- The CLI carries the id exactly as native resolution prescribes —
  explicit argument > env > stored state — where "stored state" is the
  CLI's per-workspace state file, the hosted analogue of the pwd-file:
  same recovery property (a later bare invocation in the same workspace
  recovers the session), without presuming a git worktree exists.
- A Worker-side or service-to-service caller passes its session id per
  request; the service resolves it against the registry. An unknown or
  closed session id is refused on the stamp path, so a stamp can never
  cite a session the service did not open — the hosted form of #9075's
  "stale pwd-files do not silently misattribute new work" acceptance.

### 4.3 Non-forgeability, in three layers

Anyone can edit an issue body, so a Metadata block alone proves
nothing. The hosted model makes provenance verifiable:

1. **Server-side sourcing.** Every fleet-owned provenance field is
   filled by the service from a source the caller does not control:
   times from the service clock, queue fields from the queue record,
   session fields from the session registry, identity from the
   validated token. This *narrows* #9073's "caller-supplied values
   never overwritten" rule, deliberately: that rule continues to govern
   caller-owned fields; fleet-owned provenance is service-authoritative
   in the hosted world because the caller is outside the trust
   boundary. On the laptop the binary is inside it — the divergence is
   the trust boundary, named, not a semantics change: the *fields and
   their meanings are identical*.
2. **Stamp signature.** The service signs the canonical provenance
   object and writes the signature into the block (`stamp_sig`, §5)
   and the DB row. Verifiers — the collector, the query surface, any
   reader — can check that a block's provenance was minted by a fleet
   stamping authority and not hand-written or edited afterward. The
   native create path should sign identically (same canonical form), so
   verification is edition-blind; that parity hook is filed as an
   implementation issue under #9081 cross-linked to #9073 rather than
   silently expanding #9073's scope here.
3. **Insert authority.** Only the service and the native binary hold
   write credentials to the metadata DB. A forged or edited body block
   therefore diverges from the DB row, and §15's rule already governs
   divergence: it is **reported**, body wins on content, and provenance
   queries are served from the DB — so a forged block gains its author
   nothing on any fleet surface.

Key custody (algorithm, service signing key vs a fleet-CA-issued
certificate) is **deferred** (§8): the field and the requirement are
fixed here; the algorithm is owned by the fleet CA line
(`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets`) with
#9073's implementation.

## 5. Question 2 — stamping parity: the hosted stamp set

Field-by-field, against the native set of §1. Rules: meanings never
change; additions are additive only (the block stays `pm_meta: 1` —
§12's format decision allows arbitrary keys, and the collector parses
additively); a field with no hosted existence gets a named replacement,
never a silent null with a different meaning.

| Native field | Hosted source / replacement |
| --- | --- |
| `queue_id` | Same. Read by the service from the queue record (§6's one queue). Null when the work is not queue-originated — same rule as native. |
| scheduled start (§13) | Same: the queue packet's scheduled start, read from the queue record by the service. |
| `picked_up_at` | Same: the claim record's pickup time, from the queue DB via the front door — not from caller assertion. |
| `work_started_at` | Same: the claim/bind record's work-start time. Where a hosted caller works without a claim, the session-open time (§4.2) is the service-sourced fallback, and the field's source is distinguishable via `stamp_sig`-covered `stamped_by` (additions below). |
| `created_at` | The service clock at stamp time. This is the point of server-side stamping: one clock authority for the hosted world, instead of every caller's clock. |
| `session_id` | The server-minted session id (§4.2), resolved by the service against its registry. |
| `lane` | The lane claim of the caller's agent credential, or the session record's lane. |
| `model` | The session record's declared model (declared at session open, as native declares it from the serving leaf). |
| `lane_start_issues` | The session record's lane-start issue set, or the queue record's, read by the service. |
| `host` | The participant that ran the work, per the peer principle: for a CLI caller, its machine host (Codespace name, laptop name, agent-host name); for work executed by the service itself (sync verbs, ingestion), the service's deployment identity. Host names a participant kind + instance, never a special "ours" value (S19). |
| `worktree` | **Replaced when no checkout exists.** A CLI caller on a real checkout stamps `worktree` exactly as native (path). A caller without one (Worker, agent runtime without a checkout) leaves `worktree` null and stamps the replacement field `workspace_ref` (below). |
| pwd (§13) | Subsumed by `workspace_ref.path` when a filesystem exists; absent otherwise. Never a required field hosted. |

**Additive fields** (both editions may stamp them; native leaves them
null until #9073's signing hook lands):

- `workspace_ref`: `{kind: "worktree" | "session" | "codespace" | "worker" | "service", id, repo, branch, sha}` — where the work
  happened, in the strongest form the caller's world supports. For a
  worktree caller it *accompanies* `worktree` (id = the path); for a
  Worker it is the only locator. `repo`/`branch`/`sha` are included
  whenever a checkout exists, which is what most consumers of
  `worktree` actually wanted from it.
- `auth_subject`: the tenancy `binding_id` of the authenticated account
  (or the agent credential's account) that requested the stamp.
- `stamped_by`: the stamping authority — native binary identity/version
  or the hosted service's deployment id — so a reader can tell which
  edition minted a stamp and which sourcing rules applied (notably to
  `work_started_at`). This is provenance about provenance, and it is
  what keeps the §4.3.1 divergence honest instead of silent.
- `stamp_sig`: the §4.3.2 signature over the canonical provenance
  object (all fields above, caller-owned fields excluded).

## 6. Question 4 — queue semantics

**Answer: one queue. The hosted world gets a front door, not a queue.**

- The queue core already lives in the fleet-queue Turso database behind
  the broker protocol (§1, #6741). There is no "laptop queue" to remain
  the only queue — there is a laptop-*hosted broker process* in front
  of hosted data. The design therefore adds a **second broker front
  door**: the fleet-hosted Worker exposing the broker protocol over the
  hosted transports (REST per verb; WebSocket where a verb streams —
  the per-verb framing is G's transport inventory, §8), authenticating
  callers per §4.1 and stamping claim provenance server-side.
- **Same packet schema**: `repo-task/v1`
  (`pkgs/fleet/repo-task-v1.schema.json`) validates identically at both
  doors; validation code is shared semantics, with the TS door's
  validator generated from or fixture-checked against the same schema
  (§6 discipline below). Same dedupe and work-identity rules (#10627),
  same lane ledger, same `agent.events` stream — one event stream is
  what keeps hosted and native claims visible to each other.
- **Atomicity is unchanged** because it never lived in the broker: it
  is the conditional UPDATE in the Turso backend under libSQL
  single-writer serialization. Two front doors cannot double-claim,
  any more than two native broker clients can; the doors are stateless
  protocol endpoints and peers (S19).
- The **fallback claim path** (GitHub-issue markers) carries over
  through the service (§2), so a hosted lane and a laptop lane claim
  the same issue pool with interlocking markers.
- Deliberate divergence, named: the hosted door *requires*
  authentication and refuses anonymous claims; the native door keeps
  its local trust model. Claim *semantics* are identical; the trust
  boundary differs, as with stamping (§4.3.1).

### Shared-semantics discipline (with G)

One data model across two implementations is kept honest by: the
`pm_meta: 1` block format and `repo-task/v1` schema as the shared
contracts (both already language-neutral JSON); a shared fixture corpus
(golden packets, golden Metadata blocks, golden stamp inputs/outputs)
that both the Rust and TS implementations test against — the fixture
corpus is an implementation issue under #9081; and the rule that a schema
change lands as a schema PR first, implementations second. G (#9085)
owns the program-wide statement of this discipline; this is its
instantiation for C's two contracts.

## 7. The remaining hosted surfaces — query, sync tools, ingestion

*Section order note: §§3–6 answer the issue's questions 1, 3, 2, 4 in
dependency order (auth constrains the stamp set). This section covers
the rest of #9081's "What".*

### 7.1 Metadata query surface (hosted)

#9074's surface, hosted: list/filter issues and PRs **by Metadata
fields** — direct parent, session, lane, host, queue id, link
status/relation pairs — served from the metadata DB, with GitHub as
fallback for fields not yet ingested (§13's rule). Two access shapes:
REST through the service (authenticated, §4.1) for interactive and
agent callers, and the direct-Turso read path (§3.3) for bulk
consumers. Body-vs-DB divergence is reported on the surface, per §15;
the hosted surface adds no new divergence semantics.

### 7.2 Sync tools (hosted form of #9016)

#9016's scope candidates become hosted verbs, so hosted and agent
callers stop shelling out to raw `gh` exactly as native callers are
meant to:

- **Project sync**: the writes `project-reconcile.sh` computes (status
  sets, item adds) as a service endpoint taking typed ops, executed
  server-side over the rotation, replacing per-call `gh project`
  invocations and outbox file shapes for hosted callers. (The native
  verb is #9016's own step; the hosted endpoint implements the same op
  set against the same semantics — one op vocabulary, two doors.)
- **Queue/GitHub sync**: the recurring scripted flows (program tending,
  stewardship verdict posting, pool reviews) as service endpoints with
  the rotation, the retry-safety rule, and value-free errors inherited
  from the native design.
- The "small library surface" candidate of #9016 is answered for the
  hosted world by the service itself: the TS CLI + the service's route
  modules are the shared surface; no separate TS gh library is built
  unless a second TS consumer appears (recorded as a deferral, §8).

### 7.3 Ingestion interplay (#9076)

The hosted edition hosts the webhook half of dual ingestion naturally:
GitHub webhooks terminate at a public HTTPS endpoint, which a Worker
is and a laptop broker is not. Design: the **webhook listener is a
fleet-hosted route** (signature-verified; Metadata check → DB
insert/refresh → queue enqueue when the event warrants work, per §16),
and the **interval sweeper runs as a scheduled Worker** over the same
check code path. The native ingestion of #9076 remains a peer: both
run the same check, both write the same DB, and §16's self-healing
(backfill, retroactive completion for quickly-merged PRs, triage
forwarding on insufficient Metadata) is edition-blind. Whether the
native listener also runs is an operations choice, not a design fork —
the check is idempotent by construction (insert/refresh keyed on the
item).

## 8. Deferrals (named owner + trigger, per the acceptance)

1. **Broker-protocol transport framing** — REST vs WebSocket per queue
   verb, streaming shapes, reconnect behaviour. Owner: component G
   (#9085), transport inventory. Trigger: G's design merging. C fixes
   the requirement: every queue verb reachable over REST or WebSocket,
   claim/heartbeat latency-sensitive, no verb requiring a native-only
   transport.
2. **Stamp signature algorithm and key custody** — Ed25519 service key
   vs fleet-CA certificate; rotation and verification distribution.
   Owner: the fleet CA line
   (`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets`) with
   #9073's implementation. Trigger: #9073 build start. C fixes the
   field (`stamp_sig`), the canonical object's membership, and the
   edition-blind verification requirement.
3. **Metadata DB physical topology** — one Turso DB written by both
   editions from day one, vs hosted-first with native cutover. Owner:
   #9074 implementation. Trigger: #9074 build start. C fixes the
   invariant: one logical DB, one schema (#9074's), both editions
   writing it; physical sequencing is #9074's call.
4. **Per-surface token flows** — PKCE vs device flow per caller class,
   token lifetimes. Owner: component H (#9086 Q1). Trigger: H's design
   merging. C fixes its requirements in §4.1.
5. **A standalone TS gh library** — not built. Owner: #9016 triage.
   Trigger: a second TS consumer of gh-domain semantics beyond the
   fleet-hosted service and its CLI.

## 9. Statement coverage (component C slice of the inventory)

| Statement | Where answered |
| --- | --- |
| S4 (fleet tool's hosted capabilities exist as a TS service/CLI) | §§0, 2, 3 |
| S14 (direct Turso when prudent, rules defined) | §3.3 (C's rules; G owns the program statement) |
| S18 (dual tier-1; parity + deliberate divergence) | §§2, 4.3.1, 6 — every divergence named in place |
| S19 (peer principle; no special-casing) | §§5 (`host`), 6 (front doors as peers) |
| #9081 Q1 (split of duties) | §3 |
| #9081 Q2 (stamping parity) | §5 |
| #9081 Q3 (auth + non-forgeability) | §4 |
| #9081 Q4 (queue semantics) | §6 |
| #9078 program question 3 (the hosted queue question) | §6 |
| #9078 program question 5 (divergence discipline) | §6, final part |

## 10. What this record does not do

- No implementation. The granular implementation issues are filed under
  #9081 and listed in the progress note that accompanies this design's
  PR; each names its acceptance in the #9073/#9074/#9075 style.
- It does not amend #9073–#9076 or #9016: where the hosted shape needs
  a native change (the signing parity hook), it files a cross-linked
  issue under #9081 instead of editing another issue's scope.
- It does not answer A's service shape, B's gaps, E's schema, F's
  reference-vs-copy question, or G's program-wide transport/prudence
  statements — it consumes E as settled, defers to G and H where named,
  and stays inside component C.
