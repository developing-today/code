# Implications for the fleet

_Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).
This page connects the census to our own programme: the fleet
transport design ([#13752](https://github.com/developing-today/code/issues/13752)),
the capability-auth + p2p-transport investigation
([#13779](https://github.com/developing-today/code/issues/13779)) and
its synthesis record
([capability auth + p2p transport comparison](../2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison/2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison.md)),
the compile coordinator ([#13765](https://github.com/developing-today/code/issues/13765))
and Actions-runner exec ([#13774](https://github.com/developing-today/code/issues/13774))._

## 1. The transport bet is confirmed, and it is no longer a bet

The #13752 design chose iroh for direct seat-to-seat transport:
NodeId identity, self-hosted relays, hole punching, pkarr/DNS
rendezvous. At design time iroh was a fast-moving pre-1.0 library.
That risk is gone: **1.0 froze the wire protocol** (a v1 endpoint
talks to any other v1 endpoint regardless of minor version or
language), the release train has run monthly since (1.5.0,
2026-11-19), n0's own QUIC fork (noq) versions alongside, and an
independent Go reimplementation exists — evidence the wire spec is
stable enough to reimplement. Building the Rust fleet binary's
transport on iroh 1.x is now building on a platform, not tracking a
project.

Two operational corollaries:

- **Pin to the 1.x line and take the monthly minors.** The pre-1.0
  lines are being sunset on a published schedule (0.35.x relay support
  ends 2026-12-31; the canary lines already lost it).
- **Self-hosting the relay is a supported shape, not a hack.** The
  `iroh-relay` crate is the same code n0 runs publicly, and the
  community pressure visible on the public board — relays for
  coordination only, never for application data — matches our own
  instinct: our relays should be rendezvous and hole-punch
  coordinators, with direct paths carrying the traffic.

## 2. Gossip's road to 1.0 is the near-term moving part

Of the first-party family, the stabilisation order is now explicit:
**irpc 1.0 shipped (2026-11-10), iroh-gossip is on an announced road
to 1.0 (0.103.0, 2026-11-16), blobs is the de-facto stable data
plane, docs lags.** For our programme:

- Anything we design on gossip today (seat presence, broadcast
  notices, the p2panda data plane — p2panda-net _is_ an iroh
  application riding gossip and blobs) should expect one more
  stabilisation step, not a redesign: the wire-freeze pattern core
  went through is the template.
- **New protocol work of our own should be built on irpc 1.0.** It is
  the stable RPC layer blobs and gossip themselves run on; the
  community `iroh-rpc` crates are superseded. This applies directly
  to the live-session layer in the #13779 synthesis (composition C):
  the session-opening and facet machinery sits on iroh streams, and
  irpc is the maintained way to structure request/response over them.
- **Docs is not a foundation to build on yet.** If a design needs
  multi-writer key-value sync today, that is p2panda's log sync (per
  the synthesis), not iroh-docs.

## 3. `subduction_iroh` on iroh 1.0 keeps composition B honest

The synthesis adopted Keyline (Keyhive's auth model) as the estate's
reference semantics for revocation while deferring Keyhive's _code_
until its disclosed gaps close. The transport half of that stack is
no longer hypothetical: **`subduction_iroh` — Subduction's transport
over iroh — was upgraded to iroh v1.0 in mid-2026**, with its own
signed handshake above iroh's TLS because its threat model does not
delegate identity to the transport. Teamtype made the same 1.0 jump.

What this buys us: if and when composition B's code becomes
adoptable, the path runs on the same iroh 1.x transport as everything
else — no second network stack. It also validates a design instinct
worth keeping: a serious consumer layers its own identity handshake
over iroh rather than trusting transport identity alone. Our seat
identity story (fleet CA / interim IdP CA attesting seat keys) is the
same shape, one level up.

## 4. The community's open wants are our shopping list

The most strategically useful finding in the community research is
that **the layers iroh does not provide are publicly acknowledged
wants, not problems we imagined**:

- **Group discovery.** A public Ideas thread asks how to find the
  endpoint IDs of nodes belonging to a group; iroh has no
  group-membership discovery primitive. Our fleet needs exactly this
  (which endpoints are seats of this estate, right now?) — and the
  answer has to come from our own layer: the broker's registry and
  the auth graph, not from the transport.
- **Authorisation.** Iroh authenticates by key and ships no
  authorisation model at all; the board's long-running authorisation
  thread is users working out their own layers. This is the gap the
  #13779 compositions fill — Meadowcap-as-spec for replicated data,
  Biscuit + capnp facets for live sessions — and nothing upstream is
  about to fill it for us.
- **Blinded address lookup.** n0's October 2026 design work on hiding
  lookup metadata matters because our default global path (pkarr via
  a relay/DNS bridge) otherwise leaks who is resolving whom. Watch
  this workstream; when it lands, our address-lookup configuration
  should adopt it rather than inventing a house variant.
- **Post-quantum identity and hardware key custody** are open design
  conversations upstream (Ed25519 endpoint IDs are the thing a future
  quantum computer would forge; TPM/Secure-Enclave custody is an
  active thread). Neither is a near-term action for us, but both
  argue for keeping our identity layer _above_ the transport key —
  CA-attested seat identity that can rotate curves and custody
  without renumbering the fleet — rather than baking raw endpoint
  keys into durable records.

## 5. iroh-willow stays a research artefact

The synthesis named the evidence that would re-open the data-plane
question: Willow's Confidential Sync unpausing _and_ a maintained
Rust sync — "or iroh-willow ported to current iroh." This census
checked: **neither has happened.** iroh-willow is still crate 0.28.0,
unreleased, built on iroh 0.34-era APIs with git patch-overrides, and
independently inventoried as a broken build with no consumers.
Decision point 5 in the synthesis (whether any domain earns the
Willow data model) therefore stays where it was: the _model_ remains
attractive for mutable infrastructure state, Meadowcap-as-spec
remains borrowable without it, and adopting iroh-willow itself would
mean forking and porting it to iroh 1.x as estate-owned code. Nobody
should plan a dependency on it.

## 6. Patterns worth stealing outright

- **Delta Chat's bootstrap discipline.** The largest production user
  ignores global discovery: tickets ride inside an existing
  end-to-end encrypted channel, relays fold into servers users
  already have, realtime identities are ephemeral and separate from
  long-term account keys. Our estate has the same shape available —
  the queue broker is an authenticated channel to every seat — and
  the Actions-exec handoff design (#13774: coordinator establishes
  the session, runner dials out, handle is a public identifier) is
  already this pattern. Prefer it over global pkarr publication for
  anything that is not meant to be publicly resolvable.
- **Two pkarr deployments, two trust stories.** If we publish pkarr
  records at all, publish them the iroh way (our own relay/DNS
  bridge, central-but-untrusted, signatures verified end to end) —
  and never let a design silently slide between n0's relays and the
  Pubky relays as if they were one system.
- **Dumbpipe as the product-shape proof.** The closest public
  relative of our route/exec work is a shipped, packaged tool whose
  entire product is "direct connections by endpoint ID." The fleet's
  route forms (T0–T7) and the Actions-exec work are the same idea at
  estate scale; dumbpipe's clone family (iroh-ssh, pai-sho) shows the
  demand is real and the shape generalises.

## 7. Watch list

| Signal                                                                | Where it shows up              | What it would change                                                                                                             |
| --------------------------------------------------------------------- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| iroh-gossip 1.0 release                                               | n0 blog, crates.io             | Locks the broadcast layer our p2panda option rides on; re-check gossip API deltas then                                           |
| Blinded address lookup landing                                        | n0 blog, iroh releases         | Adopt for our lookup config; closes a metadata leak in the default path                                                          |
| iroh-willow ported to iroh 1.x, or Willow Confidential Sync unpausing | n0 org, worm-blossom           | Re-opens synthesis decision point 5 (Willow data model for mutable state)                                                        |
| Post-quantum identity design converging upstream                      | GitHub Discussions #4532, blog | Feeds our CA/IdP curve planning (decision point 4)                                                                               |
| A group-discovery or authorisation primitive appearing upstream       | Discussions, registry page     | Would shrink the layer we compose ourselves under #13779 — check before building big                                             |
| Discord-only decisions                                                | Not publicly visible           | The public record (Discussions + blog) is what this record tracks; anything decided socially in Discord reaches us late or never |
