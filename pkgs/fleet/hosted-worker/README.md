# fleet-hosted

The hosted fleet front door: one Cloudflare Worker whose route modules
are the component-C front doors — stamping, query, queue, ingestion,
sync — over one shared auth/tenancy plumbing layer.

Design: `doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition`
(component C of epic #9078, design issue #9081), §3.1. This package is
the service skeleton from #12583; each door is filled by its own issue.

## What is real in this slice

- **RP-token auth validation** (`src/jwt.ts`, `src/auth.ts`): callers
  present #9077 relying-party tokens as Bearer credentials. Validation
  is the standard relying-party check — JWS signature against the
  provider's JWKS (WebCrypto only, RS256/ES256), then issuer, audience
  and lifetime claims. The provider is configuration (`RP_ISSUER`,
  `RP_AUDIENCE`, `RP_JWKS_URI`), because #9077 itself is still a
  design-stage dependency; tests mint a local provider key.
- **Caller kinds** (design §4.1): people/agent runtimes via RP tokens;
  agent-role credentials (#1253), whose lane/session claims are
  required and become the caller's ceiling; per-caller **service
  tokens** recorded in the tenancy plane, looked up by SHA-256 and
  scoped to named doors.
- **Tenancy mapping** (`src/tenancy.ts`): a validated subject maps to
  a tenancy account keyed by its minted `binding_id` (32 lowercase
  hex, the component E / #2258 contract) — never an email or provider
  subject. Unmapped or inactive accounts fail closed, and a request
  asserting a different tenancy than its credential's
  (`x-binding-id` / `x-tenancy-binding` header) is refused as a
  cross-tenant attempt.
- **Data-plane refusal** (design §3.3 rule 3): a per-user Turso DB
  credential (libsql access-claim shape) presented as service auth is
  refused with `data_plane_credential_refused`. It is a read
  credential for the data plane, never an authentication here.
- **Session registry** (`src/sessions.ts`, `src/routes/sessions.ts`,
  #12584; design §4.2): the server-side replacement for #9075's env +
  worktree pwd-file in worlds with no worktree. The service mints the
  session id (`ses_` + 128 bits of crypto randomness as hex — the
  native #9075 id format, so ids mean the same thing in both
  editions; the client never chooses its id), and records binding_id
  (from the caller's credential, never a client assertion), lane,
  declared model, open/close times, and the lane-start issue set.
  Routes, all behind the shared plumbing: `POST /v1/sessions` (open),
  `POST /v1/sessions/resume` (CLI re-attach), `POST
/v1/sessions/resolve` (the stamp path's question — may this session
  be cited?), `POST /v1/sessions/close`. Unknown ids are refused
  `404 session_unknown` (a session owned by another tenancy answers
  identically — no cross-tenant existence oracle), closed ids `409
session_closed`, malformed ids `400 invalid_session_id`. An agent
  caller opens under its credential's lane; declaring a different one
  is refused (`lane_beyond_credential`).
- **Session resolution** (`src/session-resolution.ts`, #12584; design
  §3.2): the CLI-side carriage — explicit argument > env
  (`FLEET_SESSION_ID`, then `OPENCODE_SESSION_ID`) > per-workspace
  stored state, a step-for-step transcription of the native resolver
  (`pkgs/fleet/crates/fleet/src/session_state.rs`), including its
  failure modes: malformed nonempty values fail closed, a stored id
  that is ended or unverifiable is a named stale error, and a
  supplied identity is persisted for a later bare invocation.
  `test/session-parity.test.ts` runs the case table transcribed from
  the native resolver's own tests against it.
- **Stamping create path** (`src/routes/stamping.ts`,
  `src/metadata.ts`, `src/stamping.ts`, `src/github.ts`, #12585;
  design §3.1, §5): `POST /v1/stamps` is the hosted world's only
  stamping authority. It parses any pm-metadata block in the caller's
  body (#9060 §12 format; a malformed block refuses the create),
  stamps every fleet-owned provenance field from service sources
  only — service clock, queue record, session registry, validated
  token — overwriting caller-supplied provenance values wholesale,
  preserves caller-owned fields (`direct_parent`, `links`, `lineage`)
  exactly (injecting required-field defaults when absent), signs the
  provenance (#12586), creates the GitHub item through the hosted
  rotation point, and inserts the full record (full Metadata JSON +
  extracted keys) into the metadata store in the same request. A
  failed insert leaves the GitHub item intact and is reported in the
  response (`metadata_inserted: false`) for ingestion to heal.
  Unconfigured signing fails closed (503) before any write.
- **GitHub rotation point** (`src/github.ts`, #12585): the fleet
  App's installation-token rotation, held as Worker secrets
  (`GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY_PKCS8`,
  `GITHUB_APP_INSTALLATION_IDS` — configuration by name only).
  Installation tokens are minted from an App JWT (RS256, WebCrypto)
  and cached; rate-limit and permission refusals fail over to the
  next installation, the native #8887 rule. Callers never see a
  GitHub token, and there is no raw passthrough: every hosted GitHub
  write goes through a verb that can stamp.
- **Metadata query surface** (`src/routes/query.ts`, `src/query.ts`,
  `src/query-cli.ts`, #12587; design §7.1, #9060 §13 + §15):
  `GET|POST /v1/query` list/filter issues and PRs by Metadata
  fields — direct parent, session, lane, host, queue id, link
  relation/status pairs — served from the metadata store, scoped to
  the caller's tenancy. GitHub is the fallback for fields the DB
  has not yet ingested (§13): with a repo named, matching items
  that exist only on GitHub join the results (source `github`),
  attributed to a tenancy by their stamped `auth_subject`. Every
  DB result whose current body can be read is divergence-checked
  (§15): a body block that differs from the DB row is reported as a
  finding naming the differing fields, and the served Metadata is
  the body's — bodies win on content; each result also carries the
  #12586 stamp verification of the served provenance. The TS CLI
  query verbs (`src/query-cli.ts`) build the same filter from flags
  and render the same document, JSON first. **The collector's read
  path** is this surface: a service token scoped to the `query`
  door, one filtered read per genealogy slice; high-volume
  consumers use the direct-Turso read path (design §3.3 rule 1)
  instead.
- **Queue front door** (`src/routes/queue.ts`, `src/queue.ts`,
  `src/queue-validate.ts`, `src/queue-store.ts`,
  `src/queue-fallback.ts`, #12589; design §6): the broker protocol
  of the one fleet queue over hosted transports — one POST route
  per verb (`/v1/queue/<verb>`: validate, enqueue, claim, lane,
  bind, heartbeat, checkpoint, park, release, wake, block,
  unblock, dep-recheck, cancel, fail, complete, list, show,
  timeline, health, revive, and the events verbs), each answering
  the broker envelope at HTTP 200 exactly as the native broker
  does. The engine is a port of the native broker's verbs over the
  `QueueStore` interface: same repo-task/v1 packet validation
  (error strings included), same dedupe and work-identity rules,
  same lane ledger, same agent.events timeline, same completion
  gate. Claim atomicity is the store's conditional update — the
  guarantee the native backend gets from its conditional UPDATE
  under libSQL single-writer serialization; the doors are
  stateless peers, so work enqueued at one door is claimed at the
  other. Two deliberate divergences, both the design's: this door
  requires authentication and refuses anonymous claims (the native
  door keeps its local trust model), stamping the caller's
  credential-resolved `binding_id` into every event detail as
  claim provenance; and the fallback-claim protocol — an issue's
  GitHub comments carry the claim when the queue backend is down —
  is performed as a service under `/v1/queue/fallback/*` over the
  GitHub comment surface, with the native CLI's marker format and
  result shapes unchanged. Not ported: the claim-flow composite
  (a client-side composite of claim+lane+bind), the sweep /
  quarantine-recheck background verbs (dep-recheck is ported),
  and the merge-intent stack, creation-outbox, signal-count and
  coverage verbs of other stacks.
- **Ingestion door** (`src/routes/ingestion.ts`, `src/ingestion.ts`,
  #12590; design §7.3, #9060 §16): the hosted half of dual ingestion
  — the native ingestion of #9076 remains a peer, and both editions
  run the same check. `POST /v1/ingestion/github` is the GitHub
  webhook receiver: deliveries hold no fleet credential, so the
  route is public and the handler authenticates each delivery by
  its `X-Hub-Signature-256` HMAC (503 unconfigured, 401
  unverified, both before any parsing); events/actions outside
  the §16 set are acknowledged as ignored. `POST
/v1/ingestion/sweep` runs the interval sweeper over a named or
  configured repo scope (the Worker's `scheduled` handler runs
  the same sweep on the wrangler cron trigger). One check code
  path serves both: the item is read fresh through the GitHub
  item source (never the payload's copy), its pm-metadata block
  is parsed (malformed fails closed, 400), the DB record is
  inserted or refreshed keyed on the item under the block's own
  stamped tenancy, and a block-less item gets an ascertained
  block — parent resolved from its `primary-parent/<slug>` label,
  provenance from the item read, signed when the service holds
  stamp material — stored and written back where the fleet has
  write access. Metadata that stays insufficient (no resolvable
  parent, not queue-tracked) is forwarded for full triage as one
  deduped repo-task/v1 triage packet per item. The check is
  idempotent by construction: duplicate deliveries converge on
  one DB row and at most one triage task, and a create whose
  insert failed at the stamping door (#12585's
  `metadata_inserted: false`) is healed by the first check that
  sees the item.
- **Thin TS CLI** (`src/cli.ts`, package-root `cli.ts`, #12588;
  design §3.2): the hosted caller's command line — the entrypoint
  the verb modules above name. Verbs: `session open|resume|close`
  (carriage per §4.2: explicit > env > per-workspace stored state,
  the stored id's liveness checked against the registry before it
  is cited), `create issue|pr` (the caller's body text and
  caller-owned Metadata fields, assembled by the CLI; the request
  is a fixed allowlist of caller-owned fields and verifiable
  claims — no timestamps, no queue fields, no provenance),
  `query` (src/query-cli.ts's flags, rendered JSON first), `next`
  (work intake as the client-side claim + lane + bind composite
  the queue door leaves to the client) and `claim` (the single
  verb), and `sync project|flow` (src/sync-cli.ts's flags). The
  caller presents a #9077 relying-party token it already holds —
  resolved explicit > `FLEET_HOSTED_TOKEN` > the CLI's token file,
  storage local to the caller; acquisition flows (PKCE / device)
  are component H's surface (#9086, design §8 deferral 4).
- **Health + capabilities** (`src/health.ts`, `src/capabilities.ts`):
  public identity surface. `/capabilities` mirrors the native
  `fleet capabilities` JSON shape — same `build` / `capabilities` /
  `compiled_capabilities` / `domains` fields, native registry values —
  with hosted additions (`domains[].hosted` disposition from design
  §2, `filling_issue`, and a `doors` list). A domain reports
  available only when its filling issue landed: so far the
  hosted-transformed `gh` domain, via the stamping and query doors,
  and the `queue` domain, via the queue door.

## What is a stub

No door routes remain stubs. The last one was the sync door —
`POST /v1/sync/project` and `POST /v1/sync/queue-github`, filled
by #12591 — whose routes previously authenticated, then answered
`501 not_implemented`. They are implemented now; the smoke suite
exercises both end to end.

Stamp signing (#12586) is a separate slice, deliberately not
absorbed here. The session registry (#12584) is real, above — it is
a §4.2 service surface rather than one of the five §3.1 front doors,
so it does not appear in the capabilities `doors` list.

## Seams left for later slices

- **Tenancy directory**: component E's tenancy DB does not exist yet,
  so `TenancyDirectory` is an interface with a fixture implementation
  (`TENANCY_FIXTURE_JSON`, tests/local dev only). A DB-backed
  implementation replaces it behind the same interface.
- **Route paths** under `/v1/` are the skeleton's; each filling issue
  owns its door's final verb shapes (the queue door's broker-protocol
  transport is #12589's design surface).
- **Session store**: the registry's production home is the fleet
  metadata DB's session table (#9074's DB), so `SessionRegistry` is
  an interface with a fixture in-memory implementation
  (`FixtureSessionRegistry`; the worker entry holds one instance at
  module scope). A DB-backed implementation replaces it behind the
  same interface.
- **Metadata store**: the create path's insert target is #9074's
  metadata DB, which does not exist yet, so `MetadataStore`
  (`src/stamping.ts`) is an interface with a fixture in-memory
  implementation, wired like the session registry. A Turso-backed
  store replaces it behind the same interface.
- **Queue record source**: queue facts (queue id, scheduled start,
  pickup / work-start times) come from the queue record behind the
  `QueueRecordSource` seam (`src/stamping.ts`). Wired by #12589:
  `StoreQueueRecordSource` (`src/queue-records.ts`) reads the queue
  store's claim records — a session is queue-bound when a task's
  progress carries its session id — scoped to the caller's own
  tenancy through the session registry. A session with no queue
  binding still stamps its queue fields null (the design §5 rule
  for work that is not queue-originated).
- **Queue store**: the queue door's backend is the `QueueStore`
  interface (`src/queue-store.ts`) — task/ledger/lease/lane/event
  rows plus the one conditional-update primitive claim atomicity
  rests on. Its production home is the existing fleet-queue Turso
  DB (#6741), the same backend the native broker serves, behind a
  libSQL implementation of the same interface; until it lands the
  Worker wires the fixture in-memory store at module scope, like
  the other seams. The dep-recheck probe is the same posture: the
  native broker runs with no GitHub probe configured, and so does
  this door by default (task deps resolve from the store; GitHub
  deps stay blocked until their TTL).
- **GitHub writer**: `GitHubWriter` (`src/github.ts`) is the seam
  between the stamping route and the write itself. The Worker wires
  the App rotation writer when fleet App configuration is present
  and the fixture writer otherwise (local/dev semantics — this
  package is not deployed).
- **GitHub item source**: `GitHubItemSource` (`src/github.ts`) is
  the query door's read seam — current bodies for the §13 fallback
  scan and the §15 divergence check. Both writer classes implement
  it (the App rotation class over the same installation tokens; the
  fixture over its recorded items, where mutating a recorded body
  simulates a hand-edit), and the Worker wires the same object as
  writer and source. The query door's fallback scan is bounded
  (10 pages of 100 per repo); ingestion (#12590) is the
  completeness mechanism.

## Develop

```sh
npm test        # node --test over test/*.test.ts (no network, no wrangler)
npm run smoke   # in-process local run answering the acceptance checks
npm run typecheck
npm run dev     # wrangler dev (set RP_ISSUER / TENANCY_FIXTURE_JSON first)
```

Local-only: `wrangler.toml` authorizes no account deployment; a later
C issue owns production deployment.
