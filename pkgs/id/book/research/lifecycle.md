# Identity, permission, update and content lifecycles

*Research snapshot of November 23, 2026. Code and issue observations refer to the cited revisions and collection times.*

The [Whose authority survives disconnection, recovery and retry?](research-authority.md) separates current authority, historical evidence and receipt visibility after restart. The [Media that remains understandable when the renderer changes](research-media.md) applies those rules to thumbnails, captions and seeks.

Seven design areas have decisions or executable components in the surveyed source. The next work joins these at user-facing entry points. Source inspection establishes what the code contains. The proposed fixtures below have not been run.

## Identity and recovery

The #5884 setup record and parent #5684 require a familiar login ceremony and an honest answer to three questions. Who can sign? Who can deny access? Who can help after loss? Android's product design delegates a scoped device identity to the phone. Successful login after reinstall and restoration of a local journal are separate operations. [Setup ceremony][setup] · [Android design][android] · [Parent issue](https://github.com/developing-today/code/issues/5684)

| Path | Existing decision or implementation | Proposed acceptance fixture |
|---|---|---|
| Existing key or device | Keyset events cover device enrollment and revocation, admin rotation, identity-provider bindings and checkpoints. Authorization depends on chain position. | **Revoked device reconnects.** Enroll a phone, revoke its device key elsewhere, reconnect it and refresh the accepted head. Previously admitted history stays verifiable. A new post-revocation action signed only by the removed key is refused under the refreshed state. Show the owner what happened to pending offline work. |
| Encrypted backup | Backup v1 wraps the identity key client-side. The backup record enforces increasing signed upload epochs, retains deletion replay protection and binds restore confirmations to backup generation and restore epoch. | **Stale confirmation.** Begin restore, delete/recreate the backup, begin another restore, then deliver the original confirmation. The newer restore remains pending. Pre-deletion uploads cannot recreate the deleted backup. Failed decryption preserves the current local identity. |
| Recovery codes | #5888 v1.2 specifies ten one-use 100-bit codes. One unused code can authorize a successor rotation after a cancellable 72-hour claim. Settlement replaces the entire set. This is a decision for custody variant c, not evidence of a shipping code ceremony in every client. | **Cutover, cancel and replay.** Attempt early settlement and settlement after cancellation. Both fail. Settle a valid claim after the deadline, verify replacement of the generation and retry an old claim. It cannot authorize a second rotation. |
| Guardians | The directory crate has successor requests, signed guardian approvals, cancellation and Rule R. A guardian quorum authorizes a fresh successor without splitting or reconstructing the prior key. Folding observes checkpoint finality, ordinary competing continuations and attested first-seen evidence. The CLI exposes the flow. | **Threshold and competing head.** Configure threshold two of three. A duplicate approval counts once. Add the second distinct approval. Compare verifier and CLI progress before and after the configured delay, under cancellation and under a valid ordinary continuation of the same head. Repeat on two receiving replicas. |

