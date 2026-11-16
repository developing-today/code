# Agents in worlds (MCP bridge) — v4 outline: hosted agent services (#6303)

- **Issue:** #6303 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v4 refined outline, filed by queue task 12953 (lane code-i)
- **Predecessor:** v3 outline #6302, landed on main as
  `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
  (PR #10097, merge `e2601e582365`), itself built on the v2 outline
  #6301 (PR #8298, merge `ae7ace2ed8`) and the v1 design #6300
  (PR #7911). Entry condition verified fresh 2026-11-16: v3's
  design is written, its Drew questions are listed explicitly
  (v3 §5, with Q1 written to be answered in a form v4's hiring
  model can inherit), and its follow-up steps are filed as native
  sub-issues #10049–#10054 with blocked-by relations. (#6302 is
  OPEN only because it was reopened 2026-11-14 after a wrongful
  bot close while those sub-issues are open — not because the
  design is unwritten.) This outline takes v1's mechanisms (agent
  key, UCAN-style delegation, capability ceiling, policy point,
  hash-chained audit), v2's citizenship (roles as versioned world
  configuration, per-agent quota ledgers, owner review tooling)
  and v3's collaboration (boards, shared documents, handoffs, all
  through world state) as given, and covers only what changes when
  agents are **hosted**: always-on agent services running on the
  T1 hosting substrate, listed in a registry, hired/attached to
  worlds under the public-server trust model.
- **Siblings:** v1 #6300 (design landed; spike steps #7913–#7917) ·
  v2 #6301 (outline landed; steps #8299–#8303) · v3 #6302 (outline
  landed; steps #10049–#10054) · epic #6280.

## 1. Governing principles (#1240)

Unchanged from v1 §1, v2 §1 and v3 §1, with the v4 consequence
spelled out: hosting is an **operational** fact, never an
authority fact. A hosted agent is someone's agent program running
on someone's host — every act it performs is an ordinary world
action under its role ceiling (v2 §2.1), charged to a quota
ledger (v2 §2.3) and written to the audit log (v1). Hosting adds
availability (always-on presence, wake-on-event, restart after
failure); it adds no capability, no private channel and no
privileged copy. The p2p and hosted behaviours are identical: a
board claim by a hosted agent is the same checked world write as
a claim by an owner-run agent (v3 §2.2), a hosted edit carries
the same provenance (v3 §2.3), and the hosted instance holds no
copy of any world's boards, documents, registry listings or
population rules beyond what operating that world's hired agents
requires. Local-first always; hostable by anyone at any scale;
the hosted instance is the public backstop, never a requirement.

## 2. Scope

### 2.1 Registry of agent programs

- A **listing** names an agent program: a pinned build (content
  hash, not a mutable tag), its owner/publisher, the role-shape
  it requires (capabilities it will exercise, quota classes it
  will spend, world surfaces it touches), and its version
  history. Listing is a claim by the publisher, auditable like
  any world-adjacent record — it is not an endorsement, and
  hiring a listing never inherits the publisher's authority.
- **Listing is distinct from permission.** What may be listed
  (open self-serve vs. curated; §5 question 4) and what any
  given owner may hire are separate decisions; the registry
  answers "what is this program", the world's owner answers
  "what runs here".
- Registry records render through the same legibility surfaces
  as the rest of the system: an owner considering a hire can
  answer, from world state and the log alone, which program,
  which version, published by whom, with what declared
  role-shape.

### 2.2 Hiring and attaching agents to worlds

- A **hire** is a v2 grant with a program attached: the world's
  owner grants a listed program a role in their world. The hired
  agent acts under that role's ceiling and nothing else — v2
  §2.4's bans (no inter-agent delegation, no direct agent
  channel) and v3's world-state-only collaboration apply to
  hired agents unchanged.
- **Lifecycle is world state.** Attach, version change (hire
  v1.2 → v1.3), suspension and detach are audited world events
  under the owner's authority, visible to visitors under v3
  §2.5's legibility floor: who hired whom, at which version,
  under which role, and when. A hire names the program version
  it approved; the host never silently upgrades a hired agent.
- A hired agent's quota ledger is explicit at hire time: which
  ledger funds its actions (the hiring world's budget, the
  agent owner's ledger, or a world-configured split) — the
  default follows the ruling on v3 §5 question 4, and the hire
  record states which rule applied so the charge is never
  ambiguous.

### 2.3 Always-on hosting on the T1 substrate

- **Liveness is the host's promise, behaviour is the world's.**
  The host keeps the hired program running, wakes it on the
  world events its role subscribes to, and restarts it after
  failure; every wakeup and restart is reconcilable against the
  audit log (a restarted agent replays from world state, never
  from a private checkpoint the world cannot see).
- Restart and failure semantics must compose with the outcome
  of issue #839: the outline assumes at-least-once wakeups with
  idempotent, version-checked world writes (v3 §2.2's
  stale-safe-claim spirit), and the design step confirms or
  revises that against #839.
- Standing populations (v3 §2.5) and hired agents compose: a
  hired agent counts against the world's per-role population
  caps and event-rate budgets like any agent. The host imposes
  no caps of its own (§1).

### 2.4 Public-server trust model

- The host operates the agent **process** but cannot widen its
  world authority: it cannot raise the role ceiling, approve
  owner-only actions (v2 §2.5's queue stays owner-only), accept
  board work the role may not accept (v3 §2.4 default stands),
  or spend beyond the ledgers named at hire.
- **Host-operator visibility is bounded and stated.** World
  state and the audit log are visible to anyone the world's own
  rules admit (v3 §2.5); process internals beyond that (model
  traffic, host-side logs) are governed by an explicit,
  published retention and access rule per host — §5 question 5 —
  never by silent default.
- Self-hosting is first-class: an owner running their own hired
  programs on their own substrate gets exactly the same
  semantics, and a world that moves a hire from the public
  backstop to self-hosting (or the reverse) changes only the
  hire record's host field, never the agent's authority.

### 2.5 Metering and ledger interplay

- Hosted compute is a cost with a payer. The outline carries
  v2 §5 question 2 (quota denomination: counts vs. real cost)
  and v3 §5 question 4 (who pays for shared upkeep) into their
  hosted form: the design charges host compute to the ledger
  named at hire, in the denomination the quota ruling settles,
  with exhaustion rendering to owners and visitors per v3
  §2.5 rather than failing silently.
- No billing surface is designed here (§3): ledgers record
  obligation in action/resource counts (and cost denomination
  if so ruled); money movement is out of scope.

## 3. Non-goals for v4

- Agent-created agents and any form of self-provisioning
  (unchanged from v2/v3) — hired agents are hired by owners,
  never by other agents.
- Cross-world hiring or federation of world state: a hire is
  per-world; v3's single-world rule for boards and documents
  stands.
- A billing or payments surface: ledgers record, money moves
  elsewhere.
- Private channels between hosted agents, host-mediated or
  otherwise — v2 §2.4's bans stand on every substrate (§1);
  hosted agents coordinate through world state like any agents.
- Changes to v1's identity, delegation or transport rules, to
  v2's role/quota/review model, or to v3's board/document/
  handoff semantics — v4 consumes all three as-is.

## 4. Key decisions this iteration must settle

Design decisions for the v4 design step (not Drew rulings; each
step issue in §6 carries the ones in its scope):

1. **Registry record schema** — listing fields, build-pin
   format, role-shape declaration language, version history
   and publisher identity, and where listings live (world
   object, substrate record, or both).
2. **Hire lifecycle events** — attach / version-change /
   suspend / detach event shapes, the audit trail each leaves,
   and what a hire promises about upgrade behaviour (pinned by
   default; what an owner opt-in to tracking a release line
   means).
3. **Liveness and restart contract** — wakeup triggers, restart
   guarantees, replay-from-world-state rules, and the
   composition with issue #839 (§2.3).
4. **Host process ceiling** — exactly what the runtime may do
   on a hired agent's behalf (wake, restart, deliver
   subscribed events) and what it structurally cannot (act,
   approve, spend, upgrade).
5. **Hired-agent legibility** — how hire records, program
   versions and ledger attribution render to owners and
   visitors under v3 §2.5's floor, including digest shapes for
   always-on activity.
6. **Conformance cases** — no-silent-upgrade, no-host-widening,
   ledger-attribution and replay-from-state invariants, plus a
   scripted run: a hired listed program collaborating on a v3
   board and document alongside an owner-run agent, then a host
   move (backstop → self-hosted) with authority unchanged.

## 5. Open questions needing Drew's ruling

Listed explicitly per this issue's acceptance criteria. None
blocks the step breakdown in §6; each step issue names the
question it depends on.

1. **Guest citizenship, decided at last (inherits v2 §5 Q1 and
   v3 §5 Q1):** hiring _is_ the guest-citizen case — an agent
   whose owner is not the world's owner, holding a role under
   someone else's world configuration. May agents of different
   owners collaborate in one world, and if so, whose board
   configuration, whose quota ledgers and whose review queue
   govern the shared surfaces? V3 asked that any v3-only ruling
   say whether it expects to survive v4; v4 needs the ruling in
   its final form, because every hire record inherits it.
2. **Attention floor for hired agents (sharpens v3 §5 Q3):**
   v3 asked whether an agent-majority world needs an actively
   reviewing owner. Hired always-on agents sharpen it: may a
   hired agent keep running in a world with no actively
   reviewing owner — and if an owner's attention lapses, does
   the hire suspend, continue read-only, or continue under
   tighter standing caps?
3. **Who funds hosted compute (inherits v2 §5 Q2 and v3 §5
   Q4):** a hired agent burns host compute continuously. Is it
   charged to the acting agent's ledger, the hiring world's
   budget, or a world-configured split — and in counts or in
   cost denomination? The answer sets the hire record's
   required fields (§2.2).
4. **Registry posture:** is listing open and self-serve (the
   world owner as the only gatekeeper at hire time) or curated
   (someone approves what may be listed)? Either way the design
   holds §1; the ruling decides where abuse prevention lives.
5. **Host-operator visibility (new in v4):** beyond world state
   and the audit log, what may a host operator observe and
   retain about hosted agent activity, for how long, and
   disclosed where? The outline's default is: nothing beyond
   operating the process, stated publicly, with self-hosting
   always available as the exit.

## 6. Next-step breakdown (to file as native sub-issues of #6303 as v3's steps land)

Sequencing note: v4's steps extend v3's surfaces, so each names
its v3 step dependencies (#10049–#10054, all OPEN as of
2026-11-14) alongside the v2/v1 edges it inherits; filing
follows the packet's "when the step's turn approaches" rule —
the breakdown below is the filing plan, and the sub-issues are
filed with blocked-by relations as the v3 steps they extend
land.

1. **Registry record + listing flow** — listing schema,
   build pins, role-shape declarations, publisher identity
   (decision 1). Blocked by v3 steps #10049 (board object
   model, for role-shape language reuse) and v2 #8299 (role
   configuration).
2. **Hire lifecycle + audit shapes** — attach/version-change/
   suspend/detach events, hire-record fields including ledger
   attribution, no-silent-upgrade invariant (decisions 2, 6 in
   part). Blocked by step 1 and v2 #8302 (owner review
   tooling — hires surface there).
3. **Liveness + restart contract on T1** — wakeup triggers,
   restart guarantees, replay-from-world-state, composition
   with #839 (decision 3). Blocked by v3 step #10053
   (agent-majority profile: caps and budgets the hosted agent
   counts against) and v2 #8300 (multi-agent sessions).
4. **Host process ceiling + operator visibility** — runtime
   allow-list, structural cannots, published retention/access
   rule (decisions 4–5 in part; Drew questions 4–5). Blocked
   by step 2 and v1 #7916 (policy point).
5. **Metering + conformance + hosted spike run** — ledger
   charging for host compute, conformance cases, scripted run
   incl. host move with authority unchanged (decisions 5–6;
   Drew questions 2–3 feed in). Blocked by steps 1–4 and v3
   step #10054 (shared-world conformance + spike).

Every step also depends on the v2 steps (#8299–#8303) and the
v1 spike (#7913–#7917) for the surfaces it extends; the
blocked-by relations name the direct edges and the step issues
name their remaining edges.

## 7. Dependencies

- v3 outline #6302 (landed) and its steps #10049 (board object
  model), #10050 (shared documents), #10051 (handoff + review),
  #10052 (agent-majority profile), #10053/#10054
  (conformance + spike) — v4's steps extend these surfaces.
- v2 outline #6301 (landed) and its steps #8299 (role
  configuration), #8300 (multi-agent sessions), #8301 (per-agent
  quota ledger), #8302 (owner review tooling), #8303 (citizenship
  conformance).
- v1 design #6300 (landed) and its spike steps #7913 (shell
  profile), #7914 (MCP bridge), #7915 (identity + delegation),
  #7916 (policy point), #7917 (spike run).
- F12.10 quota design #1067 via plan of record #6312 — hosted-
  compute charging (§2.5) must stay shape-compatible with the
  per-agent ledger events (v1 §9.3, v2 §2.3).
- Issue #839 (restart semantics) — the liveness contract (§2.3,
  decision 3) must compose with its outcome.
- The `id-shell` contract (`pkgs/id/contract/shell-contract.v1.json`)
  — registry and hire records render through it; no contract
  change is assumed.

## 8. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Predecessors: #6300 +
  `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` · #6301 +
  `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md` ·
  #6302 +
  `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
- This issue is the v4 step; its sub-issues file per §6 as the
  v3 steps land.
