# Design intake — hosted editions: PM service, ID service, and fleet tool as workable Cloudflare Workers equivalents

- Program epic: #9078 (native sub-issue of #8987, Turso + Cloudflare epic)
- Components: A #9079 · B #9080 · C #9081 · D #9082 · E #9083 · F #9084 · G #9085 · H #9086 · I #9087
- Intake date: 2026-11-12 (Drew's directive 16:58 CST; architecture amendment 17:03 CST)
- Status: INTAKE SCAFFOLD. This note seeds the design mill's statement inventory and
  framing. The summit session owning #9078's queue item produces the full build-out
  (state-of-the-art expansion → versioned designs → granular implementation issues)
  and grows/replaces sections of this note through versioned design docs.

## 1. Program framing

The PM stack and pkgs/id cannot run on Cloudflare Workers today. This program builds
workable equivalents of the PM service, the ID service, and the fleet tool for the
Workers world, on a multi-tenant data architecture (per-user Turso databases + a
central tenancy DB, content-addressed media in R2, Workers + Durable Objects serving).

**Dual tier-1 platforms (amendment, binding).** Cloudflare + Turso is a primary
platform, AND the faster Rust-based binaries with extended features are also primary.
Neither edition is a reduced sibling or a legacy predecessor. All design language is
parity + deliberate divergence: extended features may exist Rust-side first or only,
where Workers cannot support them.

**Peer principle (amendment, binding).** A self-hosted server (the ID server) is a
peer node in the protocol/architecture — the same participant kind as a hosted-CF
node or a Rust-binary node — differing only in backend complexity (Kubernetes +
multi-TB tiered storage) and, potentially, advanced clustering of ID services.

## 2. Statement inventory (seed — the summit verifies and completes it)

| #   | Statement                                                                                                                                                                                                                                              | Maps to             |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------- |
| S1  | The PM service gets a workable Workers edition: aggregator/dashboard/reconcile equivalents as one TypeScript service, Durable Objects where coordination needs them.                                                                                   | A #9079             |
| S2  | The hosted PM edition consumes the #9060 data model (board registry, hierarchy, Metadata) — same model, new runtime.                                                                                                                                   | A #9079             |
| S3  | The ID service gets a workable hosted edition, built on the existing serverless line (#5524, #5681 + children, #3811) — gaps only, no re-filing covered ground.                                                                                        | B #9080             |
| S4  | The fleet tool's hosted-world capabilities (Metadata stamping/query, session provenance, sync tools) exist as a TypeScript service/CLI against hosted backends.                                                                                        | C #9081             |
| S5  | ID supports Turso/SQLite-over-HTTP as a database store in addition to redb; IndexedDB is evaluated for the browser client; one storage interface, pluggable backends.                                                                                  | D #9082             |
| S6  | Store pluralism is pragmatic, not maximal: CF+Turso is the main line; the abstraction serves the tier-1 editions and the self-hosted backend, not store-count for its own sake.                                                                        | D #9082 (amendment) |
| S7  | Each user has per-user ID database(s) in Turso; a central tenancy DB is the control plane; Workers + Durable Objects serve routing/auth/sync.                                                                                                          | E #9083             |
| S8  | Hosted per-user instances connect and sync their data; everyone can also connect to a large public database we offer.                                                                                                                                  | E #9083             |
| S9  | Media is content-addressed and deduped in R2; sensitivity tiers decide placement/access (R2 + access controls, CDN for public, or inside the Turso DB).                                                                                                | F #9084             |
| S10 | Open question, Drew's sketch: DBs hold IDs/references, content lives once in R2, GC only when ALL references drop; access iff the user's DB references the object or it is public-published in ≥1 DB. Copy and hybrid models are evaluated against it. | F #9084             |
| S11 | This is substantially a second version in TypeScript (Workers limits); the design scopes it plainly.                                                                                                                                                   | G #9085             |
| S12 | Client-side WASM is allowed in selected areas; per component, Rust logic is evaluated for WASM reuse vs TS rewrite.                                                                                                                                    | G #9085             |
| S13 | All base functions work over REST or WebSocket transports (HTTP/2/3, QUIC where provided).                                                                                                                                                             | G #9085             |
| S14 | Direct Turso access to the user's own DB and to the public DB is allowed when prudent, under tenancy/auth rules that define "prudent".                                                                                                                 | G #9085, E #9083    |
| S15 | Hosted services are relying parties for #9077 (GitHub/Google/Microsoft gated login + devtoday OIDC + account linking); the tenancy account model aligns with #9077/#1253 roles. Identity is not re-designed here.                                      | H #9086             |
| S16 | A longer-term self-hosted tier exists in the architecture: a Kubernetes cluster with multi-TB disk arrays as the largest central storage.                                                                                                              | I #9087             |
| S17 | F's primitives are adapter-friendly toward RustFS (S3-compatible), NATS (streams/messaging), and tiered storage — storage classes with different latency/bandwidth, per-tier retention, placement/movement between tiers.                              | I #9087, F #9084    |
| S18 | Both platforms are tier 1 (CF+Turso and Rust binaries); parity + deliberate divergence, never "port down".                                                                                                                                             | all (amendment)     |
| S19 | Our server is a peer, not a special case — same participant kinds, different backends; no protocol special-casing.                                                                                                                                     | E/F/G/I (amendment) |
| S20 | Advanced clustering of ID services is a legitimate self-hosted differentiator to design for.                                                                                                                                                           | I #9087, B #9080    |

## 3. Version plan (mill pattern)

- **V1 — what exists today:** laptop PM stack (aggregator/dashboard/reconcile) on the
  #9060 model; pkgs/id native on redb; ID serverless line as specified in
  #5524/#5681/#3811; fleet binary native. The V1 doc inventories actual current
  state with code citations, including what of the serverless line is built vs
  specified-only.
- **V2 — the hosted editions at parity:** A, B, C on E's tenancy with F's content
  plane and G's transports; D's committed backends (redb + Turso/libSQL).
- **V3 — the self-hosted tier:** I's K8s/tiered storage with RustFS/NATS adapters,
  peer participation, ID clustering.
- **V4 — longer horizon:** whatever the expansion research surfaces (filed as
  designs/issues, never silently merged into V2 scope).

## 4. Open-question index

Each component issue carries its own design questions (read them there). The
program-level questions the summit must resolve or batch to Drew:

1. F's reference-vs-copy decision (S10) — the program's central question; needs
   evidence and a recommendation, batched to Drew if it stays a judgment call.
2. Tenancy↔provisioning relationship: E vs #3811's per-user DO provisioning.
3. The hosted queue question (C): a Turso-backed hosted queue with the same
   repo-task/v1 packet schema, or the laptop queue as the only queue?
4. IndexedDB verdict (D): adopt, defer, or reject — with evidence.
5. Dual-edition divergence discipline (G): shared schemas/fixtures that keep one
   data model honest across Rust and TypeScript implementations.

## 5. Reading list (source material on main / in the tracker)

- #9060's design: `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md`
- ID serverless design: #2046 (closed) and storage line #5681, #5863, #5867.
- Program + components: #9078–#9087. Identity: #9077, #1253, #2852. Data
  unification background: #3142, #1744. Sync tools: #9016, #9073, #9074, #9076.
