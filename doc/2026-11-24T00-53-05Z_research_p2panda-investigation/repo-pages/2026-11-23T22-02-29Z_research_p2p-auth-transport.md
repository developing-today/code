# Research: capability auth + peer-to-peer transport — the underlying notes

This folder is the research base for the auth + transport investigation
tracked in [#13779](https://github.com/developing-today/code/issues/13779).
The investigation's synthesis — the layer-by-layer comparison matrix, the
verdict, three composed stacks, and the decision points left open — already
landed as its own design record:
[Capability auth + p2p transport comparison](../2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison/2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison.md).
This bundle preserves the per-family research that the synthesis was built
from, so the reasoning stays auditable: every page below is a reorganised,
linked digest of one research lane's full notes.

## The question

For the fleet / id / atlas / mcpx estate, choose the **auth** foundations
(identity, capabilities, delegation, groups, revocation) and the
**transport/sync** foundations (how nodes reach each other and replicate
state). The starting leaning was "p2panda for everything"; the investigation
stress-tested that leaning family by family, from primary sources, on a
shared set of axes: identity & key management · capability model (delegation,
attenuation, revocation, expiry) · group management · encryption ·
transport & NAT traversal · sync/replication semantics · storage fit
(SQLite/Turso) · Rust maturity · Cloudflare fit · iroh fit · spec/interop
status · project health · operational complexity · what each pointedly does
not do.

The estate context the research was judged against: a Rust-first fleet
binary with SQLite/Turso stores and agent seats on laptops, sandboxes, and
GitHub Actions runners; a fleet CA program mid-build (Ed25519 / P-256 /
RSA-4096 / ML-DSA-44 roots) and an interim IdP CA (EC P-384) issuing for a
minimal OIDC provider; Tailscale/tsnet today, Cloudflare Worker doors at the
edge, and the iroh transport direction recorded in
[#13752](https://github.com/developing-today/code/issues/13752) (NodeId =
Ed25519 identity, relays + hole punching, self-hosted relay/pkarr).

## The pages

- [p2panda](p2panda.md) — the modular Rust stack (operations/logs, sync,
  discovery, iroh networking, DCGKA group encryption, group-membership
  auth, spaces). The data-plane candidate; its auth is a replicated group
  CRDT, not a capability system — by its own documentation, which names
  Meadowcap and UCAN as the compose-in capability layers.
- [Keyhive, Subduction, Beelay](keyhive-subduction-beelay.md) — Ink &
  Switch's stack: Keyline "convergent capabilities" (a CRDT of signed
  delegations/revocations with the field's best-specified revocation),
  BeeKEM group keys, and the Subduction sync protocol that already ships
  an iroh transport. Adopt the model now; the code is pre-alpha with
  disclosed live gaps.
- [Willow / Meadowcap](willow-meadowcap.md) — the spec family whose
  capability system (receiver-bound, area-scoped, offline-verifiable,
  attenuation as a validity theorem) is the export-grade asset; its data
  model fits mutable named state, its network sync layer is unfinished.
- [The ocap family: CapTP, OCapN, Endo, Goblins, Agoric](ocap-captp-endo-goblins.md)
  — live distributed objects, not data replication: immediate revocation
  and Turing-complete attenuation, no offline story, no Rust presence.
  A rich idea-mine (interposition revocation, petnames, handoffs).
- [Cap'n Proto / Cap'n Web](capnp-capnweb.md) — the only live-ocap
  programming model, proven inside workerd; Level 1 is what actually
  ships. Sits above a transport that supplies identity + encryption and
  beside whatever supplies durable offline authority.
- [UCAN](ucan.md) — the deepest capability-token semantics (delegation /
  invocation split, mandatory monotonic attenuation, receipts), with
  principals that are mechanically the estate's Ed25519 keys. Design:
  adopt now. Rust 1.0 code: not yet dependable.
- [Biscuit and the token family](biscuit-and-token-family.md) — Biscuit
  (the deployable offline token: Rust reference implementation, official
  WASM build, Datalog attenuation, external revocation list), macaroons
  as prior art, and the survey's remaining flags: Holochain's capability
  design, Holepunch's sync model, Freenet, SPIFFE discipline, and the
  Noise/WireGuard channel layer.
- [Veilid](veilid.md) — the privacy-routing framework: a working DHT with
  signed mutable records and receiver-private addressing, no capability
  system, pre-1.0. A narrow complement candidate (rendezvous/discovery),
  not an alternate base.
- [IPFS / libp2p](ipfs-libp2p.md) — the public content-addressed
  distribution network and its modular networking suite: unique
  distribution properties, a fundamental mutable-state mismatch, and a
  2026 stewardship break at the IPFS centre. libp2p stays the interop
  escape hatch.
- [Bluesky / AT Protocol](bluesky-atproto.md) — where Bluesky is heading
  (IETF standardisation, PLC governance handover, permissioned "Spaces"),
  its OAuth + permission-set answer to delegated authority, and the
  scoped read-compatibility path for `pkgs/id`
  (filed as [#13787](https://github.com/developing-today/code/issues/13787)).
- [Worm Blossom catalogue — digest](worm-blossom-catalogue.md) — a
  condensed map of the Willow collective's full public record: 59 weekly
  diary entries and 20 projects, the headline findings (including the
  Pillow collaboration with p2panda), and pointers to the full catalogue.

## Headline shape of the findings

No surveyed system provides identity + capabilities + sync + storage in
one. Every credible stack composes: a transport that proves key possession
(iroh), a capability layer, and a log- or state-based sync core. The
synthesis record carries the full matrix and the recommended default
(p2panda data plane + Meadowcap-as-spec authorisation + broker
short-window renewal now; the Keyline model as the tracked design north
star; Biscuit + capnp-over-iroh for live sessions), plus the evidence that
would change it. These pages are the evidence underneath that record.
