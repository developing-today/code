# Residency, jurisdiction and boundaries

Residency answers a question the rest of ID mostly ignores: where is this data allowed to be? A native node keeps its stores on its own disk, under its operator's control. A hosted user, a relay, or a federated peer may be in a different legal jurisdiction, and some data — a hosted user's store, a world's blobs, a bridge peer's copy of the event log — must stay inside a named jurisdiction or not move at all. This chapter describes the residency model as it ships, the boundary machinery around it, and, plainly, the one part that does not ship yet: the fenced jurisdiction migration control plane.

## The model

The residency vocabulary lives in `id-core`: the modules `residency`, `residency_inventory`, `boundary`, `boundary_surfaces`, `sanctions` and `sanctions_store`. The facade re-exports `residency` at `id::residency`. Routing and synchronisation for residency live one layer up, in `id-events`, as `residency_routes` and `residency_sync`.

On the operator side, residency is declared in the `id serve` server configuration. The server-config TOML has two optional tables owned by `crates/id-cli/src/server_config/boundaries.rs` (#8851): a `[residency]` table and a list of `[[boundaries]]` tables. Both tables are optional; a server config with neither is valid, as is an entirely empty server-config file. The exact field list inside `[residency]` and `[[boundaries]]` was not enumerated in the surveys and is unverified in this chapter; the reference chapter on configuration carries it.

Two signed domains name the boundary artefacts. A boundary evidence record is produced under the domain `id.boundary.evidence.v1`. A boundary declaration is hashed under the domain `id.boundary.declaration.v1`. Declarations state the rule; evidence is what an enforcement point checks. The design behind the shipped inputs — declaration residence, evidence sources and the verifier — is the #8851 record, and the fuller framework of declarations, evidence and enforcement is design work described below.

In `id-cli`, two modules work the boundary at runtime: `boundary_evidence` and `boundary_gate`. In `id-directory`, the `BoundaryGate` trait is the seam the directory consults. The names state the split: evidence is gathered and checked as data; the gate is the point that admits or refuses.

## Jurisdiction fences in the hosted edition

The one place jurisdiction is enforced end to end today is the hosted worker, and it fails closed. The front door consumes resolved context headers set by an authenticated internal caller — `X-Binding-Id` and `X-Jurisdiction`, plus `X-Iroh-Id` — strips every client-supplied internal header, and re-sets them internally as `x-hosted-binding-id`, `x-hosted-jurisdiction`, `x-hosted-routed: restricted` and `x-hosted-iroh-id`. Requests reach the per-user Durable Object only through the jurisdiction-restricted namespace; `src/routing.ts` is the only module that touches the raw `USERS` namespace, and there is no unrestricted fallback.

Inside the Durable Object, every route runs a common guard before it does any work: the routing context must be present, the runtime jurisdiction must match, and the persisted (binding_id, jurisdiction) pair — bound immutably by the first routed request — must match. The object's own state route, `GET /v1/state`, reports binding_id, jurisdiction, schema version and object name, so the fence is inspectable.

Local development shows the fence at its strictest. Local workerd does not implement jurisdiction restrictions, so the worker returns 503 with the error `jurisdiction_unavailable` for every fully valid request when run locally, by design. That behaviour is the subject of [The hosted edition](hosted.md); the point here is that the failure is closed, not open: when jurisdiction cannot be established, nothing is served.

## The migration control plane: status

Moving a hosted store from one jurisdiction to another is the job of the migration control plane in the issue chain #3399 → #3400–#3404. Its status must be stated plainly:

**The #3404 migration control plane core has landed; the hosted wiring has not.** The `migration_control` slice — a fenced jurisdiction migration control plane, 1,202 lines with 17 tests in `crates/id-core/src/migration_control.rs`, plus its wiring edits — landed on `main` on 2026-11-23 (PR #13347), after a flag hold on the landing was cleared by the owner's approval naming #3404. The core is library code in the shipped tree, but no command in this book's CLI reference invokes it and no hosted surface calls it yet: #3404 remains open for the hosted wiring (#3399/#3400/#3402), and no operator can run a jurisdiction migration today.

The forward design — how a fenced migration is proposed, evidenced, approved and executed without ever holding a user's store in two jurisdictions or in none — is carried by the design chapter residency, jurisdiction and boundaries, not by this chapter. This chapter describes shipped behaviour only, and shipped behaviour today is: stores are bound to a jurisdiction at first routed request, the binding is immutable inside the object, and there is no shipped migration path between jurisdictions.

## Boundaries beyond jurisdiction

Not every boundary is a jurisdiction fence. The surveys record boundary machinery in three other places, and a reader should not merge them:

- The sanctions boundary: `id-core` carries `sanctions` and `sanctions_store`, and the design record for the sanctions boundary data model is #5878. The federation layer carries key suspension records in `crates/id-sync/src/federation/slices/b02_key_revocation.rs` — operator maintenance suspension, bounded or indefinite. Suspension is a boundary on keys, not on places.
- The provider declaration lifecycle: a provider declaration moves `Filed → Admitted → Live → Suspended / Superseded / Stale → Tombstoned`. Suspension there is registry state; the provider's contents are unchanged. That lifecycle is covered in [Currency, tokens and metering](currency.md) where the registry family is discussed, because the same declaration machinery serves the program gallery.
- The payload boundary: a decision record (#5908) sets the procedure for decomposing a payload versus attaching it. That is a boundary on representation, not on residency.

## What the design records add

The design archive holds 11 records for this theme — the smallest concept theme in the book, kept as its own chapter because it has its own blocker chain (#3399 → #3400–#3404). They include the Durable Object jurisdiction pinning investigation, the jurisdiction-aware per-user hosted store designs (#2283 and the loose record `2026-11-01_jurisdiction_aware_user_store_design.md`), jurisdiction-partitioned hosted worlds and world blobs (#3397), residency boundaries for the native bridge, federation and relays (#3398), the shared-world residency investigation, the residency and boundaries framework (#5683), the residency evidence model (#5876), the sanctions boundary (#5878), boundary audit and appeals, and the boundary enforcement inputs record (#8851) behind the shipped server-config tables.

Where any of those records describes behaviour beyond the shipped fence, the hosted guard and the server-config tables named above, it is design, not current behaviour, and the design chapter says so record by record.
