# Platform completion: acceptance brief

Date received: 2026-10-14
Starting commit: `292d3c7a` (after Phase 8 PR #206)

## Owner's request (verbatim)

> what's left? please continue to fully implement the plan. when you are done, ensure that there is a method to upload and run roc programs dynamically, access the document editing and viewing collaborative view with prose mirror, access a world which allows generic live chat and document view for either web or cli/ssh, and a generic home page with a world explorer, global chat, and live feed of recent and popular documents and tags. continue until all of this is done, store this message and refer back to it don't stop until you can review and affirm each individual part works right now. don't stop to ask me questions continue to implement and if you have questions or made high touch decisions maintain a list in a document for me to address later but don't stop. choose the better options with more features, higher performance, better adaptability to varied future plans.

## Acceptance checklist

- [x] Remaining work in the authoritative plan is inventoried; completed, deferred, skipped, and open items are distinguished below. External deployments and owner-bound protocol choices remain explicitly open.
- [x] Users can upload a Roc program dynamically and run it in a world (browser source compile/install and `.wasm` upload; CLI was already available).
- [x] Users can collaboratively edit and view documents using ProseMirror. Live collaboration and save publication are tested; see the local-only replication gap below.
- [x] A world supports generic live chat and document viewing through web and CLI/SSH. SSH viewing is read-only and bounded; web editing uses the shared ProseMirror surface.
- [x] The home page provides a world explorer, global chat, and live feeds of recent and popular documents and tags, scoped to this server.
- [x] Each delivered acceptance feature has targeted tests and a documented current verification result; integrated Rust, browser unit, formatting, and focused E2E status is recorded below.
- [x] High-impact choices and unresolved questions are recorded here or in a linked decision log without blocking implementation.

## Progress

Initial inventory: complete. The roadmap's Phase 8 items P8.2–P8.6 have merged in PR #206; P8.5 was already completed as P2.7. P8.1 is explicitly deferred, P8.7 explicitly skipped, and P8.8 deferred. P7.2's Cloudflare Workers pkarr design is now documented and merged into this work branch (design only; deployment deferred). P7.5 and P3.7 real deployments/network tests remain blocked on separate networks/accounts. Additional implementation gaps from the handoff are freshness/pull policy wiring, the group-join CLI path and unsigned local group-log stand-ins, and several unresolved protocol questions (#194, #185, #160, #183, #195); these are tracked as follow-up scope rather than being represented as finished.

### Existing feature verification map and current acceptance state

- **Roc runtime:** `world_compile.rs`, `world_session.rs`, `commands/world.rs`, `world_net.rs`, and `web/world_ws.rs`. Browser source compilation/install/run and CLI source compilation/install/run both passed targeted current tests; Wasm upload/run and invalid-admin-auth coverage also passed. Bounded compilation and the import-free Wasm sandbox remain in effect.
- **ProseMirror:** `web/src/editor.ts`, `collab.ts`, and Rust `web/collab.rs` provide collaborative editor sessions. Successful saves/autosaves now store the blob and publish signed local `document.put` revisions. Targeted route, `local_events`, and `documents` tests passed. **Known gap:** the sibling `LocalEvents` store is not replicated to other peers; collaboration is live in the hosted web session, but event-backed revision discovery is currently local-only.
- **World chat and documents:** authenticated web world chat/records remain available. SSH sessions now support line-buffered chat, `/input`, paged `/records`, `/documents`, and bounded, read-only UTF-8 text/Markdown `/document` viewing. The web world links to the shared file browser/ProseMirror view; editing remains in that editor, not SSH.
- **Home/discovery:** `/` now serves a server-scoped world explorer, default-world global chat, and separate recent/popular document and tag feeds. `/files` retains the file browser. Feed candidates are locally available, globally tagged, non-deleted documents; tag popularity counts locally available documents carrying each exact global tag, without view tracking.

### High-impact choices / decision log

- Prefer the existing bounded Roc sandbox and signed/event-backed document model; do not introduce arbitrary host access or a second document authority.
- A server-global chat/feed can be a local world backed by the configured default world until cross-server replication is explicitly integrated; the UI must label its scope honestly.
- World documents should reference immutable event-backed document revisions; collaborative edits should checkpoint/publish signed document revisions without treating transient ProseMirror steps as authoritative events.
- Discovery popularity should be derived from explicit, privacy-preserving event-backed signals (views/reactions) or clearly labelled local counts; no hidden tracking.
- Deferred by the authoritative plan unless separately justified: time/witness validity, BeeKEM (owner said skip), and broad lint cleanup. P7.2/P7.5 gaps will be documented if their external account/network prerequisites cannot be exercised here.

#### Follow-up decisions for owner (implementation will not wait)

1. **Meaning of “global chat.”** Default implementation will use the server's configured default world/lobby, sharing the existing capability-authenticated world chat stream. It is global across that server's lobby, not automatically federated across servers. Federation needs world identity/trust, moderation, and replication rules.
2. **Popularity and privacy.** Prefer an explicit “most tagged” ranking until there is a consentful event-backed popularity signal. Do not add per-user tracking. If actual popularity is later desired, a public, aggregate reaction/view event is the recommended seam.
3. **Roc upload security.** Browser source compilation must use the existing admin-token/world protocol and bounded compiler; program execution remains the existing import-free Wasm sandbox. Do not silently enable native worker execution in the browser path.
4. **Document durability.** ProseMirror steps remain transient live collaboration state; save/autosave emits signed immutable revisions. Whether every transient edit must be durable is not assumed (high write volume and large event histories make that a distinct trade-off).
5. **World document model.** Current web access links to the shared file browser/editor; SSH exposes only public local text/Markdown blobs with strict bounds. Neither surface treats arbitrary Roc JSON as a signed document. A replicated records-to-document reference/index is still future work; if added, publish an explicit immutable revision reference rather than interpreting arbitrary program state as document authority.

### Implementation workstreams

- [x] Dynamic Roc program authoring/upload/run in web and CLI. **Verified:** built `examples/roc-world/targets/wasm32/host.wasm`; `cargo test --features web browser_websocket_can_compile_install_and_run_roc_sources -- --nocapture` passed, including compiling Roc source through the browser transport, installing the module, and running its world update; `cargo test --test cli_integration test_worlds_compile_roc_sources_on_the_fly -- --nocapture` passed. The invalid-admin test and existing browser Wasm upload/run test passed in the targeted browser run. `bun run typecheck` and `bun run test` passed (343 passed, 9 skipped). Execution continues to use the bounded compiler and import-free Wasm sandbox; native execution is not enabled by this browser flow.
- [ ] Join the ProseMirror collaboration persistence path to signed document revisions and blob storage **and replicate those revisions**. Implemented locally: every successful manual/autosave stores the blob and publishes a signed `document.put` + `filename` claim through `LocalEvents`, returns the event ID, and fails closed on publication errors. **Not complete:** the local event store is not synced to other peers, so another node cannot yet discover those signed revisions through this path. Validation passed: 2 web save route tests, 2 `local_events` tests, 5 `tests/documents.rs` tests, TypeScript typecheck, Vitest (343 passed, 9 skipped), and the integrated Rust suite below.
- [x] Add generic document/chat world interface with web and terminal interaction. Web world chat/records exist; SSH provides line-chat/records and bounded read-only text-document viewing. ProseMirror editing remains in the linked web document surface, not SSH.
- [x] Replace the file-list root with an adaptable home page: world explorer, server-scoped global chat, and recent/popular documents and tags. `/files` preserves the file browser.
- [x] Run targeted Rust, web unit/typecheck, and browser end-to-end checks; update this checklist with actual results and blockers.

### Current phase-brief tests

- Browser Roc source path: pass with the local Roc compiler and generated world host artifact. The generated artifact is build output and is not committed.
- ProseMirror editor/collab: existing unit and browser tests are present; durable signed local save checkpoints are implemented and targeted tests pass. Peer replication remains open.
- Integration suite: `cargo test --features web,ssh --lib` passed (**1,502 passed, 4 ignored**), including Roc source compile/install/run, home, save publication, world records, and SSH document/chat tests.
- Home page: 5 Rust home tests passed; TypeScript typecheck passed; Vitest passed (**345 passed, 9 skipped**); focused Chromium home/navigation passed (**11 passed**), including root/home, `/files`, primary navigation, and editor navigation.
- SSH world shell: 17 focused `world_ssh` tests passed, including chat, records, `/input`, and document list/view bounds; targeted `rustfmt --check` and `git diff --check` passed.
- Formatting: `cargo fmt --all -- --check` and `biome check src/home.test.ts` pass after aligning the existing `blob_gate.rs` constructor with the configured Rust formatter and formatting the new home test.

### Review pull requests

- [#209 — Browser Roc source compile/install/run](https://github.com/developing-today/code/pull/209)
- [#210 — Signed document revisions on web save](https://github.com/developing-today/code/pull/210) (peer replication is not included)
- [#211 — SSH world chat and document viewing](https://github.com/developing-today/code/pull/211)
- [#212 — Server-scoped home discovery](https://github.com/developing-today/code/pull/212)
- [#213 — P7.2 Workers pkarr design](https://github.com/developing-today/code/pull/213) (design-only)

### Broader unfinished roadmap items to close or explicitly defer

- [x] P7.2: write the Cloudflare Workers pkarr design using the already-collected round-2 research. See [`doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md`](../2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md). Design is complete; deployment and implementation remain out of scope for P7.2.
- [ ] P7.5/P3.7: real two-network and account-backed deployment matrix cannot be truthfully verified without the required networks/accounts. Keep the code-side test harness ready and leave the live matrix explicitly blocked, not falsely passed.
- [ ] Wire the existing freshness-window and pull-policy configuration into server/world settings and CLI defaults (handoff gap).
- [ ] Finish or explicitly scope the `group join` CLI path; replace local unsigned group-log stand-ins if the phase contract requires portable signed group events.
- [ ] Resolve/document the remaining protocol questions tracked as #194, #185, #160, #183, and #195 before claiming the complete original plan.
