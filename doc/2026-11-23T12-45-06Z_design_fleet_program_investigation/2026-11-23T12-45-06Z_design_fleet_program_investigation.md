# Fleet Program — Full Investigation, Next-Steps Design & Build-Out Prompt

**You are the coordinator of a program-scale investigation and build-out of Fleet** — Drew's
agent-fleet operating system: the `fleet` multicall binary and its brokers in
`developing-today/code` (`pkgs/fleet/`), the queue/placement/dispatch machinery that moves
work between AI seats, the secrets and bridge services those seats share, the repo
constellation (`dezren39/fleet`, `developing-today/fleet`, 102 forks), and the quota plane
that keeps the model pools burning without ever spending funds. Dig into the **full system**:
what it routes, what it guarantees, where it lies to itself, and what it should become. This
prompt is a **starting point, not a conclusive or final stopping point**. Work the design end
to end, begin implementation wherever you are unblocked, and when a pass finishes, review
what remains, expand the plan, and repeat — until nothing more can be done with the access
and authority you have. Use subtasks and orchestration heavily: launch many parallel agents
(analyze, research, design, document, implement) and keep them pointed at the frontier. A
phase finishing is a signal to start the next one, not to stop and ask.

---

## 1. What Fleet is (ground truth — verify, don't re-derive)

Fleet is the substrate every other program here runs on: one binary, many seats, a shared
work queue, shared secrets, shared builds, shared messages. Seats are technically the same
software with different kinds and capabilities — a Muse seat, an OpenCode lane, a grok seat
differ in enrolment and platform, not in protocol. Governing rulings (Drew): **any agent can
claim any job** (lane names auto-register; there is no gatekeeping lane registry); **plan
quotas only, never funds**; **finished work is merged work** (not merged = not done).

