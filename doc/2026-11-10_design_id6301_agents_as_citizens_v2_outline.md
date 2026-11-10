# Agents in worlds (MCP bridge) — v2 outline: agents as citizens (#6301)

- **Issue:** #6301 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v2 refined outline, filed by queue task 13955 (lane code-y)
- **Predecessor:** v1 design #6300, landed on main as
  `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` (PR #7911,
  merge `2e8ba8e031`). This outline takes v1's mechanisms — agent key,
  UCAN-style delegation, capability ceiling, policy point, hash-chained
  audit — as given, and covers only what changes when a world has
  *citizens*: configured roles, several agents at once, quotas that
  outlive a session, and an owner who reviews what their agents did.
- **Siblings:** v1 #6300 (design landed; spike steps #7913–#7917) ·
  v3 #6302 (shared agent worlds) · v4 #6303 (hosted agent services).

## 1. Governing principles (#1240)

Unchanged from v1 §1, with the v2 consequence spelled out: roles,
quotas and review are properties of the **world and the delegation**,
never of the transport or the hosting — a role definition means the
same thing over iroh P2P as through the hosted edge, and the hosted
instance holds no privileged copy of any world's role configuration.

## 2. Scope

### 2.1 Roles as first-class world configuration

In v1 a role is a binding the host checks at join (owner key → role →
ceiling). In v2 the role becomes a **world-owned configuration
object**:

- A role definition — capability ceiling (§6.1 of v1), quota class
  (§3 below), delegation-lifetime bounds (v1 §9.2 range), review
  policy (§4) — lives in the world's configuration, versioned with
  it, editable by the world owner through the same surface that edits
  every other world setting. No role exists only as a string in a
  token scope.
