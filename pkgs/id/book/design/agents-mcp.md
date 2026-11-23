# Design: agents in worlds and the MCP bridge

This chapter is design. It describes the agents-in-worlds programme as designed in six records: the four-generation outline series #6300–#6303 and the two #10662 registry records. Six records is the smallest theme in the archive survey for this book, and it is kept as its own chapter because it is the programme's entire MCP surface. The final section states what exists today, and the distinction matters in this chapter more than in any other: the shipped MCP bridge is a translator and policy library only. It has no server and no transport. The live MCP client surface — an agent process connecting over MCP to a bridge that holds a world session — is forward design in this chapter, not shipped behaviour.

Related chapters: [Design: shells and client surfaces](shells-surfaces.md), [Design: capabilities and providers](capabilities-providers.md), [The MCP bridge](../reference/mcp.md), [Agents in worlds: the MCP bridge and policy](../concepts/mcp-and-agents.md).

## Governing principles

All four generations restate the Phase 14 principles from #1240, recorded in §1 of the v1 design, `2026-11-09_design_id6300_agents_in_worlds_v1.md` (2026-11-09):

1. Local-first always. The bridge runs wherever the agent runs, including a laptop talking to a local world. No cloud component is required for any v1 flow.
2. Hostable by anyone at any scale. The hosted edge is an optional transport front for the same bridge, not a different product.
3. Peer-to-peer behaviour is identical to hosted behaviour. Authority, ceilings, quota and audit are properties of the world and the agent's delegation, never of the transport. Iroh (peer-to-peer) and HTTP/MCP (hosted edge) carry the same frames and enforce the same rules.
4. The hosted instance is the public backstop, never a requirement. A world that cannot be reached through the hosted instance still works end-to-end peer-to-peer.

Each later generation adds one consequence of the same principles: v2 states that roles, quotas and review are properties of the world and the delegation; v3 states that collaboration is a property of world state; v4 states that hosting is a deployment shape, never a capability tier.

## Generation 1 — one agent, one world, one session (#6300)

The v1 design scopes itself in one sentence: exactly one agent, one world, one session at a time — joining as an MCP-bridge client, reading screens, posting chat, and performing one bounded program interaction, under a capability ceiling with a full audit log. Multi-agent, citizenship and hosted services are out of scope and gated behind v1.

The shape has three parts:

- The owner issues a short-lived, attenuated delegation to the agent's own key. Agent identity is its own Ed25519 key; on iroh the endpoint ID is the public key. There is no separate identity system. Delegation is UCAN-style: attenuated from owner to agent key, verifiable offline, short-lived and renewable, with revocation latency bounded by token lifetime. OAuth 2.1 applies only at the hosted HTTP edge; the world role, not the transport token, is the authority of record.
- The bridge is a translator, not an authority. It maps MCP tool calls to named client frames of the shell contract (`id-shell`, ALPN `/id-world/1`) and renders server frames back as tool results. It holds no credentials of its own and no authority beyond the agent's delegation. There is no token passthrough.
- The policy point sits in the world host, outside the agent's execution environment. Every frame the agent causes is checked in a fixed order: capability ceiling, then quota charge, then audit. The ceiling is set at role grant and enforced by the host policy layer, never by the model and never by tool annotations.

Four v1 mechanisms carry into every later generation:

- Semantic screen access. Agents receive semantic diffs and region subscriptions, not a raw frame stream. Input frames carry the screen revision they were decided against; the host rejects stale-decision input.
- The conformance floor. The shell conformance suite is the agent's safety floor: no input rights without passing it. The suite is the one defined in [Design: shells and client surfaces](shells-surfaces.md).
- Pinned tool definitions. Tools are authority-checked world commands only. Tool definitions are pinned at session start and hash-checked; a changed definition set is treated as poisoning and ends the session.
- Five non-tools, accepted unmodified from the entry research: capability minting, widening or delegation; raw credential access and token passthrough; audit-log mutation and quota adjustment; bulk export and existence probing; arbitrary shell or code execution in the host.

Quota in v1 is a per-action charge against a per-session budget. Exhaustion parks the agent with a first-class world event — the event text is `agent parked, quota exhausted` — and never fails silently. Audit is one signed, hash-chained event per action, refusals included, with periodic owner-readable summaries.

Spike steps for v1 are filed as #7913–#7917; the bridge spike work is tracked in #7916 and #13202.

## Generation 2 — agents as citizens (#6301)

`2026-11-10_design_id6301_agents_as_citizens_v2_outline.md` (2026-11-10) covers what changes when a world has citizens: configured roles, several agents at once, quotas that outlive a session, and an owner who reviews what their agents did. It takes v1's mechanisms as given.

