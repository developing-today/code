# Agents in worlds (MCP bridge) — v1 design + spike plan (#6300)

- **Issue:** #6300 (Phase 14 core topic #6280; principles from #1240)
- **Status:** v1 first-iteration design, filed by queue task 12936 (lane code-t)
- **Entry artifact:** first-pass research findings on #6280 (task 13834, lane
  code-v, comment 6729662452, 2026-11-09) — this design takes each of its
  starting constraints and records accept/reject below (§2), and resolves
  its four open uncertainties (§9).
- **Scope of v1:** exactly one agent, one world, one session at a time —
  joining as an MCP-bridge client, reading screens, posting chat, and
  performing one bounded program interaction, under a capability ceiling
  with a full audit log. Everything beyond that (multi-agent, v2 #6301,
  v4 #6303) is explicitly out of scope and stays gated behind this design.

## 1. Governing principles (#1240)

Every decision below is checked against the Phase 14 principles:

1. **Local-first always.** The bridge runs wherever the agent runs —
   including a laptop talking to a local world. No cloud component is
   required for any v1 flow.
2. **Hostable by anyone at any scale.** The hosted edge is an optional
   transport front for the same bridge, not a different product. Anyone
   can host the edge for their own world with the same code.
3. **P2P behavior identical to hosted behavior.** Authority, ceilings,
   quota and audit are properties of the world and the agent's
   delegation, never of the transport. iroh (P2P) and HTTP/MCP (hosted
   edge) carry the same frames and enforce the same rules.
4. **The hosted instance is the public backstop, never a requirement.**
   A world that cannot be reached through our hosted instance must
   still work end-to-end over P2P.

## 2. Disposition of the #6280 findings (accept / reject)

| # | Findings constraint | Disposition |
|---|---|---|
| Q1a | Agent profile needs semantic diffs / region subscriptions, not raw frame streaming | **Accept** — §4.2; delivered as an additive shell-contract revision (§9.1) |
| Q1b | Agent input frames carry the frame/revision they were decided against (stale-decision token) | **Accept** — §4.3; host rejects stale-decision input |
| Q1c | Shell conformance suite is the agent's safety floor; no input rights without passing it | **Accept** — §8, spike step 1 |
| Q2a | Agent identity is its own Ed25519 key (iroh endpoint ID); no separate identity system | **Accept** — §5.1 |
| Q2b | UCAN-style attenuated delegation from owner to agent key; offline-verifiable proof chain | **Accept** — §5.2 |
| Q2c | Delegations short-lived and renewable; revocation latency bounded by token lifetime; no long-lived standing access | **Accept** — §5.3 (defaults in §9.2) |
| Q2d | OAuth 2.1 only at the hosted HTTP edge (RFC 9728 discovery, RFC 8707 resource indicators); world role, not the transport token, is the authority of record | **Accept** — §5.4 |
| Q3a | Capability ceiling set at role grant, enforced by host/policy layer, never by the model or tool annotations | **Accept** — §6.1 |
| Q3b | Quota charged per action; exhaustion parks the agent visibly ("agent parked, quota exhausted" world event), never silent failure | **Accept** — §6.2 |
| Q3c | Audit: one signed/hash-chained event per action incl. refusals; periodic owner-readable summaries | **Accept** — §7 |
| Q4a | Tools = authority-checked world commands only; tool definitions pinned and hash-checked (poisoning/shadowing/rug-pull) | **Accept** — §4.4 |
| Q4b | Five non-tools: capability mint/widen/delegate; raw credential access / token passthrough; audit-log mutation / quota adjustment; bulk export / existence probing; arbitrary shell/code execution in the host | **Accept, unmodified** — §4.5 |
| — | (no findings constraint rejected) | All constraints accepted; the design's open choices are confined to §9 |

## 3. Shape of the system

```
owner client ──delegation (UCAN-style, short-lived)──▶ agent key
agent (model + MCP client)
   │  MCP tools (§4.4)                    ▲ audit events (§7)
   ▼                                      │
MCP bridge ── shell frames (F13.1 contract, agent profile) ──▶ world host
   │            over iroh (P2P) or HTTP edge (hosted)           │
policy point (host side): ceiling → quota → audit ─────────────┘
```

- The **bridge** is a translator, not an authority: it maps MCP tool
  calls to named ClientFrames of the F13.1 shell contract
  (`pkgs/id/contract/shell-contract.v1.json`, contract `id-shell`,
  ALPN `/id-world/1`) and renders ServerFrames back as tool results.
  It holds no credentials of its own and no authority beyond the
  agent's delegation (no token passthrough — §4.5).
- The **policy point** sits in the world host, outside the agent's
  execution environment: every frame the agent causes is checked
  against (1) the capability ceiling, (2) the quota ledger, and only
  then executed and (3) written to the audit log — including refusals.
- The agent is **a shell**: its renderer is a model context, its input
  device is tool calls. It joins, observes and acts exactly like any
  other conformance-checked client, with the agent profile of §4.2
  layered on the same contract.

## 4. The bridge surface

### 4.1 Joining a world

1. The agent presents its Ed25519 key (iroh endpoint ID) and its
   delegation proof chain (§5.2) to the world host.
2. The host verifies the chain to the owner, checks expiry and the
   role binding (owner key → role in this world), and confirms the
   agent has passed shell conformance for the contract revision in
   use (§8). Failure at any step is a refused join — audited, with no
   partial presence.
3. On success the host opens a session with: the role's capability
   ceiling (§6.1), a fresh quota budget (§6.2), and a screen
   subscription (§4.2). The join itself is audit event #1 of the
   session.

### 4.2 Reading screens — the agent profile

Raw frame streaming is rejected for agents (findings Q1a: a 1 MiB
full-screen frame is far outside an agent's attention budget). The
agent profile instead provides:

- **Region subscriptions:** the agent subscribes to a screen region or
  an event class; the host sends only changes within the subscription.
- **Semantic diffs:** ServerFrames in the agent profile describe what
  changed in world terms (object appeared/moved/changed, chat message
  posted) rather than repaint instructions.
- Both are **additive** shell-contract additions (§9.1): new optional
  ServerFrame kinds that a shell requests; human shells are
  unaffected, and a host that does not offer them can still serve an
  agent at reduced fidelity (full frames, agent-side diffing) — never
  with different authority.

### 4.3 Acting — input with a stale-decision token

Every agent-originated input frame carries the screen revision it was
decided against. The host applies the input only if that revision is
still current for the affected region; otherwise it rejects the input
as stale and returns the current diff. An agent can therefore never
act blindly on state it has not seen (findings Q1b). Rejections are
audited like refusals (§7).

### 4.4 Tools (the MCP surface)

Each tool is one authority-checked world command, narrowly scoped,
with its definition pinned and hash-checked at session start; a
server that changes a definition mid-session ends the session
(findings Q4a — tool poisoning / shadowing / rug-pull defence):

| Tool | World command | Ceiling key |
|---|---|---|
| `world.join` / `world.leave` | session presence | `presence` |
| `screen.read` | read current screen / region state | `screen.read` |
| `screen.subscribe` | region / event-class subscription (§4.2) | `screen.read` |
| `chat.post` | post a chat message in-world | `chat.post` |
| `object.inspect` | inspect one visible object | `screen.read` |
| `program.invoke` | the one bounded program interaction (§4.6) | `program.invoke:<program-id>` |
| `presence.set` | manage the agent's own presence | `presence` |

Tool annotations (`readOnlyHint`, `destructiveHint`, …) are treated as
untrusted UI hints only (findings Q4b); enforcement is the ceiling.

### 4.5 Non-tools (never on the bridge, findings Q4b)

1. Minting, widening or delegating capabilities (owner-only, out-of-band).
2. Raw credential/key access or token passthrough to third-party APIs.
3. Audit-log mutation or quota adjustment.
4. Bulk export or existence-probing operations (an agent must not
   learn what it may not see by enumerating).
5. Arbitrary shell/code execution in the world host.

### 4.6 The one bounded program interaction

The v1 spike's single program interaction is: the agent invokes one
named, owner-approved program on one named object, with arguments
drawn from a schema fixed in the role grant
(`program.invoke:<program-id>`). Bounds: one invocation per call, no
chaining, no program-defined sub-calls escaping the ceiling, result
rendered back as a semantic diff. The spike uses the simplest existing
world program that mutates a single object; the design deliberately
does not generalise to arbitrary program calls in v1.

## 5. Identity, roles, delegation

### 5.1 Agent identity

An agent holds its **own Ed25519 keypair**, distinct from its owner's.
On iroh the endpoint ID *is* the public key and connections are
mutually authenticated under it — no separate identity system exists
or is needed (findings Q2a). The world binds the agent key to a role;
the key alone grants nothing.

### 5.2 Roles and attenuated delegation

- A **role** is a named bundle: capability ceiling (§6.1) + quota
  profile (§6.2) + the list of worlds it applies to.
- The owner **delegates** a subset of their own capabilities to the
  agent's key, UCAN-style: delegations only ever attenuate, every
  token carries an expiry, and the proof chain back to the owner is
  verifiable offline (findings Q2b). The host verifies the chain at
  join and at renewal; it never trusts the bridge's word for it.
- An agent cannot delegate further in v1 (delegation depth 1), and can
  never delegate to itself a capability it was not given (§4.5.1).

### 5.3 Lifetime and revocation

Offline-verifiable capabilities cannot be un-issued (findings Q2c),
so revocation is engineered, not wished for: delegations are
**short-lived (default 1 hour, §9.2) and renewable** — the owner's
client renews while the agent behaves; stopping renewal *is*
revocation, with latency bounded by the token lifetime. Verifiers
that are online additionally check revocation records. Long-lived
standing access for agents is refused by policy.

### 5.4 The hosted edge

Where a hosted world fronts agents over HTTP/MCP, the edge is an
OAuth 2.1 resource server per the MCP authorization profile:
Protected Resource Metadata discovery (RFC 9728), resource indicators
(RFC 8707) so a token minted for one world cannot be replayed at
another, per-tool scope step-up. The OAuth scopes are a *transport
representation* of the world role; the **role remains the authority
of record** (§9.4) — a valid transport token with no matching role
binding and delegation chain grants nothing.

## 6. Guardrails

### 6.1 Capability ceiling

Set at role grant, enforced at the host-side policy point on every
frame, never by the model (§3). The ceiling names: tools, worlds,
object classes, and the program-invoke allowlist. Anything outside it
is refused and audited. The ceiling can only be changed owner-side,
out-of-band.

### 6.2 Quota

Per-agent, per-session budgets over the expensive axes: frames
requested, tool calls, program invocations, and (where the world
meters it) world-side compute. Every action is charged *before*
execution; a charge that would exceed the budget refuses the action.
Exhaustion **parks** the agent visibly — "agent parked, quota
exhausted" is a first-class world event the owner sees — never a
silent failure (findings Q3b). Assumption on F12.10 sequencing is
stated in §9.3.

### 6.3 Behavioural guardrails

- Rate limits per tool in addition to quota (chat.post is the
  spam-prone surface; its limit is part of the role).
- The stale-decision rule (§4.3) is a guardrail, not just plumbing:
  no acting on unseen state.
- Presence honesty: the agent's in-world presence is labelled as an
  agent, with its owner's name, wherever a human's would be.

## 7. Audit log

- **One event per agent action**, including refusals and stale-input
  rejections: agent key, owner, world, tool, object, capability proof
  reference, quota charged, outcome. Refusals are the highest-signal
  events and are never sampled away.
- Events are **signed / hash-chained** with the world's journal, so a
  hosted operator cannot quietly edit an agent's history.
- **Owner summaries:** the raw log is for machines; owners get
  periodic summaries ("your archivist agent edited 14 objects, spent
  X, was refused twice") — the review surface an owner will actually
  read (findings Q3c).
- The agent is the audit *subject*, never its editor (§4.5.3).

## 8. Conformance and safety floor

The existing F13.1 shell conformance suite is the agent's safety
floor: an agent build that cannot pass shell conformance does not get
input rights (findings Q1c). The spike adds agent-profile cases:
semantic-diff rendering, stale-decision rejection, ceiling refusal,
quota parking, and tool-definition pinning. Passing conformance is
checked at join (§4.1).

## 9. Resolved uncertainties (from the #6280 findings)

1. **Semantic-diff profile — additive revision, not a separate
   profile document.** Region subscriptions and semantic diffs ship
   as an additive revision of the `id-shell` contract (new optional
   ServerFrame kinds + a capability flag), keeping one conformance
   suite and one versioning story. If field experience shows agent
   needs diverging structurally from shells, v2 can split the
   profile — v1 does not pre-pay that cost.
2. **Delegation lifetime — 1 hour default, owner client renews.**
   Minutes are too chatty for a working session; hours bound
   revocation latency acceptably. The renewal actor is the **owner's
   client** (the host must not be able to renew access to itself);
   the host only verifies. Roles may set 15 min – 4 h; outside that
   range requires owner-side policy change, not a role edit.
3. **Quota sequencing — the spike does NOT wait for F12.10 (#1067).**
   Per plan of record #6312, #1067's design lands first, but the
   agents spike implements §6.2 as a *local, per-session* budget
   inside the policy point (same event shape F12.10 will consume),
   so the spike can run before world's quota system exists and be
   replaced by it without changing the bridge or the audit shape.
4. **Hosted-edge role mapping — scopes name the role, the role is
   the authority.** OAuth scope strings carry the world + role
   (`world:<id> role:<name>`); the edge validates the transport
   token, then the host re-derives authority from the role binding
   and the delegation chain (§5.4). A token can therefore be *valid*
   and still grant nothing — transport success never mints authority.

## 10. Spike plan (v1 implementation breakdown)

The spike proves §4–§7 with the smallest end-to-end build. Steps are
filed as linked implementation issues from #6300 (native sub-issues
of #6300, cross-linked from #6280):

1. **Agent shell profile in the contract** — additive `id-shell`
   revision: region subscriptions, semantic-diff ServerFrames,
   stale-decision token on input frames; conformance cases (§4.2,
   §4.3, §8).
2. **MCP bridge (read + chat)** — the bridge binary: join/leave,
   screen.read / screen.subscribe, chat.post, object.inspect, with
   tool-definition pinning (§4.1, §4.4).
3. **Identity + delegation** — agent key handling, role binding,
   UCAN-style attenuated delegation with 1 h default lifetime and
   owner-client renewal; hosted-edge OAuth mapping (§5).
4. **Policy point: ceiling + session quota + audit** — ceiling
   enforcement, per-session quota with visible parking, hash-chained
   audit events incl. refusals, owner summary rendering (§6, §7).
5. **Bounded program interaction + end-to-end spike run** —
   `program.invoke` for the one named program, then the scripted
   spike: one agent joins, reads, chats, invokes, leaves — with the
   full audit log and conformance suite green (§4.6, §8).

Sequencing: step 1 before step 2; step 3 before any non-local run;
steps 4–5 last, together. Each step's issue carries its own
acceptance criteria derived from the sections it names.

## 11. Non-goals for v1

- Multi-agent worlds, agent-to-agent delegation, agent-created agents.
- Arbitrary program interaction beyond §4.6.
- Cross-region / hosted-federation behaviour changes — the bridge
  inherits the world's existing rules unchanged.
- Anything in v2 #6301 or v4 #6303.

## 12. Links

- Core topic: #6280 · Principles: #1240 · Plan of record: #6312
- Entry artifact: first-pass findings on #6280 (comment 6729662452)
- Shell contract: `pkgs/id/contract/shell-contract.v1.json` (`id-shell`)
- Sibling ladder (still gated): v2 #6301, v3 #6302, v4 #6303
- Spike breakdown (sub-issues of #6300, filed by task 12936): step 1
  #7913 (agent shell profile) · step 2 #7914 (MCP bridge; blocked by
  #7913) · step 3 #7915 (identity + delegation) · step 4 #7916
  (policy point) · step 5 #7917 (bounded program interaction + spike
  run; blocked by #7914, #7915, #7916)