- Grants are explicit world state: *this agent key holds this role in
  this world, granted by this owner, at this time* — enumerable by
  the owner, revocable independently of delegation expiry (revocation
  ends the grant; expiry only ends a session's proof of it).
- Role changes are themselves audited world events, and a changed
  ceiling applies at the next frame check, not the next join — a
  citizen's rights track the current configuration, not the
  configuration at join time.
- Built-in starter roles (observer, participant, archivist-class
  editor) ship as ordinary definitions a world can copy and edit —
  defaults, not a privileged tier.

### 2.2 Multiple agents, distinct roles, one world

- Several agents hold sessions in one world concurrently, each under
  its own role, ceiling and quota. The policy point checks per agent,
  per frame — v1's check sequence (ceiling → quota → audit) is
  unchanged, now keyed by (agent, role) instead of session alone.
- Agents are visible to each other exactly as their roles allow: an
  agent's presence, chat and world edits are world events like any
  shell's. Co-presence needs no new contract surface in v2 — what an
  agent may *do about* another agent is §2.4's narrower question.
- Join arbitration: a world may cap concurrent agents per role; a
  refused join is audited with the reason, as in v1.

### 2.3 Per-agent quotas that outlive the session

V1's quota is a per-session budget inside the policy point (§9.3 of
v1, deliberately replaceable by F12.10 #1067). V2 makes quota a
**per-(agent, world) ledger**:

- Budgets are granted with the role, replenish on the role's stated
  cadence, and persist across sessions — a citizen cannot reset its
  budget by rejoining.
- Charging stays per action (v1 Q3b); exhaustion still parks the
  agent *visibly* (world event), and the park now names the role's
  replenishment time.
- The ledger's event shape stays the one F12.10 consumes (v1 §9.3):
  when the world's quota system lands, the per-agent ledger becomes
  an account in it rather than a parallel system.
- Owners see spend per agent and per role; a role's aggregate spend
  is a first-class number, not a sum owners compute by hand.

### 2.4 Agent-to-agent boundaries (co-presence, not collaboration)

V2 settles only what citizenship requires:

- **No agent-to-agent delegation.** An agent cannot widen, mint or
  pass on authority (v1 §4.5 already bans this for tools; v2 states
  it between agents): a citizen's ceiling is set by its role grant
  and nothing another agent does can change it.
- **No direct agent channel.** Agents interact only through world
  state and world chat, under the same frames as every shell — there
  is no side channel a host cannot audit. (Rich collaboration
  semantics on top of world state are v3 #6302's subject.)
- **Mutual auditability.** An action by agent A that affects an
  object agent B can see is visible to B's owner summaries under the
  same rules as any other world event — citizenship does not create
  private effects.

### 2.5 Owner review tooling

V1 gives owners a signed audit log and periodic summaries (v1 §7).
V2 makes review a working surface, not a report:

- **Per-agent review view:** one place per world listing each agent,
  its role, current quota state, recent actions and refusals —
  rendered from the audit log, which remains the record of truth.
- **Above-ceiling requests queue for review:** when an agent's action
  is refused by the ceiling, the owner may be offered the *request*
  (not a standing widening) to approve or deny once. Approval executes
  that one action under the owner's authority and is audited as such;
  it never edits the role. (Whether *standing* approvals for an
  action class should exist is an open question for Drew, §5.)
- **Refusal digests:** repeated refusals of the same shape surface as
  a pattern ("archivist was refused object.export 12 times") so a
  misconfigured role is fixable in one edit — the fix is a role edit
  (§2.1), never an in-band exception.

## 3. Non-goals for v2

- Agent-to-agent collaboration semantics — task boards, shared
  documents, agent-majority worlds: v3 #6302.
- Always-on hosted agents, agent registries, hiring/attaching agents
  to worlds: v4 #6303.
- Agent-created agents and any form of self-provisioning.
- Money: quotas count actions and resources, not currency; no
  billing surface is designed here (see §5, question 2).
- Changes to v1's identity, delegation or transport rules — v2
  consumes them as-is.

## 4. Key decisions this iteration must settle

These are design decisions for the v2 design step (not Drew rulings;
each step issue below carries the ones in its scope):

1. **Role configuration format and versioning** — roles live in world
   configuration: file/record shape, how a role edit is versioned,
   and how an in-flight session learns its ceiling changed (§2.1).
2. **Grant/revoke surface** — which client operations create, list
   and revoke grants, and their audit event shapes.
3. **Quota ledger placement** — per-agent ledgers in the policy point
   vs. in the F12.10 account system when it exists; the migration
   must not change the bridge or audit shapes (v1 §9.3 constraint).
4. **Review-queue mechanics** — how an above-ceiling request is held,
   expires, and executes on one-shot approval without becoming a
   delegation.
5. **Concurrent-agent limits** — per-role concurrency caps and join
   refusal semantics under contention (§2.2).
6. **Agent profile split** — v1 §9.1 deferred this: if multi-agent
   field experience shows agent needs diverging structurally from
   shells, v2 is where the agent profile may split from the shell
   contract. Default remains *no split* until evidence says otherwise.

## 5. Open questions needing Drew's ruling

Listed explicitly per this issue's acceptance criteria. None blocks
the step breakdown in §6; each step issue names the question it
depends on, and work that does not depend on a ruling can start.

1. **Third-party citizens:** may an agent hold a role in a world
   whose owner is *not* the agent's owner (a guest citizen under
   someone else's world configuration)? v1 assumes owner = grantor.
   Guest citizenship changes the trust reading of §2.1/§2.5 (whose
   review queue, whose quota funds the agent) and v4's hiring model
   will inherit the answer.
2. **Quota denomination:** are per-agent quotas purely action/resource
   counts, or should a role's budget be able to denominate real cost
   (host compute, model spend) with a hard stop at a stated ceiling?
   §2.3 designs counts; cost-denominated budgets are a different
   promise to owners.
3. **Standing approvals:** §2.5 offers one-shot approval of a refused
   action. Should owners also be able to pre-approve an *action
   class* for a role (a standing, revocable exception), or is
   one-shot-only a deliberate guardrail? Standing approvals drift
   toward silent ceiling edits; the audit can record them either way,
   so this is a product-trust call, not a mechanism call.

## 6. Next-step breakdown (filed as native sub-issues of #6301)

1. **Role configuration** — role definitions as versioned world
   configuration: shape, starter roles, grant/revoke operations and
   their audit events (§2.1, decisions 1–2).
2. **Multi-agent sessions** — concurrent agents under distinct roles:
   per-(agent, role) policy checks, presence/visibility, join
   arbitration and concurrency caps (§2.2, §2.4, decision 5).
   Blocked by step 1 and by v1 spike steps #7914/#7916.
3. **Per-agent quota ledger** — persistent per-(agent, world) budgets
   on the policy point, F12.10-compatible event shape, owner-visible
   spend per agent and role (§2.3, decision 3). Blocked by step 1 and
   v1 spike step #7916.
4. **Owner review tooling** — per-agent review view, one-shot
   approval queue for above-ceiling requests, refusal digests (§2.5,
   decision 4). Blocked by steps 1 and 3.
5. **Citizenship conformance + multi-agent spike run** — conformance
   cases for role checks, quota persistence across rejoins,
   no-delegation-between-agents invariants; scripted run: three
   agents, three roles, one world, full audit (§2, §4 of v1).
   Blocked by steps 2, 3 and 4.

Sequencing note: every step also depends on the v1 spike (#7913–
#7917) for the surfaces it extends; the blocked-by relations above
name the direct edges and the step issues name their v1 edges.

## 7. Dependencies

- v1 design #6300 (landed) and its spike steps #7913 (shell profile),
  #7914 (MCP bridge), #7915 (identity + delegation), #7916 (policy
  point), #7917 (spike run) — v2's steps extend these surfaces.
- F12.10 quota design #1067 via plan of record #6312 — §2.3's ledger
  must stay shape-compatible (v1 §9.3).
- The `id-shell` contract (`pkgs/id/contract/shell-contract.v1.json`)
  — decision 6 revisits its agent profile only on evidence.

## 8. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Predecessor: #6300 + `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md`
- Successors (still gated): v3 #6302 · v4 #6303