- Roles become world-owned configuration objects. A role definition — capability ceiling, quota class, delegation-lifetime bounds, review policy — lives in the world's configuration, is versioned with it, and is editable by the world owner through the same surface that edits every other world setting. No role exists only as a string in a token scope.
- Grants are explicit world state: this agent key holds this role in this world, granted by this owner, at this time. Grants are enumerable by the owner and revocable independently of delegation expiry. Role changes are audited world events, and a changed ceiling applies at the next frame check, not at the next join.
- Starter roles ship as ordinary definitions a world can copy and edit: observer, participant, and an archivist-class editor are the examples the outline names. They are defaults, not a privileged tier.
- Several agents hold sessions in one world concurrently, each under its own role, ceiling and quota. The policy point checks per agent and per frame, keyed by (agent, role). A world may cap concurrent agents per role; a refused join is audited with the reason.
- Quota becomes a per-(agent, world) ledger. Budgets are granted with the role, replenish on the role's stated cadence, and persist across sessions. A citizen cannot reset its budget by rejoining. V1's per-session budget is explicitly replaceable by the F12.10 quota work in #1067.
- No inter-agent delegation and no direct agent channel are introduced. What one agent may do about another agent is limited to world actions under its own role. V2 follow-up steps are filed as #8299–#8303.

## Generation 3 — shared agent worlds (#6302)

`2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md` (2026-11-14) covers agents collaborating. Its rules:

- Every coordination act is a world action. Claiming work, handing it off, commenting on it and editing a shared artifact are ordinary world operations under the acting agent's role ceiling, charged to its quota ledger and written to the audit log. There is no coordination surface a host cannot audit, replay and render for owners.
- No new authority flows between agents. A post on a board cannot widen another agent's ceiling. A handoff transfers a task reference, never a delegation; the receiving agent acts under its own role or not at all.
- World chat remains the only conversational surface, under the same frames as every shell.
- A task board is a world-owned object, configured like a role: its columns and states, which roles may create, claim, move and close items, and its visibility are world configuration. An item carries title and body, state, creator, current claim holder (agent and role), a claim epoch, and its full history as audit events. Closing an item is its own capability, separate from claiming it.
- Shared documents carry provenance from the audit log. Worlds whose population is mostly agents, with human visitors, use the same rules; the outline introduces no agent-only world class.

V3 implementation steps are filed as #10049–#10054. The outline itself records that #6302 stays open because those step sub-issues are open, not because the design is unwritten.

## Generation 4 — hosted agent services (#6303)

`2026-11-16_design_id6303_hosted_agent_services_v4_outline.md` (2026-11-16) covers the always-on, hosted agent: listed in a registry, hired or attached to worlds, and run by a hosting substrate under the public-server trust model. Its central rule is stated in §1: a hosted agent holds exactly the agent key, role ceiling and quota ledger it would hold on its owner's machine. The host can run its process; the host cannot widen its authority, read its sealed state, or act in its name beyond the delegations the agent's owner issued.

- A listing is a signed, versioned record: the agent program's identity (the public half of its agent key), a program manifest, the roles it is willing to be hired under, its operator, and its availability terms — always-on, scheduled, or on-demand wake.
- Listings are discoverable, not endorsed. The registry is an index with provenance — who listed the program, when, under which operator — and the listing's history (worlds served, claims completed, claims expired, review outcomes, all as audit references) is the evidence a hiring owner makes the trust call from.
- Suspension is by the listing's operator or its owner, at any time; suspension stops new hires and starts a drain of existing work. A registry operator can delist for abuse; delisting is an audited, public act with the evidence attached, never a silent removal.
- Hiring is a world action by the world owner. It names the listing, an ordinary v2 role for the agent to hold, and the quota class and ledger the agent's actions charge. Hiring creates no special role class.
- There is no central registry monopoly: the registry protocol is one protocol that federated and per-community registries also speak.

The v4 breakdown is four steps: #10662 (listing record and registry protocol), #10663 (hire and attach), #10664 (presence, wake and drain), #10665 (host trust spike). The plan of record for the Phase 14 programme that contains all four generations is epic #6312, under core topic #6280.

## The registry records (#10662)

