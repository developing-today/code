# Walking skeleton for #9106: the path through the 126 rows

**Issue:** #12232 (plan step B1, `planning/plans/2026-11-21_results-first.md`).
**Design of record:** the #9106 summit set,
`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/` (PR #9238). The
39-component sequence and its dependencies on #9078/#9091 stand as
recorded there and on #9112 (plan decision D5). This record moves no
row: it names which of the 126 mapped rows the one path runs through,
in the order the path runs through them, and the interfaces they
share. Every row not named here stays in its recorded slot in
`crosscut/sequencing.json` and the part manifests.

The plan records B1 as "done in code rather than as a document"
(#12268). The code is the primary record — the path's tests cite the
modules below — and this note is the row-level map the plan's table
asks for ("the one path through the 126 rows; the shared interfaces
and their single owners; the integration test's oracle").

## The five steps

From #12232:

1. Write a signed record at an `id://` address on node A.
2. Store it.
3. Sync it to node B over the existing transport.
4. Write a conflicting record on B.
5. Get a conflict report on both nodes.

## The rows on the path, in path order

Order follows the manifests' recorded `requires` edges between the
named rows. "Realized as" is the module on `main` and the test that
exercises it on the path; a row whose module is merged but which the
path does not execute is not listed.

| # | Row | Component | What the path takes from it | Realized as |
|---|-----|-----------|-----------------------------|-------------|
| 1 | `9110-02` | #9110 | Versioned port contracts and the federation slice registry the path's modules hang from | `pkgs/id/crates/id-sync/src/federation/mod.rs`, `federation/slices/mod.rs`; module reach enforced by `.github/scripts/check-rust-module-reach.sh` (#12268, decision D13) |
| 2 | `A9120-content-id` | #9120 | The raw-blake3-256 digest that names a record in its address and as a version's event id | `id_core::id_shape::ContentId` (`crates/id-core/src/content_id.rs`) |
| 3 | `A9113-profile-dispatch` | #9113 | Hash-profile dispatch: verify bytes against the digest an address names | `id_core::hash_profile::ProfileDispatch` (`crates/id-core/src/hash_profile.rs`) |
| 4 | `A-GRAPH-9125-01` | #9125 | Typed triples and the canonical signed graph record a writer signs | `id_core::graph::{term, record}` (`crates/id-core/src/graph/`) |
| 5 | `A9114-envelope-identities` | #9114 | Payload / body / signature / signed-envelope identity separation for the record envelope | `id_core::signed_envelope` (`crates/id-core/src/signed_envelope.rs`) |
| 6 | `A9117-uri-parser` | #9117 | Strict parser for the `id://` address (authority, namespace, content target) | `id_core::id_uri_parse::IdUriV1` (`crates/id-core/src/id_uri_parse.rs`) |
| 7 | `A9117-uri-format` | #9117 | Canonical formatter and the caller-local resolver-hint check a reader applies | `id_core::id_uri_format` (`crates/id-core/src/id_uri_format.rs`) |
| 8 | `B32` | #9141 | The mandatory universal signed-and-validated acceptance gate every write and every received version passes | `id_sync::federation::slices::b32_ingest_gate` (`crates/id-sync/src/federation/slices/b32_ingest_gate.rs`) |
| 9 | `B07` | #9142 | The store a node keeps versions in, and the first-class conflict-report artifact (steps 2 and 5) | `id_sync::federation::slices::b07_conflict_report::{ConflictStore, ConflictReport}` |
| 10 | `A9120-content-store` | #9120 | The durable form of step 2: versions and reports kept as verified content on disk | `id_store::content_store` (`crates/id-store/src/content_store.rs`), via `id_sync::federation::record_store` — slice B2(b), #12492 / PR #12497 |
| 11 | `B08` | #9142 | Sync scope: the scope / trust-domain / retention bounds a version must satisfy to cross a link | `id_sync::federation::slices::b08_sync_scope::SyncScopeConfig` |
| 12 | `B09` | #9142 | Divergence detection and anti-entropy: inventories compared digest-first, missing signed versions moved, never merged | `id_sync::federation::slices::b09_anti_entropy::{Replica, reconcile, divergence}` |

Two pieces of the path are not mapped rows and are recorded here so
the map is complete:

- **The join (decision D14, owner #12232).** Parts A and B each have
  their own envelope; the path joins them by fixing that a version's
  body is the signed record's canonical bytes, its event id is the
  record's envelope digest (the same 32 bytes the `id://` address
  names), its object is the record's logical graph, and its parents
  are the records it follows. Slice B2(a) (#12279, merged) checks the
  join at admission in `id_sync::federation::record_join`, so a broken
  join is refused on the way in, not only on read.
- **The transport (step 3's wire).** The existing transport is
  `id_sync::envelope_net` — iroh, ALPN-framed JSON, pre-#9106
  substrate. Slice B2(c) carries the B09 exchange over it between two
  processes; no mapped row owns the wire, and none is invented for it.

Step 4 (the conflicting write on B) is step 1's rows again, run by
the second node: two records that follow the same record in one graph
are two heads, which is exactly the conflict B07 reports in step 5.

## Shared interfaces and their single owners

| Interface | Owner row(s) | Surface |
|-----------|--------------|---------|
| `id://` address | `A9117-uri-parser`, `A9117-uri-format` | `IdUriV1::parse` / `IdUriV1::format`, `resolve_context` |
| Signed record | `A-GRAPH-9125-01`, `A9114-envelope-identities` | `GraphRecord::sign`, `signed_envelope::Envelope`, `graph::record::open` |
| Address ↔ version join | #12232 (D14) | event id = envelope digest = address digest; object = logical graph |
| Admission gate | `B32` | `b32_ingest_gate::{Envelope, sign_envelope, SignerAuthorization}` checked by `ConflictStore::ingest` |
| Store + conflict report | `B07` | `ConflictStore::{ingest, heads, open_report, report}` |
| Durable store | `A9120-content-store` | `id_store::content_store` behind `federation::record_store` |
| Scoped exchange | `B08`, `B09` | `SyncScopeConfig`; `Replica::{produce, receive, inventory}`, `reconcile`, `divergence` |

## One clarification, recorded rather than silently absorbed

B32's manifest entry records `requires` on `B05` (signer identity
binding) and `B31` (peer classes / unknown-peer default). On the
path, those concerns are realized **inside** the gate module — the
`SignerAuthorization` table and its `UnknownSigner` / `SignerMismatch`
refusals are `b32_ingest_gate`'s own — and the path does not route
through the separate `b05_signer_binding` / `b31_peer_classes` slice
modules. This record states that mapping; it does not re-order,
reassign or retire B05 or B31, whose rows keep their recorded slots
and their own tests (`federation_b05`, `federation_b31`).

## The oracle

One integration test does all five steps and fails if any module on
the path is stubbed: `crates/id-sync/tests/substrate_sync_path.rs`
(in-process form, #12268; admission form, #12279; durable form,
#12492), with step 1's address/read half in
`crates/id-core/tests/substrate_path.rs`. Oracles are differential
(what B reads equals what A signed; both nodes report the same
conflict key, versions and base), metamorphic (the outcome does not
depend on which side starts the exchange or on repetition), and a
plain-set model of the heads. B2(c) re-runs the same five steps with
the exchange carried over `envelope_net` between two processes; B3
(#12232 step 3) drives the same path from the CLI.

## References

- #12232 — the walking-skeleton issue; the join decision is recorded
  in its comments (2026-11-22 progress note) and as plan decision D14.
- #9112 — the delivered 39-component sequence (D5).
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/` — summit design
  set: `crosscut/sequencing.json`, part `implementation-slices.json`
  manifests (the 126 mapped rows: 384 statements mapped to issue rows,
  delivery gate in `delivery/delivery-plan.md`).
- `planning/plans/2026-11-21_results-first.md` — plan steps B0–B3 and
  decisions D5, D13, D14.
