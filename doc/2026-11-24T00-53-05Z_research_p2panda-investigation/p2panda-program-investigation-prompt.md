# Investigation: capability auth + p2p transport — p2panda-first, honestly compared

Drew's direction, 2026-11-23 15:28 CST: "we're leaning towards using p2panda for
everything but go back for auth and transport and compare research dig into and
fully understand advantages of each — keyhive/subduction, p2panda-auth/sync,
willow/meadowcap, ocapn/captp/endo/goblins/agoric, capnproto/capnweb and any
other leading options especially if they work well with
rust/cloudflare/sqlite/turso/iroh."

## The question

For the fleet / id / atlas / mcpx estate, choose the **auth** (identity,
capabilities, delegation, groups, revocation) and **transport/sync** (how nodes
reach each other and replicate state) foundations. The current leaning is
**p2panda for everything**. This investigation must stress-test that leaning:
understand each candidate family from its own docs, specs, and code; state each
one's real advantages; and say plainly where p2panda wins, where it loses, and
what a composed stack (p2panda + something) would look like if composition
beats purity.

## The estate it must fit (verified context)

- **fleet**: a Rust binary (queue broker, placement, dispatch) with SQLite /
  Turso (libsql) stores; seats are agents on laptops and sandboxes; GitHub
  Actions runners are an emerging execution surface. A fleet CA program is
  mid-build (Ed25519 / P-256 / RSA-4096 / ML-DSA-44 roots) and an interim IdP
  CA (EC P-384) already issues for a minimal OIDC provider.
- **Transport today**: Tailscale/tsnet (mcpx is Go because tsnet is Go-only),
  Cloudflare Worker doors (commander.security.cab, mcpx.security.cab) with
  OAuth front doors, and a codified route-form family (fleet
  commander/http/ssh). A design for hosts-without-Tailscale + iroh for
  non-Muse agents + a generic pool primitive is recorded (#13752): iroh
  NodeId = ed25519 identity, relays + hole punching, self-hosted relay/pkarr.
- **Data**: Turso/libsql + SQLite everywhere; Cloudflare Workers + KV + R2 at
  the edge; NixOS hosts; everything self-hosted where feasible.
- **Hard preferences**: Rust-first for new core work; no connection/secret
  material in public logs; capability attenuation over ambient authority where
  the model supports it; offline/local-first operation must not be an
  afterthought; measured claims only — maturity and performance statements
  carry sources.

## Families in scope

1. **p2panda** — core (operations/logs, data models), **p2panda-auth**,
   **p2panda-sync**, its iroh-based networking, encryption story, storage.
2. **keyhive / subduction** — Ink & Switch-lineage key management, member
   graphs, document encryption, delegation; the Subduction sync protocol.
3. **willow / meadowcap** — the Willow protocol (Earthstar lineage) and its
   Meadowcap capability system; namespaces, subspaces, path addressing.
4. **The object-capability network family** — **OCapN**, **CapTP**, **Endo**,
   **Spritely Goblins**, **Agoric**: what each layer is, interop status,
   implementation languages and maturity, the distributed-objects model vs
   data replication.
5. **capnproto / capnweb** — Cap'n Proto serialization + RPC (capability-based
   RPC design), capnp in Rust, and Cloudflare's Cap'n Web (JS/TS RPC on
   Workers); what each does and does not provide for auth.
6. **The wider field** — any other leading option that fits the estate:
   UCAN, Biscuit, macaroons, Veilid, Holepunch/Hypercore, libp2p, Freenet,
   Holochain, AT Protocol patterns, Noise/WireGuard identity, SPIFFE/SPIRE —
   surveyed briefly, deep-dived only if one plausibly beats the named families
   on an axis that matters here.

## Axes (every family, same axes)

Identity & key management · capability model (delegation, attenuation,
revocation, expiry) · group management · encryption (in transit, at rest,
end-to-end, forward secrecy) · transport & NAT traversal · sync/replication
semantics (CRDT/log/operation-based; offline behaviour; conflict story) ·
storage fit (SQLite/Turso; log-structured?) · Rust maturity (crates, activity,
who maintains) · Cloudflare fit (Workers/KV/R2/Durable Objects; WASM) ·
iroh fit (native, composable, competing) · spec/interop status · project
health (funding, releases, community, license) · operational complexity ·
what it pointedly does NOT do.

## Deliverables

- One research note per family in `research/` (this folder's sibling), written
  from primary sources (project docs, specs, repos, crates.io) with links.
- A comparison + recommendation document: the matrix across the axes, the
  honest case for and against "p2panda for everything", at least one composed
  alternative, and the decision points that remain Drew's.
- Filed as a tracking issue in developing-today/code with the master prompt as
  its first comment, and landed as a dated design record via PR (the
  investigation-program pattern used for id/atlas/fleet/mcpx).