- **Source home**: `pkgs/fleet/` in `developing-today/code` — **13 crates** (fleet,
  fleet-agent, fleet-bridge, fleet-config, fleet-dispatch, fleet-dispatch-test,
  fleet-laptop-agent, fleet-proxy, fleet-proxy-proto, fleet-queue, fleet-registry,
  fleet-search, fleet-secrets). The multicall-binary founding design is the dated record
  `doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary` (epic #5412). There is **no single
  fleet charter doc** — the README, the book's "Why fleet exists", and the `doc/` archive
  (~2,900 paths) hold the design in pieces. Producing the charter is part of your job (§2K).
- **The book exists and is the as-built record**: `pkgs/fleet/book/` — **55 chapters,
  98,894 words**, live at https://developing-today.github.io/code/fleet/, built by the
  `fleet-docs` flake package and deployed inside the mcpx-docs Pages job under `/code/fleet/`.
  The **DOC-01 contract** (`pkgs/fleet/README.md`, "Operator and contributor contract
  (DOC-01, #10728)", machine-checked) binds every change: _any change to a command, flag,
  default, capability, surface or procedure updates the matching book chapter in the same
  change._ The book is complete for the `b8bbcc0` surface and **already stale for main**:
  it has no `fleet queue repair` coverage (PR #13218 updated `docs/verbs.md` but not the
  book), and its dispatch chapter still calls placement sessions a local-JSON "STOPGAP"
  after the backend table landed. The installation chapter pins the `b8bbcc0` release.
- **Three binary revisions are in play** (verified 2026-11-23): the sandbox seat binary
  (source rev `044a6bda94b`), the deployed laptop builds (`b8bbcc0c4956` — queue-broker
  generation 5, laptop-agent generation 3, both activated 2026-11-23 ~09:08Z), and main,
  which has moved past both. The queue surface alone is ~51 leaf verbs (claim, park, block,
  unblock, dep-recheck, quarantine-recheck, revive, repair, dispatch, session, outbox,
  stack, …). Version skew between seats, broker, and book is a standing condition to design
  against, not an accident to lament.
- **Queue broker**: one crate (`fleet-queue`, broker = its `queue-broker` feature) over a
  Turso DB named `fleet-queue`; 19-statement schema (tasks, leases, lanes, workers,
  checkpoints, lane ledger, append-only `events` timeline, creation outbox + circuit,
  merge-intent, `placement_sessions`). Eight stored statuses — LIVE = queued, claimed,
  parked, handed_off, blocked; TERMINAL = done, cancelled, failed. **Quarantine is not a
  status**: it is `blocked` + `last_error`. A claim is one conditional UPDATE. Packet
  validation requires a dedupe attestation (`payload.dedupe` with `checked_at`/`basis`);
  the repair verb re-performs the dedupe check live and **refuses to stamp a false
  attestation**.
- **The two big queue defects are FIXED ON MAIN and DEPLOYED NOWHERE** (verified
  2026-11-23): the double-serve defect
  [#13091](https://github.com/developing-today/code/issues/13091) closed via
  [PR #13143](https://github.com/developing-today/code/pull/13143) (merged 10:04Z; park-term
  exclusion + completed-holder absorption), and the packet-repair verb
  [#13191](https://github.com/developing-today/code/issues/13191) closed via
  [PR #13218](https://github.com/developing-today/code/pull/13218) (merged 11:14Z). The
  running broker (gen 5, activated 09:08Z) predates both merges, and both deployed binaries
  answer `fleet queue repair` with "unknown command" (first-hand check). The pending step
  is the rollout: publish a fresh `fleet-static-*` release from main via
  `scripts/muse/publish-fleet.sh`, bump the bootstrap pin in `scripts/muse/bootstrap.sh`,
  flip the broker generation, and let the book's repair chapter ride the same change
  (DOC-01). Lane reports of double-serves after the fix must be triaged against this
  timeline before anyone re-opens the defect: code state, broker generation, and seat
  binary are three different facts.
- **Placement**: dispatch lands on the `fleet-command.yml` workflow; placement sessions
  were local JSON until
  [#13146](https://github.com/developing-today/code/issues/13146) slice 1
  ([PR #13169](https://github.com/developing-today/code/pull/13169), merged) mirrored them
  into the backend `placement_sessions` table (verbs /placement-put|get|list|sync) — local
  JSON remains the only decision input; status/result/followup take the newer of local vs
  backend. **#13146 stays open on criterion 1 only**: a cross-seat
  `fleet queue session status` read for another seat's session, which completes on the
  rollout above. Surfaces and caps (placement registry, verified): developing-today 20
  concurrent / macOS 5, developing-today-forks 20 / 5, dezren39 40 / 5. Blacksmith is a
  budgeted fallback only (3,000 free min/mo, `paid_cap_minutes: 0`, explicit
  `--allow-paid-blacksmith` for overage). The R2 Nix cache is the dispatch path's cache.
- **Packet-defect class**: malformed/mis-scoped queue packets are a recurring class, but
  **no single census of it exists** — the often-quoted "~30 issues" list was not found on
  any issue (five search phrasings, 2026-11-23). The recorded censuses are #9609's 279-row
  split (270 `max_attempts` + 9 schema) and #13191's dedupe-stamp class; live
  `fleet queue health` read `malformed_parked = 0` on all queues at research time.
  Producing the real census — and draining it with the repair verb once deployed — is
  workstream work (§2A), not a premise.
- **Agent lifecycle**: 16 kinds in `fleet-agent/src/kinds.rs` (muse, opencode,
  openchamber, openclaw, claude-code, codex, gemini, antigravity, grok, dot, duet, claude,
  chatgpt, laptop, http, generic). Enrolment = bridge `hello` registration + a 4-leg probe
  (bridge/queue/github/shell — **no secrets leg**) + a saved profile. `fleet agent next`
  rung order (in `next.rs`): **push → queue → shortlist (or a `refill` purpose-switch) →
  priority**; exit 5 = no work. The queue rung claims with a 4-hour lease (queue default
  is 45 min). The auto-register ruling (#12544) is implemented: `claim_preamble` inserts an
  unknown lane `INSERT OR IGNORE` on first claim; disabled/foreign-owned lanes still
  refuse, and `next` reports that as rung `rejected`. Task-level `requires.lane` /
  `requires.model` still filter claimers — "any agent any job" governs lane identity, not
  task requirements. The **shortlist** is a local per-host JSON file (target 8, low-water
  3, handed-out TTL 2 h, refill freshness 30 min); #12236 tracks the front door. The
  **16-lane floor is an Archer operating rule (Drew, 2026-11-22), not code** — nothing in
  fleet-agent/fleet-queue enforces it. Seats: `archer` (muse) verified in code and issues;
  the grok seat's onboarding is the token-distribution case study (§1, secrets). Other
  seats (felix, xaltha) are operationally real — they hold branches, PRs, and bridge
  traffic — but **no enrolment record for them was located in code** during research:
  where seat truth lives is itself an open question (§2C).
- **Secrets broker**: daemon in `fleet-queue/src/secrets_broker.rs` — cache-first, full
  refresh at startup and every `SECRETS_BROKER_TTL_S` (default 900 s), ~221 secrets via
  `bws secret list` + one `get` per secret (8-way parallel), read-through on miss, 503
  until the first successful refresh. Client chain: direct socket → session-broker framed
  → `bws` → optional `op`. **Open wound —
  [#13081](https://github.com/developing-today/code/issues/13081)**: the heal cron
  restarted the broker 4× while it reported `refreshing:true, secrets:0`, starving the
  very refresh it was waiting for (a cold refresh is ~120 s of bws fetches; the deploy
  health gate is shorter). Fix shape is recorded: never restart while `refreshing:true`.
  **Token distribution** (the grok onboarding, #12898 closed): the three shared
  service-token files are published in Bitwarden Secrets Manager as
  `fleet.secrets-broker-token`, `fleet.queue-broker-token`, `fleet.laptop-agent-token`;
  seats fetch them via `bws` into `~/tools/` at 0600; bootstrap auto-fetch landed via
  PR #12970. **Trap, documented in four places and still worth repeating: the BWS name
  `secrets-broker.token` is a DIFFERENT value (the broker daemon's own token) — never use
  it as a client token.** The laptop secrets-broker deliberately still runs generation 2
  (source_rev `working-tree`): its flip waits on a health window longer than a cold
  refresh.
- **Bridge**: inter-seat messaging via the message-bridge Worker
  (`pkgs/message-bridge/worker`, bridge.security.cab). Agent verbs use
  `message-bridge.fleet-token`, derived as base64url(HMAC-SHA256(owner token,
  "message-bridge/fleet-token/v1")) when the fleet secret is absent. Batch ids are
  **worker-side only** — one send = one batch, replies inherit it; the Rust CLI never
  sets one. **Laptop-agent** is its own package (`pkgs/laptop-agent`, port 4799), the
  single entry point for fleet laptop work, with a transport ladder (broker →
  Cloudflare → Tailscale → SSH fallback) and per-service Nix deploy profiles.
- **Remote build/test**: `fleet-command.yml` exists in all three fleet copies; canonical
  org surface is developing-today/fleet, plus fleet-personal (dezren39/fleet) and
  fleet-forks (public-only). The runbook is `pkgs/fleet/doc/agents/remote-build-and-test.md`
  (PR #13128): dispatch-first, session status/result, followup. **The fleet-build
  container gap family is all OPEN** — #13159 (flag-held: Drew's item-specific decision;
  scope prepared on the issue) plus evidence records #13168/#13193/#13198/#13189/#13170.
  The mechanism is now understood: three stacked nix defects (experimental features off;
  `nixbld` group missing; `/homeless-shelter` purity), cargo absent from PATH, no bun —
  and the Containerfile writes a correct `nix.conf` **only for user `vscode`, while
  fleet-command runs as root**. A container-environment failure is not a test result;
  lanes fall back to laptop runs and record the verifying route.
- **Repo constellation**: `dezren39/fleet` is upstream (HEAD `c8f4349d` at research time)
  with **22 submodules** in `.gitmodules` — the standing "12 submodules" figure is stale,
  correct it wherever it appears. `developing-today/fleet` is a fork **22 ahead / 0
  behind** upstream (fork-local deltas, missing nothing). **102 forks** sync on every
  push to upstream main via `sync-forks.yml` (merge-upstream per fork, attempts all,
  fails only at the end, files one todo issue in developing-today/code); auth is the
  classic PAT in secret `GHCR_ORG_PAT` (workflow scope required — the first run was
  refused by all 102 on exactly that gate). Rulesets 26550095/26550098 carry the admin +
  App bypass that makes the PAT route work. First fully clean run 45022131766
  (2026-11-22). Open in dezren39/fleet: **7 issues** — five fleet-build leg failures
  (#35 nix, #36 opencode, #37 code, #38 zettel, #39 openchamber), #88 (build leg), and
  **#87** (sync-upstream cannot run on new org forks — App-id mismatch; its remainder is
  the app-install decision in code#6645) — plus PR #1 (Renovate config).
  `developing-today/fleet` has 0 open issues.
- **Nix pin & releases**: the fleet flake.lock pin is **`f9887a8a434b…`** (nixpkgs-unstable
  tip) in both fleet repos — the standing `11dd50b` figure is stale. Consumer contract:
  `inputs.fleet.url = "github:dezren39/fleet"` + `nixpkgs.follows = "fleet/nixpkgs"`.
  Static releases: `scripts/muse/publish-fleet.sh` builds the musl binary, asserts
  static + bridge capability, publishes tag `fleet-static-<short-sha>` with asset
  `fleet-x86_64-linux-musl`. The bootstrap pin is **`fleet-static-b8bbcc0`**, which is
  also the latest published release — pin and latest agree, and **both are behind
  main**. Deployment truth: `pkgs/deploy/deploy.py` + `--registry` (validated registry,
  health check, auto-flip-back, append-only generation log) is the reliable route; the
  queue-broker's missing gen-4 directory (flipped via a PR mechanism outside deploy.py,
  log repaired by hand) is the standing counterexample.
- **Quota plane**: there is **no `fleet-quota` crate**. The live code is
  `pm-stack/aggregator/src/quota-*.ts` (15-min gatherer, `quota-current`); the doctrine
  lives in Drew's rulings and the goal-workspace registry, not in-repo — state that
  plainly wherever it matters. Admission semantics in code: walled = 100% or sustained
  ≥99.5%; stale/error/retained reads are `unknown`, and **unknown is never a wall** (a
  failed quota read never idles a pool). Drew's provider-disable wall is stricter and
  distinct: binding window <1% remaining **AND** plan-limit failures combined — one
  failed call alone never benches a provider. Section-risk notifications
  [#8882](https://github.com/developing-today/code/issues/8882) are **closed/shipped**
  (PR #8889; sections workhorse/speed/specialist/summit/small; walled ≥100%,
  pending-wall ≥85%). The stage-3 epic #12694 is open: S1/S2 closed, **S4 admission
  #12698 open**, **S7 forecast review #12701 open** (`status:in-review`; its
  implementation issue #13263 is closed and `quota-weekly-review.ts` is present).
- **Census** (Search API, 2026-11-23): the fleet label space in developing-today/code
  (`app/fleet` + eight `epic/fleet-*` / `design/*` labels) holds **185 open issues, 406
  all-state**. `app/fleet` alone: 113 open (63 `status/blocked`, 62 `queue/blocked`,
  8 `queue/queued`). **`kind/design` dominates: 98 of the 113** — and most of that is
  one design DAG, the pm-orchestration series (#9198–#9519, under
  `epic/fleet-orchestration`, 67 open). Fleet-labelled issues are ~46% of all open
  design work in the repo. The design backlog is the centre of gravity.
- **Neighbours**: `fleet search` is a flag-for-flag port of the Atlas client (byte-parity
  guard) — fleet's search quality is Atlas's freshness problem (Atlas program,
  [#13241](https://github.com/developing-today/code/issues/13241)). The ID program
  ([#13240](https://github.com/developing-today/code/issues/13240)) runs its lanes on
  fleet; fleet's guarantees are ID's floor. The pm-stack orchestration DAG is fleet's
  biggest design consumer and its own program — coordinate, don't absorb.

## 2. Drew's directions (the next-steps agenda — design all of it, build what's unblocked)

### A. Queue correctness at scale

The queue is the fleet's heart; make its guarantees explicit and then true. Deliverables:
an exactly-once claim semantics spec (what the conditional UPDATE does and does not
promise under lease expiry, heartbeat loss, broker restart, and Turso failover); a
post-fix verification programme for #13091 that separates code state, broker generation,
and seat binary before declaring the double-serve dead — then the rollout that makes it
dead in production; the packet-defect census (produce it, don't quote it) and a
repair-verb drain of every quarantined/malformed row the census finds, with the verb's
own discipline (deterministic repairs only, live re-check, false attestation refused)
extended to the other defect classes the census names; quarantine ageing and
self-triage (a quarantined row should surface its defect class and its repair path, not
sit silent); lease/heartbeat economics (the 4 h agent-next lease vs the 45 min default —
justify or unify); dep-recheck and wake semantics under load. Scale questions to answer
with measurements, not vibes: claim throughput at 16+ concurrent lanes, broker latency
over the tailnet, Turso group behaviour at the current task volume and at 10×.

### B. Placement intelligence & dispatch

Placement today mirrors sessions and follows static caps. Design the fully featured
version: close #13146 criterion 1 on the rollout and then go past it — cross-seat
session visibility as a designed surface (who may read whose sessions, with what
staleness bound); placement that learns from history (per-repo, per-command-class
success/latency records feeding surface choice instead of static rules); cap awareness
that degrades gracefully (queue-depth signals before a surface saturates, macOS's 5
treated as the scarce resource it is); Blacksmith budgeting as a first-class policy
(when is a paid minute worth it, decided in advance, not per-incident); dispatch
result caching and artifact reuse across lanes working the same repo; a placement
dry-run/explain surface ("why would this run go there?") that an operator can audit.

### C. Agent lifecycle & multi-seat coordination

Design the seat as a first-class citizen: a seat registry whose truth is queryable
(where does enrolment actually live — close the felix/xaltha gap the research found);
enrolment for new kinds (the 16-kind list will grow — what does a new kind have to
prove in the 4-leg probe, and should a secrets leg join it?); seat capability profiles
(what a seat may claim, dispatch, and read, stated once and enforced in the broker, not
in client goodwill); the lane model hardened (auto-register is a ruling — design its
abuse cases: lane squatting, stale lane ownership, worker-id collisions of the
`repo-worker-opencode` shape); shortlist science (is 8/3 the right shape? should the
shortlist be shared per seat instead of per host? what does refill quality mean —
measure how often a handed-out task is already dispositioned, the #13091-shaped tax on
the front door); bridge evolution (batch semantics are worker-side only — should the
CLI grow real threading, delivery receipts, and ordering guarantees?); cross-seat
handoffs as a designed protocol (the handoff tool exists — make its contract explicit:
single-use, 15-min, what state crosses).

### D. Secrets broker & credential distribution

Make the secrets plane boring: land the #13081 heal fix shape (never restart while
`refreshing:true`; health gates longer than a cold refresh) and design the general
rule — every fleet health gate must be sized from the slowest legitimate operation it
waits on; refresh economics (221 `bws get` calls per refresh — batching, delta
refresh, per-secret TTLs, and what the 900 s default should be at 10× secrets);
distribution beyond the three token files (the BWS `fleet.*` route worked for grok —
generalize it into the enrolment path for every shared file a seat needs, with the
`secrets-broker.token` trap designed out: names that differ by one dot should not be
able to hold different security principals); the laptop secrets-broker's deliberate
gen-2 non-flip, resolved as a designed cutover with a health window that fits; client
chain failure semantics (503-until-first-refresh vs serving stale — decide per secret
class); credential redundancy (Drew's rule: a Bitwarden-stored credential also gets a
private local copy) as an audited property, not a habit.

### E. Self-healing channel & broker stack

The channel stack (tailscaled → relay → session broker → secrets broker → gh) already
heals itself on a 5-min cron and cold-recovers from a script — design it as a system:
the five legs' failure taxonomy and which layer owns each recovery; heal-vs-restart
decisions that cannot starve the thing they heal (the #13081 lesson, generalized);
init-ensure's test-exec discipline (binaries resolve by test-exec, never `-x` — the
#12498 defect class) made a stated invariant with a test; cold-start proof as a
repeatable drill (kill all legs, rebuild, verify — the 2026-11-22 cold test,
automated); generation hygiene for every broker (the gen-4 directory anomaly must
become impossible: deploy.py + registry as the only flip path, with the log as an
append-only proof); observability of the stack itself (a leg's health readable
without reading its logs).

### F. Remote build/test platform

The dispatch path is how every lane verifies work; make it worthy. **#13159 and its
evidence family are flag-held — Drew's decisions: investigate and design around them,
never implement them.** Within that bound: design the container the family needs
(one image whose nix.conf, user model, and toolchain are correct for the user the
workflow actually runs as — the vscode/root split is the design's centre); the
fleet-run-is-not-a-sandbox lesson (the container sees the host's /etc — which bug
classes can it therefore never verify, and where do those verify instead?); parity
of the three `fleet-command.yml` copies as a checked invariant (the #12235 lesson:
byte-identical today, scheduled-sync mystery and all — design the drift alarm);
cache economics (R2 hit rates, what belongs in the cache, upload discipline);
org-cap scheduling across the three surfaces as one system (60+ concurrent slots
exist across orgs — placement should see them as one pool with per-org ceilings).

### G. Repo constellation, fork sync & releases

102 forks staying current is a platform feature, not a cron job. Deliverables: the
sync-forks failure taxonomy (the #12484/#12490 todo-issue trail — what actually
fails, how often, and what self-heals vs needs a human); #87's remainder (the
app-install decision in code#6645) prepared as a decision with substance for Drew,
not pre-decided; the five fleet-build leg failures in dezren39/fleet (#35–#39, #88)
triaged to root causes (they are the umbrella's own CI telling the truth — read
them); submodule discipline at 22 (what each submodule is for, when a member joins
or leaves, the gitlink-only rule for private members); the release train
(publish-fleet → bootstrap pin → broker generations → book) made one designed
pipeline with a single "fleet is current" check that reads all four; pin hygiene
(the `11dd50b` figure survived months past its truth — design how pins announce
their own staleness).

### H. Quota plane

Bring the quota plane in from the cold: the doctrine currently lives in rulings and
a goal-workspace registry while the code lives in pm-stack — design the in-repo
home (a fleet-quota crate? a pm-stack contract fleet consumes? decide with reasons);
close S4 (#12698 admission) and S7 (#12701 forecast review) or record exactly what
blocks them; the measurement gate as an implemented property (no dispatch decision
without a ≤60-min-old read, unknown never closes a pool, burn-by-expiry ordering
with OpenAI as final reserve — all stated as code-level invariants with tests that
name their oracle); wall semantics kept two-tier (admission wall vs Drew's stricter
disable wall) and never conflated; section-risk (#8882, shipped) extended —
per-section forecasts, and the quota records flowing to Atlas as a named source so
"what did we burn and why" is searchable; pool composition as a designed surface
(new provider onboarding without a doctrine rewrite).

### I. Observability & the operator surface

An operator should be able to answer "what is the fleet doing?" in one command.
Design: a fleet status surface that unifies queue depth by status, broker
generations vs main, seat enrolment and last-seen, shortlist health, dispatch
in-flight, quota walls, and channel legs — each with its source named; the
`events` timeline as a queryable audit log (who claimed, parked, blocked, and why,
across seats); anomaly surfacing shaped like the stewardship cycle's rules (name
items unmoved in 48 h as stalled, with their blocker — automate the sweep the
coordinator currently does by hand); cost accounting per lane/task (dispatch
minutes, pool burn attribution) feeding the quota plane; Fleet Reports as a
generated, Atlas-ingested artefact instead of hand-written notes.

### J. The fleet ↔ Atlas ↔ ID integration surface

Fleet is where the three programs touch. Design the seams as contracts: **Atlas** —
`fleet search` freshness is Atlas's ingest problem (#13241 §F), but the fleet side
owns the client contract (result caching keyed by index version, batch search for
lanes, offline fallback via libSQL embedded replicas, query telemetry from real
lane searches feeding Atlas's golden set); **ID** — ID's lanes run on fleet, and
ID's own roadmap (hosted backstop, federation) will want fleet-shaped primitives
(queue, secrets, dispatch) as libraries: audit which fleet crates are embeddable
today and what their stable APIs would be (the binary is one composition of them —
same rule as ID's); **mcpx** — the Atlas MCP server is wired through mcpx; fleet's
own MCP surface (should the queue/broker be MCP-addressable for non-fleet agents?)
is a design question to answer explicitly, with the tier/ceiling discipline Atlas
already proved. Every seam gets: the contract, the failure modes, and the test
that names its oracle.

### K. Charter, book, issue map, and the living record

1. **A fleet charter** — the document that does not exist: purpose, principles
   (any-agent-any-job, quotas-never-funds, merged-is-done, seats-are-one-software),
   component map, and the roadmap this program produces (short-term: the rollout
   and the census drain; long-term: §§A–J). Landed in the `doc/` archive and
   linked from `pkgs/fleet/README.md`.
2. **Extend the fleet book** — the book exists; the investigation's design series
   joins it under https://developing-today.github.io/code/fleet/ (new chapters in
   the existing book, built by `fleet-docs`, deployed by the same Pages job —
   never a second book, never a standalone deploy). First tranche is already
   owed: the `repair` chapter and the placement-sessions correction, riding the
   rollout per DOC-01. Every workstream above names the chapters it adds or
   rewrites; coverage is proved the bookwright way (mechanical diff of the
   binary's surface against the book, hand-checked edges, zero-warning build).
3. **An issue-by-issue map of ALL fleet issues** — every open issue in the
   185-issue label union in developing-today/code plus dezren39/fleet's 7,
   prioritized in best working order, **with all blockers marked both on the
   issues themselves and in the priority list** (a chapter in the book + the
   tracking issue kept current). The pm-orchestration DAG's place in the map is
   stated honestly (it is the bulk of the design backlog and has its own program
   — the map orders fleet's own work first and cross-references the DAG).
   Blockers are worked, not just listed: when a blocker is itself workable, it
   goes first.
4. **Designs + PRs** — every workstream produces landed designs (repo `doc/`
   archive) and, wherever unblocked, merged implementation PRs. CI runs on main
   only; merge when verified.

## 3. How to work (fleet rules — binding)

- GitHub via the **gh CLI / REST only** (the GitHub connector is banned). File a
  GitHub issue in the owning repo the same pass for any problem found; naming it
  in a doc is not a substitute.
- Execution on the **laptop** (primaries stay on main + clean; worktrees on
  branches, landed by PR). Never `git stash` in shared clones. Never force-push
  main.
- Before implementing any issue: premise check (is it still true on current
  main — and on the _deployed generation_, which is a different question) +
  singular-ownership check (fresh reads; a claim comment from another agent is a
  hard stop). Re-read an issue's newest comments immediately before merging.
- Builds/tests dispatch-first (`fleet queue dispatch`), laptop fallback; a
  container environment failure is not a test result — record which route
  verified. Tests name their oracle.
- Evidence standard: every factual claim carries its checked source or is
  labelled unverified; counts computed from a named source, never estimated in
  prose. Binary revision, broker generation, and book surface are three
  different facts — never let one stand in for another.
- **Standing holds are Drew's, item by item.** Flag-held items (#13159 and its
  evidence family, #13106, the PR shepherds, and the rest of the recorded hold
  set) are **not yours to implement** — investigate them, design around them,
  prepare their scopes; landing them needs Drew's item-specific go-ahead naming
  the issue. Decisions that are Drew's (code#6645's app-install call, #8479's
  Atlas-side purge question where it touches fleet data) get surfaced with
  substance — options, ramifications, recommendation — never silently decided
  and never re-asked once recorded.
- Secrets by name only, never values. The `secrets-broker.token` trap (§1) is
  the standing example of why. Never touch the `vertexinc` org. External
  communications need per-item pre-approval.
- Quota doctrine: plan quotas only, never funds; a 402 is a wall, one retry
  max; a failed or stale quota read never idles a pool; burn-by-expiry,
  OpenAI last.

## 4. The iteration rule

This prompt is the **starting point**. When every workstream has reported, do
not conclude: re-survey (issues, binaries, generations, designs, quota state),
reconcile what the passes learned, expand the design set and the issue map, and
run the next pass. The program ends only when a full pass finds nothing more
that can be designed, documented, or implemented with the authority available —
and even then, the charter, the book, the issue map, and the DOC-01 contract
remain as the living record the next pass starts from.
