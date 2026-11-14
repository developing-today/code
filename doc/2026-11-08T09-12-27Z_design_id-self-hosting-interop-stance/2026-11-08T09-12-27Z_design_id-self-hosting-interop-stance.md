# Self-hosting and interop stance

Issue: [#5975](https://github.com/developing-today/code/issues/5975)
Program: [#5955](https://github.com/developing-today/code/issues/5955) (statements S78–S81 of the 2026-11-04 brief)

## Decision

The system is built to be a real place that people can share, create in, and
deal with one another in **without requiring our hosting, our servers, or our
involvement**. Three positions follow from that requirement, and this record
states them as decisions so sibling work can test against them:

1. **Interoperability is a protocol property, not a hosting relationship.**
   A server or client interoperates when it satisfies the interop contract
   below — whoever wrote it, whoever hosts it, and whether or not it ever
   talks to infrastructure we run.
2. **Self-hosting is a supported shape, not an accident.** Running the whole
   system, or a supported subset of it, on your own servers is a first-class
   deployment. That genuineness is load-bearing: the AGPL/licensing analysis
   for this program depends on self-hosting being really possible, not
   theoretically possible.
3. **Our defaults are our choices, not the protocol.** Permission defaults,
   licensing-policy defaults, and moderation defaults ship with our product;
   they bind nobody else. A self-hoster who changes all of them has changed
   their policy, not broken the protocol.

## The interop contract

An implementation **conforms** — and therefore interoperates — when it
satisfies every load-bearing element below. An implementation that satisfies
none or only some of them may be a fine system, but it is **merely separate**:
it does not interoperate, and no amount of shared code changes that.

Load-bearing (protocol-level; changing one of these changes interoperability):

- **Wire format.** The message and event encodings on the wire, including
  their versioning rules. Two implementations interoperate only if each can
  parse what the other emits and reject what it cannot honour. The concrete
  format is being settled in
  [#5674](https://github.com/developing-today/code/issues/5674); this stance
  fixes only that the wire format _is_ load-bearing, and defers its unsettled
  details there.
- **Identity.** How participants are named and how a name is bound to proof
  of control (keys, signatures, delegation). An implementation that invents
  its own identity binding cannot authenticate ours, and vice versa.
- **Capability negotiation.** How peers discover and agree what the other
  supports before relying on it — including attenuated capabilities and
  their ceilings — so a peer is never silently assumed to hold a capability
  it never offered.
- **Registry format.** The shape of registry entries and their references,
  so a world, service, or participant registered in one implementation can
  be resolved by another.

Not load-bearing (product choices on top of the protocol):

- Chat and world user interfaces, and whether they are present at all.
- Our registry as a service (as opposed to the registry _format_).
- Permission, licensing, and moderation defaults (see below).
- Hosting topology: relays, bridges, and residency arrangements an operator
  picks for their own deployment. Boundary enforcement across bridges,
  federation, and relays is specified in
  [#5882](https://github.com/developing-today/code/issues/5882); residency
  implementation is [#5529](https://github.com/developing-today/code/issues/5529);
  writer sets and federation mechanics are
  [#5789](https://github.com/developing-today/code/issues/5789).

### Rust-library reuse is not conformance

Reusing our Rust libraries is the fastest route to a conforming
implementation, and it is encouraged — but **reuse alone does not guarantee
interoperability**. A subsystem built from our libraries can still pin an
old wire version, skip capability negotiation, or register entries in a
divergent shape, and then it is merely separate despite sharing our code.
Conversely, a from-scratch implementation in any language that satisfies
the contract interoperates. Conformance is judged against the contract,
never against a parts list.

### Proof

This stance states the requirement; it does not prove it. The conformance
proof obligation lives in
[#5814](https://github.com/developing-today/code/issues/5814) (adapter
conformance harness and interop matrix): every load-bearing element above
gets a harness check and a matrix row, so "does it actually interoperate"
has a testable answer per element rather than a claim.

## Supported self-hosting shapes

Supported — a deployment we intend to keep working, and whose breakage is a
bug:

- **Full system.** Worlds, chat, and registry together, on the operator's
  own servers — the exact same system we host, run by someone else.
- **Worlds without chat.** The world/event substrate with no chat surface.
  Chat is a product layer over the substrate, not a substrate requirement.
- **Chat only.** Chat over the same identity and capability machinery,
  without hosting worlds.
- **Registry only.** A registry service speaking the registry format, so
  other deployments can resolve entries without adopting our hosting.

Unsupported or accidental — may happen to run today, but no compatibility
is promised and breakage is not a bug:

- Partial-protocol deployments that speak the wire format but substitute
  their own identity binding or skip capability negotiation, while claiming
  to interoperate. (They are welcome as separate systems; the claim is the
  problem.)
- Mixed-version fleets outside the wire format's own versioning rules.
- Subsets that drop a load-bearing element their peers rely on — for
  example a worlds host that cannot resolve registry references its worlds
  contain.

## What is explicitly not required of a self-hoster

- Running, or connecting to, any server we operate.
- Our permission defaults, our licensing-policy defaults, or our moderation
  defaults. These are **server-admin choices** on a self-hosted server.
  Adopting different ones is a policy difference, never a protocol
  violation, and tooling must not treat it as non-conformance.
- Our chat or world clients; any conforming client is a full client.
- Announcing the deployment to us, registering it with us, or accepting
  updates from us.

## References

- [Issue #5975](https://github.com/developing-today/code/issues/5975) — this stance's tracking issue
- [Issue #5955](https://github.com/developing-today/code/issues/5955) — parent worlds design program
- [Issue #5674](https://github.com/developing-today/code/issues/5674) — wire format and transport adapters (unsettled wire details defer here)
- [Issue #5814](https://github.com/developing-today/code/issues/5814) — adapter conformance harness and interop matrix (the proof obligation)
- [Issue #5789](https://github.com/developing-today/code/issues/5789) — writer sets v2 / federation
- [Issue #5882](https://github.com/developing-today/code/issues/5882) — boundary enforcement across bridges, federation, and relays
- [Issue #5529](https://github.com/developing-today/code/issues/5529) — residency implementation
- [AGPL/GPL hosting research](../2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting/2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting.md) — why genuine self-hosting matters to the licensing analysis
