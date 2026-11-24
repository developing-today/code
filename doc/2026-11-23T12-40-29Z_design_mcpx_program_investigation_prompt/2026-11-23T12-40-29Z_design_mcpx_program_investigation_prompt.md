# MCPX Program — Full Investigation, Next-Steps Design & Build-Out Prompt

**You are the coordinator of a program-scale investigation and build-out of mcpx** — Drew's
MCP aggregation and proxy daemon in `developing-today/code` (`pkgs/mcpx/`), the single gateway
through which every agent on the fleet reaches every MCP server. Dig into the **full system**:
what it aggregates, how the daemon lives and dies, how its 241 settings behave, who consumes it
(harnesses, fleet, Atlas, ID), and what it should become. This prompt is a **starting point, not
a conclusive or final stopping point**. Work the design end to end, begin implementation
wherever you are unblocked, and when a pass finishes, review what remains, expand the plan, and
repeat — until nothing more can be done with the access and authority you have. Use subtasks and
orchestration heavily: launch many parallel agents (analyze, research, design, document,
implement) and keep them pointed at the frontier. A phase finishing is a signal to start the
next one, not to stop and ask.

---

## 1. What mcpx is (ground truth — verify, don't re-derive)

mcpx is a Go daemon + CLI (version 0.1.0) that owns every MCP server process on a machine and
exposes them to agents through one surface, so no harness ever loads every tool schema into
context. The agent loop is `mcpx ls` → `mcpx search` → `mcpx types <namespace>` →
`mcpx run` / `mcpx exec` (TypeScript scripts on deno/bun/node with namespaces preloaded).
One canonical source in `pkgs/mcpx/` (the dedup shipped; [#8822](https://github.com/developing-today/code/issues/8822)
is its tracking home). Programme homes: epic [#8993](https://github.com/developing-today/code/issues/8993)
(one canonical MCP gateway and server parity), maintenance epic
[#12459](https://github.com/developing-today/code/issues/12459), and the super tracker
[#1900](https://github.com/developing-today/code/issues/1900) (every theme, in one order, with
the disagreements between them).

- **Surface**: 75 CLI commands in six groups (running code, finding tools, the daemon,
  configuration, inspection, daemon operations) — including `exec`, `catalog` (token-budgeted tool listing),
  `search` (keyword or semantic), `types`, `prompt` (words in, script back), `proxy`
  (an OpenAI-compatible LLM reverse proxy), `serve` (MCP over stdio), `stdio-proxy` (daemon
  API over an ssh tunnel), `settings`, `servers`, `registry`, `dashboard`. Public Go API at
  `pkg/client`; ~60 internal packages (`daemon`, `settings`, `catalog`, `policy`, `elicit`,
  `mcpauth`, `mcpclient`, `mcpserver`, `model`, `tasks`, …).
- **Daemon lifecycle**: transports are local socket/systemd, ssh, and **tsnet** (the daemon
  as a tailnet node). Upgrades use a **takeover handoff** — the successor starts as the old
  daemon's child and receives stdio children's pipes over a unix socket; streamable HTTP
  upstream sessions survive, SSE sessions do not. Cold restarts resume streamable HTTP
  sessions from `upstream-sessions.json` (#1414 phases 1–2 landed; phase 3 open). The
  self-described stopgap replacement is a **supervisor that owns each stdio child's pipes
  for its whole life** — design [#1384](https://github.com/developing-today/code/issues/1384),
  Phase 3 [#9963](https://github.com/developing-today/code/issues/9963). When no daemon runs,
  the CLI starts one inline and polls readiness on the `daemon.inlineStartPoll` interval.
- **Config system**: a typed settings registry — **241 distinct setting paths** (193 public
  - 48 internal/plumbing — the registry `mcpx config --schema --plumbing` prints;
    an earlier census figure of 252 counted `Path:` literals across the whole settings
    tree, test fixtures included, and is superseded), each
    reachable via config file, environment, and `mcpx settings`, with resolved-value
    provenance. The registry carries an **effect-probe contract**: every setting has a named
    e2e probe proving it changes behaviour, or a written exemption; the exemption count is
    ratcheted (`EffectExemptionPin`) and can only shrink, only in the PR that adds the
    replacing probe. **The exemption tail is exactly 5 settings** (verified 2026-11-23;
    `EffectExemptionPin` = 5):
    `daemon.inlineStartPoll`, `daemon.tsnet.hostname`, `daemon.tsnetUpTimeout`,
    `http.streamBufferInit`, `embeddings.wasmPath` — they wait on harness shapes:
    a controllable tailnet, a timing-observable inline-start harness, an allocation oracle,
    an in-tree embed-exporting wasm fixture. Three further settings the first census
    listed in the tail — `plugin.instructions`, `plugin.disabledTools`,
    `protoTasks.cancelGrace` — gained their e2e probes on 2026-11-23 (PRs #13345,
    #13370, #13361; #8473 closed) and are no longer exempt. Coverage burn-downs:
    #8465, #8469 (blocked), #8471 (parked), #8472, #8473.
- **The book exists and is the Pages root**: `pkgs/mcpx/docs/` (mdBook, `.#mcpx-docs`,
  siteUrl argument) — 84 SUMMARY chapters, 120 markdown files, ~288k words including the
  package-local archives (`decisions/`, `changes/`, `compare/`, `spec/`). It deploys via
  `.github/workflows/mcpx-docs.yml` as the **whole repo Pages site** at
  https://developing-today.github.io/code/ — the fleet book already joins under `/fleet/`,
  atlas and id books join the same deploy. mcpx design records live in the book tree,
  **not** the repo-root `doc/` archive (0 mcpx entries there). No mechanical
  book-vs-binary coverage proof has ever been run — the audit is yours to build.
- **Deployment**: Nix modules in `pkgs/mcpx/modules/` are consumed **through the
  openchamber flake** (`inputs.openchamber.nixosModules.mcpx` etc.); `services.openchamber.enable`
  cascades to opencode + mcpx (2026-11-04 consolidation). The laptop runs
  `services.mcpx` with settings in `home/common/mcpx-settings.nix`; the live
  `~/.config/mcpx/config.json` (beside its `config.json.pre-cascade-20261103` fossil)
  aggregates many servers: chrome-devtools, cloudrun, codebase-memory, codedb,
  codex-connectors, deepwiki, desktop-commander, antigravity, and more.
- **Integrations, verified**: **Atlas is a live remote server in the laptop's mcpx** —
  server `"atlas"` → `https://atlas-search.developing-today.workers.dev/mcp` (#8625; the
  wiring landed under Drew's 2026-11-22 order; the remainder is cross-client acceptance).
  **ID's MCP bridge + `mcp_policy::PolicySession`** landed on the ID side (#7916, #7917
  closed) — MCP behind a capability layer. **fleet ↔ mcpx is design-stage**: design-mill
  passes #10389 / #10390 / #10393 / #10405 are open and unlanded. The opencode plugin is
  the deep harness integration (v2 rewrite #1858, SET-REMAINDER #2841 blocked; harness-free
  plugin core #1861). Server parity (GitHub/Atlassian/Datadog logins, #8809) is **Drew-gated**.
- **Census** (Search API, 2026-11-23): **21 open `app/mcpx`** (374 all-state), **45 open
  `epic/mcpx-platform`** (110 all-state), 92 open issues mentioning mcpx anywhere.
  18 open mcpx-mentioning issues carry `status/blocked`. Blocker heads: the exemption
  tail (§above), #1858 → #2841, #2686 + #10330 (test venues), #9900 → #1881 (release
  channels), #8017 (github failover), #3016 (nightly stress), #12738 (deploy wedge in
  the mcpx check phase), #13225 (Pages staging).
- **Embeddings inside mcpx**: the daemon has its own embeddings subsystem for semantic
  tool search (`internal/catalog/embeddings.go`) — a wazero Wasm embedder
  (`embeddings.wasmPath`) and remote OpenAI-compatible endpoints. This is tool-discovery
  search, a different index from Atlas's corpus search; the boundary is un-drawn.

## 2. Drew's directions (the next-steps agenda — design all of it, build what's unblocked)

### A. Server onboarding and lifecycle

Make adding a server a designed, boring path end to end: declaration shapes (stdio
command, remote URL, adapter-wrapped CLI #11147/`adapters.allowBare` + cwd, OpenAPI-derived
`api` servers, plugins, registry finds), credential resolution (in-memory `mcpx secrets`,
env, per-server env), health/probe semantics, restart and reload without disturbing
unchanged servers, and removal with session cleanup. Finish the supervisor design
(#1384 → #9963): one lifecycle design that unifies takeover, cold-restart resume
(#1414 phase 3), and supervised stdio — a daemon upgrade should become a plain restart
of a front end whose children never notice. Cover multi-daemon machines (`mcpx daemons`)
and the systemd/socket-activation posture.

### B. Per-client policy and capability ceilings

mcpx is where every client's reach is actually decided, but today policy is scattered
(rate limits and elicitation in `proto.*`, write policy, autonomy levels). Design **one
per-client policy model**: client identity (harness, seat, session), a capability
ceiling per client (which namespaces, which tools, read/write posture, rate and budget),
and enforcement inside the daemon so every transport inherits it. Tie the ladder to the
fleet's existing disciplines instead of inventing a third one: **Atlas's sensitivity
ceilings** (shareable < personal < local-only < secret-bearing, enforced server-side per
client identity) and **ID's capability discipline** (attenuable, revocable, audited —
the `PolicySession` shape). Open questions to design, not assume: how a ceiling is
declared (settings, config, per-server manifest), how elicitation binds user identity
(#7843, #1957), annotation trust, and what the audit record of a refused call looks like.
#12900 thread 2 (is Atlas headed for multi-client use?) is Drew's decision — design the
ceiling model so it survives either answer; don't pre-build multi-tenancy.

### C. Remote and hosted servers as versioned deployments

The Atlas server is the template: a hosted MCP server shipped as a versioned Cloudflare
deployment and wired into mcpx by URL. Generalize it: how a remote server is declared,
versioned, health-checked, and rolled back from the mcpx side; OAuth authorization for
remote servers **and for mcpx's own /mcp** (#1950, 61 conformance rows); session resume
across deploys; what `mcpx registry` should promise about a hosted server's provenance.
Close the release-channel gap on mcpx itself (#9900 → #1881): the install channels the
docs promise must be channels that exist.

### D. Config-system completion — burn the exemption tail to zero

The ratchet only moves one way; finish it. For each of the 5 exempt settings, design
the probe (or, where a setting is a pure allocation hint like `http.streamBufferInit`,
the contract decision that reclassifies it honestly — an exemption with a permanent
reason is a design smell, not a resting place): a harness-controlled tailnet for the
two `daemon.tsnet.*` settings and `daemon.tsnetUpTimeout`'s bring-up budget; a
timing-observable inline-start harness for `daemon.inlineStartPoll`; a small
embed-exporting **wasm fixture model in-tree** for `embeddings.wasmPath` (which also
unblocks real testing of the Wasm embedder). The three settings the first census
listed alongside these five (`plugin.instructions`, `plugin.disabledTools`,
`protoTasks.cancelGrace`) are already done: their e2e probes landed 2026-11-23
(PRs #13345, #13370, #13361; #8473 closed), so they are no longer exempt and need
nothing here. In parallel:
the remaining burn-downs (#8465 CLI running group, #8469, #8471) and the test-venue
blockers (#2686, #10330) that decide how much of this CI can see.

### E. Daemon lifecycle, transports, and reach

Beyond §A's supervisor: tsnet as a first-class reach (a daemon on the tailnet,
reachable from any seat — the `stdio-proxy`/ssh pattern generalized), inline-start
semantics, stream buffering as a designed contract (#4375: one bounded response-read
contract for all outbound calls — the `http.streamBufferInit/Max` pair is one
instance), dashboard and observability of the daemon itself, and the upgrade path's
failure shapes. Every bound in the daemon should be a setting with a probe (§D) —
no magic numbers with a comment.

### F. Observability and the event record

mcpx already keeps events, logs, stats, and feedback (`mcpx feedback`, the event
store, logstore). Design the coherent observability story: one event record for
every tool call (client, ceiling decision, latency, bytes, refusal reason), stats
that answer "which servers/tools does the fleet actually use", feedback wired to
something that consumes it, and the dashboard as the live view of the same record —
not three partial systems. This is also the audit substrate §B's policy model needs.

### G. Library and embedding use

mcpx's parts should be usable without the daemon: the public Go API (`pkg/client`
today), the C ABI / WASM / mcpx-as-its-own-upstream programme (#1877), the
harness-free plugin core with per-harness adapters (#1861), and embedding the
catalog/settings/policy packages as libraries in fleet and ID. Define the versioned
library surface (what is public, what is internal, what stability is promised),
with embedding examples taken from real call sites.

### H. The integration surface: mcpx ↔ fleet ↔ Atlas ↔ ID

- **fleet**: land the design-mill line (#10389 one fleet surface, #10390 pass-through
  subcommands + endpoint/credential resolution, #10393 MCP everywhere — namespaces,
  fan-out, remote graph, #10405 transitive capability purity). `fleet` verbs should
  reach MCP through mcpx, with mcpx's ceilings (§B) as the enforcement point.
- **Atlas**: close #8625's remainder (cross-client acceptance: OpenClaw / OpenCode /
  OpenChamber / Muse lanes, identity-stamped, ceiling-enforced, audit-logged) and
  draw the two-search boundary: mcpx's semantic tool discovery vs Atlas corpus
  search, one mental model for an agent choosing between them.
- **ID**: design the interop the landed ID bridge now makes possible — ID as an
  mcpx upstream (worlds/apps as namespaces), mcpx as the gateway through which ID
  clients reach external MCP servers under ID's capability discipline, and code-mode
  interop (#1940: nested code mode, `?codemode=false`, the `mcpx_exec` contract).
- **Harnesses**: the opencode v2 plugin remainder (#2841, behind #1858), and the
  per-harness skills (`mcpx-{agy,codex,claude,opencode2}`) kept in lockstep with
  the plugin core (#1861).

### I. The model layer and the AI surface

Settle where the model comes from (#1891, #1925 — blocked): the provider layer mcpx
owns, model profiles, the `prompt` command's contract, the OpenAI-compatible
`proxy`, and life after MCP Sampling's deprecation. Include administration by intent
(#1931: finish the operation surface, then route a sentence to a declared operation),
the request repair ladder (#1927), and lifecycle hooks on exec (#1929) — these are
one AI-surface design, currently four issues.

### J. Book extension, issue map, and the living record

1. **Extend the mcpx book** (it exists — do not rebuild it): first run the missing
   **mechanical coverage audit** — every CLI command and flag, every one of the 241
   settings (with default + provenance + probe/exemption status), every transport,
   every public library entry point, mapped to a chapter or a named gap; then write
   the missing chapters (the §A–§I designs join as book chapters in `pkgs/mcpx/docs/`,
   per mcpx's own convention), fix reverse gaps (chapters describing surfaces the
   code no longer has — the ASSESSMENT corrections-header pattern), and keep the
   zero-warning build + docs-twins gates green. The book's deployment chapter must
   absorb the new reality: its deploy now carries the fleet, atlas, and id books
   under subpaths (#13225's staging fix is part of this workstream's surface).
   Extend the docs-sync contract the way fleet's DOC-01 works: a surface change
   (command, flag, setting, transport) updates its chapter in the same change.
2. **An issue-by-issue map of ALL mcpx issues** — every open `app/mcpx` and
   `epic/mcpx-platform` issue (and the mcpx-adjacent #8625/#8993/#1900 families),
   prioritized in best working order, **with all blockers marked both on the issues
   themselves and in the priority list** (book chapter + tracking issue kept
   current). Blockers are worked, not just listed.
3. **Designs + PRs** — every workstream produces landed designs (mcpx product
   designs in the book tree; programme records in the repo `doc/` archive) and,
   wherever unblocked, merged implementation PRs.

## 3. How to work (fleet rules — binding)

- GitHub via the **gh CLI / REST only** (the GitHub connector is banned). File a GitHub
  issue in the owning repo the same pass for any problem found.
- Execution on the **laptop** (primaries stay on main + clean; worktrees on branches,
  landed by PR). Never `git stash` in shared clones. Never force-push main.
- Before implementing any issue: premise check + singular-ownership check (a claim
  comment from another agent is a hard stop). Re-read an issue's newest comments
  immediately before merging.
- Builds/tests dispatch-first (`fleet queue dispatch`), laptop fallback; a container
  environment failure is not a test result — record which route verified. Tests name
  their oracle. Note the standing trap: the fleet-build container cannot run some
  mcpx suites (the #13159 family) and `nix build .#deploy` wedges in the mcpx check
  phase (#12738) — route around them and record the route.
- Evidence standard: every factual claim carries its checked source or is labelled
  unverified; counts computed from a named source, never estimated in prose.
- **Respect standing holds**: flag-held items are not yours to implement without
  Drew's item-specific word — for mcpx that is #8472's `plugin.disabledTools` +
  `plugin.instructions` remainders and #8473's `protoTasks.cancelGrace`. Investigate
  and design around them; prepare their probes; never land them. Decisions that are
  Drew's (#8809 server logins, #8810's OS/call-method/auth answers, #1943's CNAME
  "apply it", #12900 thread 2) get surfaced with substance — options, ramifications,
  recommendation — never silently decided and never re-asked once recorded.
- Secrets by name only, never values. mcpx's own secrets handling is in-memory only;
  keep it that way in every design. Never touch the `vertexinc` org. External
  communications need per-item pre-approval. Quota doctrine: plan quotas only,
  never funds; a 402 is a wall.

## 4. The iteration rule

This prompt is the **starting point**. When every workstream has reported, do not
conclude: re-survey (issues, settings registry, book coverage, consumers, the live
daemon's server set), reconcile what the passes learned, expand the design set and
the issue map, and run the next pass. The program ends only when a full pass finds
nothing more that can be designed, documented, or implemented with the authority
available — and even then, the book, the issue map, the exemption ratchet at zero,
and the docs-sync contract remain as the living record the next pass starts from.
