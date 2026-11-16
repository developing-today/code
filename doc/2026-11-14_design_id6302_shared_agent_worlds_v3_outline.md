# Agents in worlds (MCP bridge) — v3 outline: shared agent worlds (#6302)

- **Issue:** #6302 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v3 refined outline, filed by queue task 16854 (lane any-b)
- **Predecessor:** v2 outline #6301, landed on main as
  `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
  (PR #8298, merge `ae7ace2ed8`), itself built on the v1 design #6300
  (PR #7911, merge `2e8ba8e031`). Entry condition verified: v2's
  design is written, its Drew questions are listed on #6301, and its
  follow-up steps are filed as native sub-issues #8299–#8303. This
  outline takes v1's mechanisms (agent key, UCAN-style delegation,
  capability ceiling, policy point, hash-chained audit) and v2's
  citizenship (roles as versioned world configuration, per-agent
  quota ledgers, owner review tooling) as given, and covers only
  what changes when agents **collaborate**: interaction through
  shared world state — task boards, shared documents, and worlds
  whose population is mostly agents with human visitors.
- **Siblings:** v1 #6300 (design landed; spike steps #7913–#7917) ·
  v2 #6301 (outline landed; steps #8299–#8303) · v4 #6303 (hosted
  agent services; gated on this outline).

## 1. Governing principles (#1240)

Unchanged from v1 §1 and v2 §1, with the v3 consequence spelled
out: collaboration is a property of **world state**, never of a
transport or a private channel — a task board behaves identically
over iroh P2P and through the hosted edge, a shared document's
provenance is the audit log either way, and the hosted instance
holds no privileged copy of any world's boards, documents or
population rules. Local-first always; hostable by anyone at any
scale; the hosted instance is the public backstop, never a
requirement.

## 2. Scope

### 2.1 Collaboration only through world state

V2 §2.4 drew the boundary v3 builds on: no inter-agent delegation,
no direct agent channel, mutual auditability. V3 keeps all three
and builds the collaboration semantics on top of them:

- **Every coordination act is a world action.** Claiming work,
  handing it off, commenting on it, editing a shared artifact —
  each is an ordinary world operation under the acting agent's
  role ceiling, charged to its quota ledger (v2 §2.3) and written
  to the audit log. There is no coordination surface a host cannot
  audit, replay and render for owners.
- **No new authority flows between agents.** An agent's post on a
  board cannot widen another agent's ceiling (v2 §2.4); a handoff
  transfers a _task reference_, never a delegation. The receiving
  agent acts under its own role or not at all.
- **World chat remains the only conversational surface**, under
  the same frames as every shell. Boards and documents below are
  how agents coordinate _work_; chat stays how anyone present
  talks, and neither becomes a side channel for the other.

### 2.2 Task boards as world objects

- A **board** is a world-owned object, configured like a role
  (v2 §2.1): its columns/states, which roles may create, claim,
  move and close items, and its visibility are world
  configuration, versioned with the world and editable by the
  owner through the same surface as every other world setting.
  Starter boards ship as ordinary definitions a world can copy
  and edit — defaults, not a privileged tier.
- An **item** carries: title/body, state, creator, current
  claim holder (agent + role), a claim epoch, and its full
  history as audit events. Item state transitions are world
  actions under role ceilings (a role that may claim may not
  necessarily close; closing is its own capability).
- **Claims are atomic and stale-safe.** Claiming an item is a
  checked world write in the spirit of v1 §4.3's stale-decision
  token: the claim names the item version the claimer saw; a
  claim against a moved item fails visibly rather than silently
  double-assigning. Claims expire on a board-configured lease;
  an expired claim returns the item to its prior state as an
  audited world event, so a parked or exhausted agent (v2 §2.3)
  cannot strand work forever.
- **Contention is data, not an error path.** A refused claim,
  like a refused join (v2 §2.2), is audited with its reason and
  visible to the refused agent, which may pick other work —
  boards never queue intents invisibly.

### 2.3 Shared documents

- A **shared document** is a world object co-edited through the
  bridge: every edit is a world action attributed to (agent,
  role) with the audit event carrying the edit's position and
  payload hash, so provenance — who wrote what, when, under
  which role — is reconstructible from the log alone.
- **Conflict model is per-document world configuration**, chosen
  from a small set the design must fix (§4): append-only log,
  sectioned ownership (a role owns sections; cross-section edits
  are proposals), or last-writer-wins with full history retained.
  No model permits an unrecorded overwrite: even LWW keeps the
  losing text in history.
- **Proposals are the cross-ceiling path.** An agent whose role
  cannot edit a document or section can attach a _proposed edit_;
  applying it is an action of an agent (or owner) whose role can,
  audited as that actor's edit with the proposal linked —
  authorship and authority stay distinct, mirroring v2 §2.5's
  one-shot approval shape without becoming a delegation.

### 2.4 Handoffs and cross-agent review

- **Handoff is a board operation, not a message.** The holder
  releases an item with a handoff note (a document or item
  comment in world state); another agent claims it under §2.2.
  What a handoff must carry — work state, artifacts produced,
  open questions — is a board-template convention (§4), enforced
  by the board's required-fields configuration rather than by
  asking agents to be tidy.
- **Cross-agent review uses roles, not rank.** A second agent
  whose role grants review on a board can comment, request
  changes (an item state), and flag — through world state, all
  audited. Review cannot raise the reviewed agent's ceiling,
  cannot spend its quota, and cannot execute owner approvals:
  v2 §2.5's approval queue remains owner-only. Whether a
  reviewer role may _accept_ work (move an item to done) or only
  recommend acceptance is a decision this iteration must settle
  (§4) — the outline's default is recommend-only until the spike
  in step 5 produces evidence.
- **Owners read digests, not streams.** V2's refusal digests
  (§2.5) extend to collaboration: per-board summaries of claims,
  stalls (items whose claim lease expired repeatedly), review
  backlogs and document-edit volume per agent — rendered from
  the audit log, which remains the record of truth.

### 2.5 Agent-majority worlds and human visitors

- A world may be configured so its **standing population is
  mostly agents** — running boards and documents continuously —
  with humans arriving as **visitors**. The visitor is a role
  like any other (v2 §2.1): typically read-mostly, with its
  capabilities set by world configuration, not a special client
  mode.
- **Legibility is a floor, not a feature.** In an agent-majority
  world a visitor must be able to answer, from world state and
  the audit log alone: who is here (agents, roles, owners), what
  changed since they last visited, what is blocked and on whom.
  Presence, board state and document history render for a
  visitor under exactly the rules §2.2–§2.4 record them — v3
  adds no separate reporting pipeline, and a world whose
  activity cannot be rendered legibly is a conformance failure
  (§6 step 5), not a documentation gap.
- **Attention protection.** Agent-population worlds can generate
  world events far faster than a human reads. World
  configuration therefore carries: per-role event-rate budgets
  on shared surfaces (charged like quota, v2 §2.3), board and
  document change digests in place of raw event streams for
  visitor roles, and population caps per role extending v2
  §2.2's join arbitration to standing populations. The caps are
  world configuration; the hosted backstop imposes none of its
  own (§1).

## 3. Non-goals for v3

- Direct agent-to-agent channels or inter-agent delegation —
  v2 §2.4's bans stand; v3 designs nothing that routes around
  them.
- Always-on hosted agents, agent registries, hiring/attaching
  agents to worlds: v4 #6303. V3 worlds are populated by agents
  whose owners bring them, under v2 grants.
- Agent-created agents and any form of self-provisioning
  (unchanged from v2).
- Cross-world collaboration: boards and documents live in one
  world; federation of world state is not designed here.
- Money movement: shared work charges quota ledgers in
  action/resource counts (v2 §5 question 2 remains Drew's to
  settle); no billing surface is designed here.
- Changes to v1's identity, delegation or transport rules, or
  to v2's role/quota/review model — v3 consumes both as-is.

## 4. Key decisions this iteration must settle

Design decisions for the v3 design step (not Drew rulings; each
step issue in §6 carries the ones in its scope):

1. **Board object model and claim semantics** — item fields and
   state machine, the claim write's exact staleness check, lease
   durations and expiry behaviour (§2.2).
2. **Shared-document conflict model set** — which of the §2.3
   models ship, how a document's model is recorded in world
   configuration, and the proposal/apply event shapes.
3. **Review acceptance authority** — whether a reviewer role may
   move an item to done or only recommend (§2.4); the default is
   recommend-only pending spike evidence.
4. **Handoff completeness rules** — what board templates may
   require of a handoff (artifacts, state note, open questions)
   and what an incomplete handoff does (refused vs. flagged).
5. **Population and rate configuration** — how standing-population
   caps and per-role event-rate budgets are expressed alongside
   v2's join caps and quota classes, and how exhaustion renders
   to visitors (§2.5).
6. **Visitor-role defaults** — the starter visitor role's exact
   read/write shape, and the minimum legibility rendering every
   conforming world must produce (§2.5).

## 5. Open questions needing Drew's ruling

Listed explicitly per this issue's acceptance criteria. None
blocks the step breakdown in §6; each step issue names the
question it depends on. Questions 1 and 2 sharpen two of v2's
still-open rulings (#6301 §5) where v3 makes them concrete; they
are carried here so the ruling lands once, on the record both
iterations share.

1. **Multi-owner shared worlds (sharpens #6301 §5 Q1):** v2
   asked whether an agent may hold a role in a world owned by
   someone else. V3's shared worlds make the general case
   pressing: may agents of _different_ owners collaborate in one
   world — and if so, whose board configuration, whose quota
   ledgers and whose review queue govern the shared surfaces?
   V4's hiring model inherits the answer, so a v3-only answer
   should say whether it expects to survive v4.
2. **Acceptance authority (new in v3):** may an agent in a
   reviewer role formally accept another agent's board work —
   moving it to done and releasing the claim — or is acceptance
   reserved to owners and the requesting role? §2.4 designs
   recommend-only as the default; a ruling either way is a
   product-trust call about how much of the loop agents may
   close without a human in it.
3. **Agent-majority worlds as a first-class deployment (new in
   v3):** is a world whose standing population is agents, with
   humans as occasional visitors, a deployment shape to design
   for directly — or should every shared world be required to
   have an actively reviewing owner (an attention floor) as a
   condition of running agent populations? §2.5's legibility
   floor is designed either way; the ruling decides whether it
   is a convenience or a safety requirement.
4. **Who pays for shared artifacts (sharpens #6301 §5 Q2):** a
   board or document is used by many agents; creation and upkeep
   actions charge someone's ledger. Is shared-surface upkeep
   charged to the acting agent (v2's per-action rule), to the
   board's owning role as a world budget, or split by a
   world-configured rule? The answer interacts with the
   quota-denomination ruling still open from v2.

## 6. Next-step breakdown (filed as native sub-issues of #6302)

1. **Board object model + claim semantics** — boards and items
   as versioned world configuration and world objects; atomic
   stale-safe claims with leases; contention as audited data
   (§2.2, decision 1). Blocked by v2 steps #8299 (role
   configuration) and #8300 (multi-agent sessions).
2. **Shared documents + provenance** — co-edited document
   objects, the conflict-model set, proposal/apply flow,
   provenance rendering from the audit log (§2.3, decision 2).
   Blocked by step 1 and v2 step #8299.
3. **Handoff + cross-agent review flows** — handoff as a board
   operation with template-required completeness; reviewer-role
   comment/request-changes/flag flows; collaboration digests for
   owners (§2.4, decisions 3–4). Blocked by steps 1–2 and v2
   step #8302 (owner review tooling).
4. **Agent-majority world profile + visitor role** — standing-
   population caps, per-role event-rate budgets, starter visitor
   role, legibility rendering floor (§2.5, decisions 5–6).
   Blocked by step 1 and v2 step #8300.
5. **Shared-world conformance + collaboration spike run** —
   conformance cases for claim staleness, no-side-channel and
   provenance invariants; scripted run: three agents of two
   roles collaborating on one board and one document with a
   human visitor, full audit (§2, §8 of v1). Blocked by steps
   2, 3 and 4.

Sequencing note: every step also depends on the v2 steps
(#8299–#8303) and the v1 spike (#7913–#7917) for the surfaces it
extends; the blocked-by relations name the direct edges and the
step issues name their remaining edges.

## 7. Dependencies

- v2 outline #6301 (landed) and its steps #8299 (role
  configuration), #8300 (multi-agent sessions), #8301 (per-agent
  quota ledger), #8302 (owner review tooling), #8303 (citizenship
  conformance) — v3's steps extend these surfaces.
- v1 design #6300 (landed) and its spike steps #7913 (shell
  profile), #7914 (MCP bridge), #7915 (identity + delegation),
  #7916 (policy point), #7917 (spike run).
- F12.10 quota design #1067 via plan of record #6312 — shared-
  surface charging (§5 question 4) must stay shape-compatible
  with the per-agent ledger events (v1 §9.3, v2 §2.3).
- The `id-shell` contract (`pkgs/id/contract/shell-contract.v1.json`)
  — boards and documents render through it; no contract change is
  assumed, and decision 6 of v2 (agent profile split) governs if
  that changes.

## 8. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Predecessors: #6300 +
  `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` · #6301 +
  `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
- Successor (entry condition met by this outline): v4 #6303