Two records settle the registry's record family. `2026-11-20T13-35-00Z_design_id10662_hosted_agent_service_listing` (2026-11-20) is the design proposal for v4 step 1, and `2026-11-23T12-46-33Z_design_id10662_listing_record` (2026-11-23) is the decision record that settles it: the listing's exact fields, its signing and versioning, the suspension and delisting records, and the query shape worlds use to find candidates. The 2026-11-23 record designs records and a query protocol only — no registry implementation, no hire mechanics (#10663), no runtime contract (#10664).

Fixed points from the decision record:

- Identity values: an Ed25519 public key is 32 bytes and a signature is 64 bytes. Signed records in this family are wire-stable: a magic and version header, the body as postcard, then the signature; the signature covers the body bytes behind a domain tag so it cannot be replayed over another message kind.
- A listing is not a bundle listing. The existing program registry serves bundle records — author, name, version, payload hash. A hosted listing references an immutable program release by that tuple; it does not re-publish the release's bytes, and removing one listing does not mutate, delete or withdraw the bundle or any other listing backed by the same bundle.
- Versioning: each revision binds its exact predecessor; an identical replay is accepted idempotently; an older revision is rejected as a replacement for a known head; two different records claiming the same identity and revision are unresolved, not resolved by timestamp. A suspended or delisted listing is not reactivated by publishing a new revision.
- Lifecycle events are append-only, authenticated and scoped: suspension applies to the listing identity; delisting is scoped to the registry that issued it, carries a reason class and a safe evidence reference, and does not delete history or the underlying artifact.
- The discovery query is bounded: optional role selector, required advertised surfaces, availability mode, an artifact or agent selector, a bounded page size and an opaque continuation cursor tied to the endpoint, query and snapshot. A result returns verified current listings only; an unresolved proof, history or freshness state excludes a listing from candidates and is reported as an incomplete result, distinct from a verified empty page.
- Role vocabulary: listings speak the starter role names accepted by `WorldScopes::from_role` in `id-caps` — `spectator`, `observer`, `player`, `guest`, `delegator`, `builder`, `moderator`. A world that renames its roles maps its names onto that vocabulary.

## The forward design: a live MCP client surface

The four generations describe an agent that connects, joins and acts. Connecting requires three pieces the design names and the shipped code does not contain: an MCP server endpoint (stdio or HTTP) that accepts a client connection, a transport that carries MCP messages to the bridge, and session management that binds one MCP session to one bridge session and one world session. In the design, that surface inherits every v1 rule unchanged: the server is a front for the translator, the policy point stays in the world host, tool definitions are pinned and hash-checked at session start, and the hosted edge variant of the same surface adds OAuth 2.1 at the edge only. Any implementation of this surface is #6300-series work; the records do not describe it as built, and neither does this chapter.

## What exists today

Grounding, from the web/MCP survey for this book (checkout `1ccb4a19457`, 2026-11-23):

- Shipped: a library-level translator and policy point in `crates/id-caps/src/mcp_bridge.rs` and `crates/id-caps/src/mcp_policy.rs`, exercised by `pkgs/id/tests/mcp_bridge_e2e.rs` and `pkgs/id/tests/mcp_spike_e2e.rs`. The translator is authority-free: it converts one MCP tool call into a shell-contract client frame (JSON), a locally rendered answer from observed server frames, or a session close. It holds no credentials; the only secret-named value it touches is the agent's own join capability, passed through into the `join` frame — the value itself is never printed in this book, only its name.
- Shipped: 8 v1 tools in a pinned order — `world.join`, `world.leave`, `screen.read`, `screen.subscribe`, `object.inspect`, `program.invoke`, `chat.post`, `presence.set`. One listed tool is refused in v1: `presence.set` always returns the no-frame-mapping error, because presence is the join itself and no presence frame exists. Five names are forbidden as tools, matching the v1 design's five non-tools: `capability.mint`, `credential.read`, `audit.mutate`, `world.export`, `host.exec`. The full tool table is in [The MCP bridge](../reference/mcp.md).
- Shipped: the policy layer. A policy session wraps a bridge session and enforces, in order, the capability-ceiling check, a quota charge before execution, and exactly one hash-chained, journal-key-signed audit event per action, refusals included. It enforces the stale-decision rule on `screen_revision` and per-tool sliding-window rate limits. Quota exhaustion parks the agent with the world event whose kind is `agent_parked` and reason `quota_exhausted`. `world.leave` is never refused by ceiling or quota and is charged nothing, but it is audited.
- Not shipped: any MCP server binary, any stdio or HTTP MCP transport, and any JSON-RPC `tools/list` implementation in `pkgs/id`. There is no ID or world MCP entry in the mcpx configuration; the only remote MCP entry there is Atlas Search. There is no gate-token environment variable for the ID MCP bridge.
- Still design: v2 citizenship as world configuration, v3 shared agent worlds, v4 hosted services, and the #10662 registry as a running service. The two #10662 records are a settled record design (2026-11-23) with no registry implementation claimed in them.
