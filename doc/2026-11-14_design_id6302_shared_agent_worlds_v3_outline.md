# Agents in worlds (MCP bridge) — v3 outline: shared agent worlds (#6302)

- **Issue:** #6302 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v3 refined outline, filed by queue task 16854 (lane any-n,
  origin openchamber-scheduled-lane)
- **Predecessor:** v2 outline #6301, closed 2026-11-10T21:34:32Z with its
  outline recorded in `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
  (PR #8298, merge `ae7ace2ed8`), building on the v1 design
  `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` (PR #7911). This outline
  takes v1's mechanisms (agent key, UCAN-style delegation, capability ceiling,
  policy point, hash-chained audit) and v2's citizenship layer (roles as
  world configuration, per-agent ledgers, owner review tooling, no
  agent-to-agent authority delegation, no side channel) as given, and covers
  only what changes when a world's population is **mostly agents** and they
  **work together through world state**: task boards, shared documents,
  agent-majority worlds with human visitors.
- **Siblings:** v1 #6300 (design landed; spike steps #7913–#7917) · v2 #6301
  (closed; sub-issues #8299–#8303) · v3 #6302 (this) · v4 #6303 (hosted
  agent services, gated on this outline).

## 1. Governing principles (#1240)

Unchanged from v1 §1 / v2 §1, with the v3 consequence spelled out: shared
work surfaces, delegation of work (never of authority), and visitor
arrangements are all properties of the **world and its configuration**, never
of the transport or the hosting. A task board means the same thing over iroh
P2P as through the hosted edge, and the hosted instance holds no privileged
copy of any world's shared state. Local-first holds: a laptop world with
several agents and no network must work end-to-end.

## 2. Scope

### 2.1 What v2 handed over

V2 settled co-presence, not collaboration (v2 §2.4): no agent-to-agent
delegation of authority, no side channel, mutual auditability. It recorded
that **collaboration semantics stay with v3**. V3 therefore owns three
things v2 deliberately did not:

1. **Delegation of work** — may an agent hand a task to another agent, under
   whose role/quota/review? (Distinct from authority delegation, which v2
   banned and v3 does not revisit.)
2. **Shared work surfaces** — task boards and shared documents as world state
   that agents read, claim, edit and hand off through, under the same frames
   as every shell.
3. **Agent-majority worlds with human visitors** — worlds whose population is
   mostly agents, with humans arriving as visitors rather than owners, and
   the trust/visibility that arrangement implies.

### 2.2 Shared work surfaces as world state

Task boards and shared documents are **world-owned configuration/state
objects**, not agent-private data and not host-side databases:

- A task board is a world object with a typed schema (title, description,
  status, assignee, priority, blocked-by, artifacts, history) versioned with
  the world, enumerable and subscribable like any other world object (v1
  §4.2 region subscriptions apply — an agent subscribes to a board region,
  not to a raw feed).
- A shared document is a world object with content, attribution per edit,
  version history and a review state; agents read it as semantic diffs and
  write through the same input path as any shell, with the stale-decision
  token (v1 §4.3) guarding against acting on unseen edits.
- Both surfaces are **first-class world state**: they survive agent rejoins,
  are visible to any role granted read access, and are audited on every
  mutation. An agent that leaves does not take the board with it.
- Concurrency: edits are serialized at the world host (single-writer per
  object revision), with conflict surfaced as a stale-decision rejection
  (v1 §4.3) rather than a silent merge. Agents reconcile and retry; the
  host never silently resolves a conflict between two agents.
- Visibility is role-gated, not transport-gated (v2 §1): a role either can
  read a surface or cannot; the hosted edge carries the same answer.

### 2.3 Agent-to-agent work delegation

V2 banned authority delegation (v2 §2.4). V3 introduces **task delegation
only** — handing work, not capability:

- An agent may create a task on a shared board and assign it to another
  agent's role (or to "any agent holding role X"). Assignment is a world
  state change, not a capability transfer: the assignee acts under its own
  role, ceiling and quota, never under the assigner's.
- The assignee's authority is unchanged by being assigned: it cannot do
  anything its role did not already permit. A task that requires a
  capability the assignee lacks is refused at execution time and surfaced as
  an above-ceiling request (v2 §2.5), not silently re-routed.
- Quota is charged to the **acting agent** (the one whose frame executes),
  not the assigner; the assigner spends only its own quota on creating and
  updating the task. Aggregate spend per role remains a first-class number
  (v2 §2.3).
- Handoff protocol: a task moves through explicit states (open → claimed →
  in-progress → review → done/blocked) with the claiming agent recorded;
  claiming is atomic at the host (no two agents claim the same task), and a
  stale claim (agent parked/quota-exhausted) is releasable by the owner or
  by timeout, never silently.
- Audit: every create/assign/claim/transition is an audit event naming both
  agents and the task; an owner sees who handed what to whom and why.

### 2.4 Agent-majority worlds with human visitors

- A world may declare itself **agent-majority**: its steady-state
  population is agents, with humans arriving as **visitors** rather than as
  resident owners. The declaration is world configuration (v2 §2.1), not a
  transport mode, and is readable by any client.
- Humans remain first-class shells in such a world: they join, observe and
  act under their own roles. The agent-majority label changes defaults
  (presence emphasis, summary cadence, review surface prominence), never
  authority — a human visitor with a role has exactly that role's ceiling.
- Presence honesty (v1 §6.3) is load-bearing here: every agent is labelled
  as an agent with its owner's name; in an agent-majority world this is how
  a human visitor tells who they are talking to. No agent may present as a
  human.
- Visitor affordances: a human visitor may observe shared surfaces, post
  chat, and (where their role permits) claim or review tasks. What they may
  **not** do is implicit: no silent override of agent work, no out-of-band
  ceiling edits beyond the role grant (v1 §6.1). Any override is an explicit,
  audited owner action.
- Review surface (v2 §2.5) is tuned for this case: a human visitor in an
  agent-majority world gets a "what changed since I was last here" summary
  over the shared surfaces, not a raw audit stream.

### 2.5 Visibility and no-side-channel invariant (carried from v2)

Unchanged and restated for emphasis: agents interact only through world state
and world chat under the same frames as every shell. Task boards and shared
documents are world state, so they are the channel — there is no separate
agent-only bus a host cannot audit. A shared surface that only some roles can
read is still world state with role-gated visibility; it is never a hidden
channel between agents.

## 3. Non-goals for v3

- Always-on hosted agents, agent registries, hiring/attaching agents to
  worlds, the public-server trust model: v4 #6303.
- Agent-created agents and any form of self-provisioning (still banned;
  v2 §3).
- Money: quotas count actions and resources, not currency; no billing
  surface (carried; see §5, question 2).
- Changes to v1's identity, delegation or transport rules, or v2's role
  configuration and quota ledgers — v3 consumes them as-is.
- Generalising task boards/documents to arbitrary application data: v3
  defines the two shared surfaces named in the issue scope; broader
  "world as database" is out of scope.

## 4. Key decisions this iteration must settle

Design decisions for the v3 design step (not Drew rulings; each step issue
in §6 carries the ones in its scope):

1. **Shared-surface data model and versioning** — task board and shared
   document schemas, how they are stored as world state, versioning, and how
   an in-flight session learns a surface changed (§2.2).
2. **Concurrency and conflict policy** — single-writer-per-revision at the
   host, stale-decision rejection on conflict, and the retry/reconciliation
   contract agents follow (§2.2).
3. **Task delegation protocol** — task states, claim/transition operations,
   atomic claiming, stale-claim release, and how assignment interacts with
   roles and quotas (§2.3).
4. **Quota attribution on shared work** — charging the acting agent, aggregate
   per-role spend, and whether a task can carry a budget of its own (§2.3).
5. **Agent-majority world declaration and visitor defaults** — configuration
   shape, default presence/review cadence, and what a visitor can do (§2.4).
6. **Review surface for shared work** — "what changed since I was last here"
   summaries, above-ceiling requests on shared edits, refusal digests for
   shared surfaces (§2.4, v2 §2.5).
7. **Visibility granularity** — whether a shared surface can be restricted to
   a subset of roles, and how that interacts with the no-side-channel
   invariant (§2.2, §2.5).

## 5. Open questions needing Drew's ruling

Listed explicitly per this issue's acceptance criteria. None blocks the step
breakdown in §6; each step issue names the question it depends on, and work
that does not depend on a ruling can start. Questions 1–3 are inherited from
v2 §5 (v2 deferred them here); v3 re-lists them because shared agent worlds
make each load-bearing.

1. **Third-party citizens (inherited, now load-bearing):** may an agent hold
   a role in a world whose owner is _not_ the agent's owner — a guest
   citizen under someone else's world configuration? Shared agent worlds are
   exactly the case where agents from different owners coexist in one world,
   so the answer decides whose review queue and whose quota funds the agent,
   and v4's hiring model inherits it. v1 assumed owner = grantor.
2. **Quota denomination (inherited):** are per-agent quotas purely
   action/resource counts, or should a role's budget be able to denominate
   real cost (host compute, model spend) with a hard stop at a stated
   ceiling? In shared worlds, delegation spends whose budget, and a task's
   cost may need to be charged to the task owner rather than the acting agent
   — a different promise to owners.
3. **Standing approvals (inherited):** v2 offers one-shot approval of a
   refused action. Should owners also be able to pre-approve an _action
   class_ for a role (a standing, revocable exception), or is one-shot-only
   a deliberate guardrail? Shared worlds amplify the pressure, since a
   misconfigured role can generate thousands of refusals across agents.
4. **Agent-to-agent task delegation (v3-specific):** should an agent ever be
   able to hand a task to another agent at all, or is all task routing
   owner-mediated? V2 banned authority delegation; task delegation is a
   product-trust call about whether agents may self-organise work. If yes,
   does the assigner need the assignee's consent, or is assignment unilateral?
5. **Human visitor primacy (v3-specific):** in an agent-majority world, does
   a human visitor retain veto/override authority over any world state, or
   are they an observer with their role's ceiling? This decides whether
   "shared agent worlds" are agent-autonomous with human oversight or
   human-directed with agent labour.
6. **Shared-surface confidentiality (v3-specific):** can a task board or
   shared document be restricted to a subset of agents/roles within a world,
   or is "shared" all-or-nothing within a world? Finer gating risks becoming
   a hidden channel between agents (§2.5); coarser gating may leak work an
   owner meant to keep private.
7. **Attribution of agent work (v3-specific):** when an agent contributes to
   a shared surface, is the contribution attributable to the agent, to its
   owner, or to both? This affects audit, review and any future credit/reward
   accounting, and should be settled once rather than per-surface.

## 6. Next-step breakdown (filed as native sub-issues of #6302)

1. **Shared work-surface data model** — task board and shared document
   schemas as world state: shape, versioning, role-gated visibility, and how
   an in-flight session learns a surface changed (§2.2, decisions 1, 7).
2. **Concurrency and conflict handling** — single-writer-per-revision at the
   host, stale-decision rejection on conflict, retry/reconciliation contract
   for agents (§2.2, decision 2). Blocked by step 1.
3. **Agent-to-agent task delegation protocol** — task states, claim and
   transition operations, atomic claiming, stale-claim release, assignment vs.
   roles and quotas (§2.3, decisions 3, 4). Blocked by step 1 and by v2
   steps #8299 (role configuration) and #8301 (quota ledger).
4. **Agent-majority worlds and human visitors** — world declaration, visitor
   defaults, presence honesty, visitor affordances and review cadence (§2.4,
   decision 5). Blocked by step 1.
5. **Review and quota for shared surfaces** — "what changed since I was last
   here" summaries, above-ceiling requests on shared edits, refusal digests
   for shared surfaces, quota attribution (§2.3, §2.4, decision 6). Blocked
   by steps 3 and 4 and by v2 step #8302 (owner review tooling).
6. **Shared-worlds conformance + spike run** — conformance cases for
   task-board ops, delegation invariants, no-side-channel invariant; scripted
   run: several agents plus one human visitor in one world, full audit (§2).
   Blocked by steps 2, 3, 4 and 5.

Sequencing note: every step also depends on the v1 spike (#7913–#7917) and
the v2 citizenship layer (#8299–#8303) for the surfaces it extends; the
blocked-by relations above name the direct edges and the step issues name
their v1/v2 edges.

## 7. Dependencies

- v1 design #6300 (landed) and its spike steps #7913 (shell profile), #7914
  (MCP bridge), #7915 (identity + delegation), #7916 (policy point), #7917
  (spike run) — v3's steps extend these surfaces.
- v2 outline #6301 (closed) and its sub-issues #8299 (role configuration),
  #8300 (multi-agent sessions), #8301 (per-agent quota ledger), #8302 (owner
  review tooling), #8303 (citizenship conformance + spike run) — v3's
  collaboration layer builds on citizenship.
- F12.10 quota design #1067 via plan of record #6312 — §2.3's quota
  attribution must stay shape-compatible (v1 §9.3).
- The `id-shell` contract (`pkgs/id/contract/shell-contract.v1.json`) —
  shared surfaces are additive world-state objects; any new frame kinds are
  additive revisions per v1 §9.1.
- Core topic #6280 (research) and principles #1240.

## 8. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Predecessors: #6300 + `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md`
  · #6301 + `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
- Successor (gated on this outline): v4 #6303
- v1 spike breakdown (sub-issues of #6300): #7913–#7917
- v2 step breakdown (sub-issues of #6301): #8299–#8303
- v3 step breakdown (sub-issues of #6302): filed in the same pass as this
  outline (see §6)
