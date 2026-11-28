# Agents in worlds (MCP bridge) — v4 outline: hosted agent services (#6303)

- **Issue:** #6303 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v4 refined outline, filed by queue task 12953 (lane code-a)
- **Predecessor:** v3 outline #6302, landed on main as
  `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
  (PR #10097, merged `e2601e582365`). Entry condition verified fresh
  2026-11-16: v3's design is written and merged; its open questions are
  listed explicitly in its §5, with Q1 expressly carried into v4 ("V4's
  hiring model inherits the answer") and the v3 outline's own §8
  declaring "Successor (entry condition met by this outline): v4
  #6303". #6302 remains open only because its implementation step
  sub-issues (#10049–#10054) are open — a parent-with-open-sub-issues
  state, not an unwritten design. This outline takes v1's mechanisms
  (agent key, UCAN-style delegation, capability ceiling, policy point,
  hash-chained audit), v2's citizenship (roles as versioned world
  configuration, per-agent quota ledgers, owner review tooling), and
  v3's collaboration semantics (boards, shared documents, handoffs —
  coordination only through world state) as given, and covers only
  what changes when an agent is **always-on and hosted**: listed in a
  registry, hired or attached to worlds, and run by a hosting
  substrate (the program's T1 tier) under the public-server trust
  model.
- **Siblings:** v1 #6300 (design landed; spike steps #7913–#7917) ·
  v2 #6301 (outline landed; steps #8299–#8303) · v3 #6302 (outline
  landed; steps #10049–#10054) · epic #6280 · plan of record #6312.

## 1. Governing principles (#1240)

Unchanged from v1 §1 through v3 §1, with the v4 consequence spelled
out: **hosting is a deployment shape, never a capability tier.** A
hosted agent holds exactly the agent key, role ceiling, and quota
ledger it would hold running on its owner's own machine; the host
can run its process but cannot widen its authority, read its sealed
state, or act in its name beyond the delegations the agent's owner
issued. Local-first always; hostable by anyone at any scale — the
registry and hire protocol in this outline are things any operator
can run, not a service only we can provide; p2p behaviour identical
to hosted behaviour — a hired agent joins, claims, edits, and hands
off through the same world actions as a local one (v3 §2.1); our
hosted instance is the public backstop, never a requirement.

## 2. Scope

### 2.1 The registry: agent programs as listable services

- A **listing** is a signed, versioned record: the agent program's
  identity (its agent key's public half), a program manifest (what
  it does, which world surfaces it uses, its resource profile), the
  roles it is willing to be hired under, its operator, and its
  availability terms (always-on, scheduled, on-demand wake).
- Listings are **discoverable, not endorsed**. The registry is an
  index with provenance (who listed it, when, under which operator),
  not a trust decision: a world owner hiring from the registry makes
  the trust call, and the listing's history — worlds served, claims
  completed, claims expired, review outcomes, all as audit references
  — is the evidence that call is made from.
- A listing can be **suspended by its operator or its owner** at any
  time; suspension stops new hires and starts the drain in §2.3. A
  registry operator can delist for abuse, which is itself an audited
  act with the evidence attached — delisting is a public record,
  not a silent removal.

### 2.2 Hire and attach: how a hosted agent enters a world

- **Hiring is a world action by the world owner** (or a role the
  owner has delegated hiring to, where world configuration allows):
  it names the listing, the role the agent will hold (an ordinary
  v2 role — hiring creates no special role class), the quota class
  and ledger the agent's actions charge (v2 §2.3, §5 question 2
  below), and a hire term (open-ended, until a date, or until a
  work quota is spent).
- The hire issues the agent a **scoped delegation** in v1's terms:
  this world, this role ceiling, this term. The agent's host
  receives the delegation, never the owner's keys and never a
  broader token. Attach is the lightweight form: an already-hired
  agent is attached to an additional board or document set within
  the same world and role — a world-configuration change, not a
  new hire.
- **Join semantics are v2's, unchanged.** The hired agent joins
  under the world's join rules, is subject to the same claim
  leases, population caps, and rate budgets as any citizen (v3
  §2.5), and its hosted-ness is visible in the world's population
  rendering: owners and visitors can always see which agents are
  hosted, and by whom. There is no stealth hosting.
- **Multi-owner worlds inherit v3 §5 Q1.** A hosted agent serving
  in a world owned by someone other than its own owner is the
  general case of that question; the hire record names whose
  configuration, ledgers, and review queue govern (§5 question 1).

### 2.3 Always-on runtime contract

- **Presence is leased, like claims.** A hosted agent's standing
  presence in a world is a renewable lease held with the host's
  heartbeat; a host that stops heartbeating loses the presence at
  lease expiry, and the agent's outstanding claims expire under
  v3 §2.2's ordinary claim leases — a dead host strands nothing.
- **Sleep and wake are first-class.** An always-on listing may
  declare event-driven wake (on claim offers, mentions, board
  changes in its worlds) instead of continuous presence; the
  world-visible difference between asleep and absent is that an
  asleep agent's lease and hire remain in force. Wake events are
  world events, audited like any other.
- **Drain, don't drop.** Ending a hire — term reached, quota spent,
  suspension, owner revocation — is a drain: the agent stops
  taking new claims, hands off or releases its in-flight items
  with v3 §2.4 handoff notes, and only then detaches. A host kill
  without drain is the failure case the claim leases already
  cover; drain is the contract that makes it the rare case.
- **Upgrades are re-hires in miniature.** A new program version
  takes over by drain-and-replace under the same listing and hire:
  the hire, role, and ledger persist; the program version that
  acted is recorded in the audit for every action, so an upgrade
  never rewrites who did what.

### 2.4 The public-server trust model, applied to hosts

- The host is **infrastructure, not a principal**: it holds no
  world role, cannot post, claim, or review, and its operator's
  identity is recorded on every hire it serves (the operator is
  accountable for availability, never for content).
- **Sealed agent state.** Anything the agent keeps that is not
  world state — its working memory, its owner's private
  configuration, credentials for its own tools — is sealed to the
  agent key. The host stores and transports sealed state; it
  cannot read it. What a host _can_ see is exactly what any world
  participant can see: the agent's world actions and their audit
  records.
- **The backstop instance adds no privilege.** Our hosted
  instance (the public backstop of #1240) runs this same contract:
  same registry rules, same sealing, same drain semantics. Its
  only special property is operational (it exists so the shape is
  always available to try), never protocol-level.
- **Abuse handling is world-shaped.** A misbehaving hosted agent
  is handled by the world's own instruments — role narrowing,
  claim refusal, quota exhaustion, hire termination — plus the
  registry-level delisting of §2.1. There is no host-level content
  moderation surface to design, because hosts never see a
  moderation-relevant plaintext (§2.4 sealing).

### 2.5 Operator surface

- An operator (anyone running a host) gets: hire/presence/lease
  state for the agents they run, resource use against the
  resource profiles listings declare, drain and upgrade controls,
  and the audit references for the actions their infrastructure
  carried. An operator never gets a world-content surface beyond
  what the world's own visibility gives any participant.
- A world owner hiring hosted agents gets the v2 owner review
  tooling unchanged, plus hire-level views: which listings serve
  their worlds, drain state, and per-hire quota burn against the
  ledger the hire named.

## 3. Non-goals

- **No marketplace mechanics.** No payments, pricing, bidding, or
  revenue share between owners, operators, and agent authors.
  Quota ledgers (v2 §2.3) account for _world_ resources; paying
  for hosting is an out-of-protocol arrangement between the
  parties, if it exists at all (§5 question 2).
- **No new agent-to-agent channel.** Hiring does not reopen v2
  §2.4: hosted agents coordinate only through world state (v3
  §2.1), exactly like local ones.
- **No central registry monopoly.** This outline designs _a_
  registry protocol and record shape; federated or per-community
  registries that speak it are in-scope by design, a single
  canonical registry is not assumed.
- **No hosting substrate build.** The T1 substrate's own design
  (how a host runs programs) belongs to the hosting program; this
  outline fixes only the contract the substrate must honour
  (§2.3–§2.4) and the surfaces it must expose (§2.5).
- **No change to v1–v3 mechanisms.** Agent keys, ceilings, roles,
  ledgers, boards, and claims are consumed as designed; anything
  here that appears to conflict is a bug in this outline.

## 4. Key decisions this iteration must settle

1. **Listing record + registry protocol** — the listing's exact
   fields, its signing and versioning, and the query shape worlds
   use to find candidates (§2.1).
2. **Hire record shape** — what a hire names (listing, role, quota
   class and ledger, term kind), and how attach differs from hire
   in the record (§2.2).
3. **Presence lease + wake contract** — lease durations, heartbeat
   cadence, the wake-event set, and how asleep renders to owners
   and visitors (§2.3, with v3 decision 5's population/rate
   configuration).
4. **Drain protocol** — the drain state machine (announced →
   handing-off → detached), its timeouts, and what a world may do
   when a drain stalls (§2.3).
5. **Sealing boundary** — which agent state is sealed, the key
   hierarchy that seals it, and the recovery story when an agent
   migrates hosts (§2.4).
6. **Operator/owner surface split** — the exact §2.5 views, and
   which of their elements are protocol (any host must expose)
   vs. implementation choice.

## 5. Open questions needing Drew's ruling

Listed explicitly per this issue's acceptance criteria. None
blocks the step breakdown in §6; each step issue names the
question it depends on. Questions 1–2 are inherited from v3 §5
where v4 makes them concrete; 3–4 are new in v4.

1. **Whose ledger pays for a hosted agent (sharpens v3 §5 Q1/Q4):**
   a hired agent's actions charge a quota ledger — the hiring
   world owner's (the hire is a purchase of work), the agent
   owner's (the agent spends its own budget wherever it serves),
   or the world-configured split v3 Q4 already asks about? Hosted
   agents make this unavoidable: the agent, its owner, the world
   owner, and the host operator can be four different parties.
2. **Is hosting itself a chargeable world resource:** does the
   _host's_ cost (compute for always-on presence) enter any
   ledger the protocol tracks, or is hosting purely an
   out-of-protocol arrangement (§3)? The answer decides whether
   §2.5's resource reporting is accounting or merely telemetry.
3. **Backstop listing policy:** may third-party listings be hired
   into worlds from our public backstop instance by default, or
   does the backstop run only first-party/agent-owner-submitted
   programs until a listing-vetting practice exists? This is a
   trust-posture call about the one instance carrying our name.
4. **Acceptance authority for hosted work (carries v3 §5 Q2):**
   v3 asked whether a reviewer-role agent may formally accept
   another agent's work. For hosted agents the question sharpens:
   if acceptance is agent-delegable at all, may a _hosted_
   reviewer accept — closing the loop with no human and no
   locally-run agent in it — or does hosted work always require
   acceptance from a non-hosted party?

## 6. Next-step breakdown (filed as native sub-issues of #6303)

1. **Registry + listing record** — the listing record, signing,
   versioning, suspension/delisting records, and the discovery
   query shape (§2.1, decision 1). Blocked by v3 step #10049
   (board object model) only for its worked examples; the record
   design itself is unblocked.
2. **Hire/attach + delegation issuance** — hire as a world
   action, the scoped delegation it issues, attach semantics,
   hire terms and their expiry (§2.2, decision 2). Blocked by
   v2 step #8299 (role configuration) and v3 step #10049.
3. **Presence, wake, and drain runtime contract** — presence
   leases, wake events, the drain state machine, upgrade-by-
   replace (§2.3, decisions 3–4). Blocked by step 2 and v3 step
   #10052 (agent-majority profile + visitor role).
4. **Host trust + operator surface spike** — sealed-state
   boundary, operator/owner views, and a spike run: one agent
   program listed, hired into a v3 shared world from a separate
   host, drained and replaced mid-work with the audit showing
   continuity (§2.4–§2.5, decisions 5–6). Blocked by steps 1–3
   and the v3 spike step #10054.

Sequencing note: v4's steps sit at the end of the phase-14 chain
by design — every step also depends on the v2 steps (#8299–#8303)
and the v1 spike (#7913–#7917) for the surfaces it extends; the
blocked-by relations name the direct edges and the step issues
name their remaining edges.

## 7. Dependencies

- v3 outline #6302 (landed) and its steps #10049 (board object
  model + claim semantics), #10050 (shared documents), #10051
  (handoff + review flows), #10052 (agent-majority profile),
  #10053/#10054 (conformance + spike) — v4 hires agents into the
  worlds these steps build.
- v2 outline #6301 (landed) and its steps #8299–#8303; v1 design
  #6300 (landed) and its spike steps #7913–#7917.
- F12.10 quota design #1067 via plan of record #6312 — §5
  question 1's answer must stay shape-compatible with the
  per-agent ledger events (v1 §9.3, v2 §2.3).
- The T1 hosting substrate (hosting program) — §2.3/§2.4 are the
  contract it must honour; if the substrate's design lands
  different primitives, this outline's contract sections are the
  parts to reconcile first.
- The `id-shell` contract (`pkgs/id/contract/shell-contract.v1.json`)
  — population and hire rendering go through it; no contract
  change is assumed.

## 8. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Predecessors: #6300 +
  `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` · #6301 +
  `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
  · #6302 +
  `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
- Successor: none — v4 is the last filed iteration of the #6280
  outline chain; further iterations are filed from the core
  topic if the program continues past hosted services.