Evidence for the table: [Keyset][keyset] · [Encrypted backup][backup] · [Recovery-code decision][codes] · [Guardian verification][guardians] · [Recovery CLI][recoverycli]. NIST also treats codes and recovery contacts as distinct methods. [NIST SP 800-63B](https://pages.nist.gov/800-63-4/sp800-63b.html#account-recovery)

**Migration and ownership.** A preexisting identity may have zero enrolled recovery paths. Its account page should show the authenticated methods that genuinely exist. Neither successful OIDC login nor a synchronized journal proves a recoverable signing key. Offer optional enrollment and verify the committed enrollment before advertising recoverability. Variant a grants the service signing power. Variant b keeps decryption client-side. Variant c enables controlled successor rotation. Preserve any existing custody choice and record when the choice is unknown. The census marked #5684 assigned. Route findings to its current owner. The code count, entropy and threshold were already decided. [Service-held proposal][custody-a] · [Client-encrypted proposal][custody-b] · [Code decision][codes]

## Permission UX and revocation

#5960 separates pre-connect disclosures for a connecting user's device from those for a world operator. A user may grant or deny their own providers and remember decisions at an explicit scope. The server's grants are disclosed, with authority retained by its operator. #5999 compares user-meaningful capability classes and widening of scope. #6017 distinguishes requested, certified discovered-used, effective-grant and observed activity. With no valid certificate, discovered usage is unknown. [Permission UX][permissions] · [Change semantics][capdiff] · [Observability][observe]

**Proposed product flow.** A world requests file reading within one local collection. Display world/release identity, purpose, provider, operation, collection and duration. A user can approve once, remember a scope or deny. A v2 write request expands the existing read decision. Display just that delta, retain existing denials and show which party controls each decision. The actual provider rechecks its grant at invocation and enforces resource bounds. On Android, both the ID grant and the OS permission must allow the operation. For old persisted decisions, translate only demonstrably covered scopes. Affected calls wait for a fresh choice when an old scope is ambiguous. [Permissions][permissions] · [Android][android] · [Linux provider][providers]

**Oracle — broadened request and revocation.** Remember read-only access for world A and collection R; reconnect unchanged. Introduce a write request, exercise it before approval and verify R is unchanged. World B cannot acquire A's remembered approval. Revoke R after disclosure and before invocation; expect a stable refusal and no operation. An operator grant never manufactures a user's device grant. Test through terminal, GUI, Android and headless replay.

## Namespaces and schemas

The registry record permits immutable content addressing and mutable array addressing. Derived namespaces select one of the two cores plus a handler, encoding, read/write policy, shorthand policy and schema version. The URI record scopes names to an authority and forbids userinfo. Mutation first asks an injected authorizer, checks expected generation, validates the change, advances generation and appends an audit entry. Inspect deployment wiring separately from the existence of the injected interface. [Record][nsdef] · [Registry state][nsreg] · [URI decision][uri]

**Proposed resolution contract.** Carry authority, namespace identity, handler/schema version and registry context in resolution caches. A mutable path can be re-resolved after a registry generation changes, while its prior signed content reference remains historical evidence. Preserve an unknown handler reference and its original bytes in offline documents. Surface an unresolved resource with enough information to retry after an upgrade. A process unable to enforce the registered write policy cannot accept writes through its local adapter.

**Oracle — two registry writers.** Two authorized writers target generation g. Exactly one succeeds, reaching g+1 with one mutation audit entry. The other rereads/rebases. An unauthorized request with either generation gets the same denial. Two authorities can assign the same label differently without polluting one another’s caches. Include legacy-reference round trips, an unknown handler, forbidden userinfo and undeclared query parameters. [Registry test][nstest] · [URI parser][parser]

## App distribution, migration and rollback

The signed program bundle binds author identity, fork lineage, manifest and payload and can be verified and sideloaded offline. Duplicate manifest JSON keys are rejected. The gallery applies operator policy; an installation never needs a central catalog. The upgrade code copies current state for a migration probe, encodes and reads it back, and commits only after exact round-trip verification. It reads legacy v1 snapshots and writes v2. A failed or oversize migration leaves the former program and snapshot intact. The snapshot checksum detects corruption; bundle signatures establish provenance. [Bundle][bundle] · [Gallery admission][gallery] · [Gallery design][ecosystem] · [Upgrade][upgrade]

**Proposed normal entrypoint.** Verify the bundle and ABI, resolve publisher trust, compare effective permissions, and show release identity and any widened grant. Prepare a candidate snapshot in isolation and run the existing migration/restore probe and regression fixture. Activate the module and checkpoint at one journal position. Treat a new content hash at an existing display version as a new artifact. Recovery after a successfully activated release has a different requirement from abandoning a failed probe. A post-commit downgrade needs a compatible reverse migration or a retained checkpoint with an explicit branch preserving subsequent edits. A blind pointer change could strand new data. An optional automatic channel needs authenticated release order and a stale-metadata policy. Offline sideloading remains independently usable. [Bundle][bundle] · [Upgrade][upgrade] · [TUF update specification](https://theupdateframework.github.io/specification/latest/)

**Oracle — transactional version change.** Start release A using a real legacy snapshot. Try invalid signature, denied scope increase, unsupported ABI, damaged state and a failing migration. After each, A is usable and its active pointer and data agree. A successful B upgrade records one activation even after delivery retry. Inject a restart on either side of activation. A different publisher cannot silently inherit trust. Preserve visible history when B has accepted input after migration and the operator asks to return to A.

## Developer tooling

The local authoring loop compiles an app directory, runs a disposable world, gates promotion on deterministic regression fixtures, watches source changes and supports a bounded seeded invariant campaign. Doctor checks inputs including toolchain, platform build, grant ceilings, execution profile, journal format and clock. Extend these normal entrypoints across SDKs. [Author guide][devguide] · [Author loop][dev] · [Doctor][doctor]

**Proposed cross-SDK fixture.** Include guest ABI version, seed, admitted input trace, expected named records/checkpoints, required capabilities, any expected refusals and the execution profile. A discrepancy report identifies the first event after which expected and actual records differ. Hand-computed app oracles constrain all guest languages independently. Keep expensive fuzz cases separately dispatchable. The named local authoring command describes the product's normal behavior. This program's build and test verification is still dispatched through Fleet. #12596, the third-language SDK, was in review in the census. Reuse its fixtures and check the active owner.

**Oracle — a failed edit preserves a working world.** Introduce a regression that credits two units in Colony’s one-ore fixture. The gate returns the earliest mismatching record and the previous working world remains usable. Repair the reducer and replay the same fixture. With a broken toolchain input, doctor reports the relevant dependency and leaves live worlds and grants untouched. Record app and toolchain hashes, Fleet run ID, route and the observed outcome separately.

## Observability and support

A participant sees permitted operations and their release-specific changes. An author sees differences between declared and certified use for releases they own. An operator sees bounded effective/observed facts for authorized instances. The current owner report streams the journal, tracks missing timestamps in older records, distinguishes partial windows and separates live-only evidence from history. [Observability design][observe] · [Owner report][report]

**Proposed diagnostic envelope.** Record operation category, capability class, app version, outcome, evidence source and time interval. Separate attempts, admitted operations, fuel and transmitted/decoded bytes. Keep resource names, raw payloads, recovery information, bearer grants and hashes that expose private relationships out of general telemetry. Restrict detailed audit access and retention. Make the diagnostic export field allowlist explicit. Do not rely on an external telemetry service to read an owner’s own authorized audit. [Observability][observe] · [OpenTelemetry data handling](https://opentelemetry.io/docs/security/handling-sensitive-data/)

**Oracle — missing data stays visible.** Inspect a window containing joins before and after timestamp recording. Display known counts and the incomplete time window, preserving missing times. With no live sample, show live state unavailable; with no certification, show unknown discovered usage. Submit an operation twice. Attempts are two, while completed logical operations remain one where the operation has an idempotency key. Revoke access before export; the actual export decision must enforce current visibility.

## Media pipeline and permissions

The store's media.put reference carries BLAKE3 hash, size, media type and dimensions or duration. The blob gate mediates retrieval. A verified original is limited to 256 MiB by this implementation. An unreferenced local blob is eligible for collection after 30 days. Separate resources can reference the same content hash. Attachment design #5844 treats previews, OCR derivatives and annotations as independently authorized objects and prevents a derivative from widening the source's access. OCR defaults to local or user-controlled infrastructure. An export that lacks attachments must disclose that loss. #9084 concerns hosted editions and is not the local ID media implementation's dependency. [Media reference][media] · [Blob gate][blob] · [Attachments][attach]

**Proposed decode flow.** Resolve authorized metadata, independently authorize the requested representation, then negotiate byte, decoded dimension, CPU and time ceilings. Fetch and verify bytes under the grant; decode within the host boundary. Index a derivative by source content hash, transform identity/version and parameters. A previously cached thumbnail can disclose a private original and must obey the applicable grant. On revocation its presence in a local cache does not authorize re-serving it through ID. Preserve application-visible states for reference only, partial fetch, verified original, verified derivative and refused. The page can remain useful offline even when the binary is absent.

**Oracle — shared bytes, independent rights.** Two resources refer to identical bytes. A participant with no applicable grant cannot use the hash to retrieve either resource’s preview. Removing one reference preserves authorized retrieval through the other. Truncation, hash failure or wrong size never publishes a verified original. A small file demanding excessive decoded memory is refused before large allocation. A second transform version gets a distinct cache entry. A missing offline original preserves the reference and annotations. Export either contains the requested original and derivatives or enumerates omissions. A client cannot forcibly erase a copy another party already exported. [Media reference][media] · [Attachments][attach]

## Delivery and decision ownership

| Existing owner / issue | Next check on the normal entrypoint | Decision boundary |
|---|---|---|
| #5684, #2380–#2382 | Map keyset, backup and guardian flows to actual CLI/mobile account status and test the enrolled methods. | #5684 was marked assigned in the census. Preserve the selected custody variant and any recorded implementation-timing hold. |
| #5955, #5960, #5999, #6017 | Share grant change, who-decides and visibility fixtures across shells. | Recheck the current approval posture. Certification and observed use are independent. |
| #9107, #9116, #9117, #9122 | Test production registry-authorizer integration, authority-separated caches and old unknown handlers. | A numeric registry remains a derived namespace; migration belongs to its declared handler. |
| #6278, #8280, #1066, #5541 | Verify signed install, failed migration, crash/retry, and user-visible authoring regressions. | If no recorded policy settles post-commit downgrade, retain the compatible checkpoint and branch the later history. The alternative is a verified reverse migration for versions that supply one. |
| #2342, #5844 | Exercise preview authorization, cache invalidation, bounded decode, sharing and export. | Existing product decisions govern any hosted OCR and disclosure; local fixtures can proceed without resolving hosted placement. |

The label census and source archive were read independently. Latest issue-comment requests from this lane returned HTTP 403. Source evidence does not establish current claim ownership. No issue mutation, release action or test result follows from this design chapter.

[setup]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T23-17-06Z_design_id_setup_ceremony/2026-11-08T23-17-06Z_design_id_setup_ceremony.md
[android]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T22-06-00Z_design_id_android_client/2026-11-08T22-06-00Z_design_id_android_client.md
[custody-a]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13_design_id5886.md
[custody-b]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-10_design_id5887.md
[codes]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T10-03-00Z_design_id_recovery_codes/2026-11-08T10-03-00Z_design_id_recovery_codes.md
[keyset]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-directory/src/keyset.rs
[backup]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/src/backup.rs
[guardians]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-directory/src/recovery.rs
[recoverycli]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-cli/src/commands/keyset_recovery.rs
[permissions]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T09-06-08Z_design_id_permission_ux/2026-11-08T09-06-08Z_design_id_permission_ux.md
[capdiff]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/2026-11-08T10-12-00Z_design_id_capability_diff_semantics.md
[observe]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T10-40-00Z_design_id_capability_observability/2026-11-08T10-40-00Z_design_id_capability_observability.md
[providers]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-native-shell/src/providers.rs
[nsdef]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-core/src/namespace_definition.rs
[nsreg]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-store/src/namespace_registry.rs
[uri]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17T04-55-00Z_design_id9117_id_uri_scheme.md
[parser]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-core/src/id_uri_parse.rs
[nstest]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-store/tests/namespace_registry_model.rs
[bundle]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/program_bundle.rs
[gallery]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/program_gallery_admission.rs
[ecosystem]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3.md
[upgrade]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/world/upgrade.rs
[devguide]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16_id_world_dev_authoring.md
[dev]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/world_dev.rs
[doctor]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/world_doctor.rs
[report]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/world_owner_report.rs
[media]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-store/src/media.rs
[blob]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-worlds/src/blob_gate.rs
[attach]: https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/knowledge-layer-attachments-media-5844.md
