# Design review of the 41 open design-labeled ID issues

*Research snapshot of November 23, 2026. Code and issue observations refer to the cited revisions and collection times.*

The [Route each open issue to its next proof](roadmap-routing.md) carry forward each of these 41 assessments with its prior action, blocker and proposed proof.

On November 23, 2026, the REST census contained 219 distinct open issues in the union of `app/id`, `epic/id-program` and `epic/id-platform`. Filtering that union by the exact label `kind/design` produced the 41 issue numbers below. Each appears once. The archive evidence is pinned to [4cfceaa](https://github.com/developing-today/code/tree/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a). The issue links supply the owning scope. The evidence links distinguish a dedicated record, a contributing child or adjacent design, and an implemented contract. A missing archive match means the indexed Markdown did not establish that decision. An issue comment or another repository may still contain it.

Each proposed next action is conditional on the current issue, latest claim, approvals and review status. A filed or merged design can coexist with an open implementation or acceptance item. This review did not close or claim issues. Source links describe the inspected code and decisions, without claiming deployed behavior or a test execution.

## Backends, data contracts and operational controls

### [#9082](https://github.com/developing-today/code/issues/9082) · store abstraction — Turso/SQLite-over-HTTP beside redb, IndexedDB evaluated (one interface, pluggable backends)

**Parent decision due.** The summit owns the shared backend fixture. Closed fixture slices left IndexedDB in the reserved class. The parent still has to adopt, defer or reject that slot. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md) · [Inspected discussion](https://github.com/developing-today/code/issues/9082#issuecomment-7099653272)

**Next action.** Record a reasoned disposition against IndexedDB transaction, ordering, durability and quota semantics. Deferral leaves the slot reserved and names the next proof, adoption requires a passing adapter fixture.

### [#9110](https://github.com/developing-today/code/issues/9110) · extensibility architecture — primitive/adapter/hook boundary spec + backend conformance

**Parent awaits backend verdict.** The crosscut record assigns port contracts, hook ordering, recovery and conformance. #9082 owns the remaining browser-profile disposition. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md) · [Inspected discussion](https://github.com/developing-today/code/issues/9110#issuecomment-6938814628)

**Next action.** Resolve #9082 first. Compare every claimed backend and public entry point against the same operation and hook-order fixtures before changing the parent status.

### [#9111](https://github.com/developing-today/code/issues/9111) · modes & configuration surface — modes, link kinds, storages, tiering as config

**Crosscut specified.** The crosscut defines configuration precedence and allowed modes and preserves mandatory validation. Part B contributes federation knobs. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md)

**Next action.** Round-trip two profiles with different stores, transports and tiers through the same primitive composition. Reject invalid combinations at configuration load without weakening authorization.

### [#5681](https://github.com/developing-today/code/issues/5681) · serverless edition storage — D1 vs Turso vs both for `id` on Cloudflare Workers

**Recorded decision.** Drew already chose both engines with roles, with Turso preferred and D1 first-class. The signed journal owns logical recovery. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-10_decision_id5681_serverless_storage.md)

**Next action.** Keep the decision. Reconcile deployed hosted capabilities and list only still-owed adapter proofs from #5864 and guardrail work before parent closure.

### [#5864](https://github.com/developing-today/code/issues/5864) · D1 journal fit (design + spike) (from #5681)

**Fit decided, measurements owned.** The direct record reports the local spike in #6350 and names three outstanding adapter measurements: two-writer fork refusal, bookmark lag, and deployed byte ceilings. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17_decision_id5864_d1_journal_fit.md)

**Next action.** Link each owed proof to #5863 adapter conformance or #5868 guardrails. Preserve the parent choice of both backends and item-specific authorization for production measurements.

### [#5805](https://github.com/developing-today/code/issues/5805) · Postgres-over-HTTP adapter (from #5672)

**Issue decision evidence incomplete.** The neighboring selective-sharing adapter design describes the parent context. No direct #5805 decision record was established in the inspected archive mapping. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08_design_id5803.md)

**Next action.** Check the issue's discussion for a decision. Otherwise specify the translated query subset, typed refusal for unsupported joins, and a permissions-filtered fixture against reference evaluation.

### [#5685](https://github.com/developing-today/code/issues/5685) · DoS suite & performance benchmark programme — adversarial CI coverage and per-interface regression gates

**Child design set.** The parent has a scenario catalogue, workload generator, standard baseline and CI tiering records. Existing design gives each measurement a reproducible role. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16T09-52-00Z_design_ci_tiering_adversarial_benchmark_suites/2026-11-16T09-52-00Z_design_ci_tiering_adversarial_benchmark_suites.md)

**Next action.** Inventory which bounded regression fixtures have an ordinary entry point, baseline and quota provenance. Report gaps by interface without dispatching a large load run.

### [#2819](https://github.com/developing-today/code/issues/2819) · native-spill write lease protocol for hot worlds (from #2059)

**Proposal and implementation evidence.** The direct design locates append authority in the hosted journal and lends compute to the native node under a fenced lease. A later actor-boundary record exists. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-01_design_id2819.md)

**Next action.** Trace the actual commit guard across lease transfer. Exercise delayed old-epoch completion, restart and new-holder acceptance against one accepted journal head.

### [#4766](https://github.com/developing-today/code/issues/4766) · one name, one owner — a file's name is canonical, its archive namespace is reserved, and its archives belong to exactly one file (pkgs/id FileOps)

**Issue and code evidence.** No archive Markdown with an explicit #4766 reference was identified. The body cites an external design. The current FileOps implementation now documents canonical names, reserved archive namespace and archive ownership. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/pkgs/id/crates/id-sync/src/fileops.rs)

**Next action.** Audit the phase-4 list against current code, especially legacy lookalikes, copies and both archive collection paths. Acknowledge implemented clauses before filing any residual.

### [#11419](https://github.com/developing-today/code/issues/11419) · related path links in hash.metadata — typed RelatedLink, PathRef validation, Dangling resolution

**Implementation with upstream prerequisite.** The issue directly names the #9114 path-links decision from PR #11418 despite zero explicit #11419 hits in the archive. The body links #10242. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17T22-00-00Z_design_id9114_related_path_links.md)

**Next action.** Check #10242 and live ownership. Reuse the sidecar snapshot API, canonical link sorting, normalized relative paths and explicit dangling targets. Match order-independent snapshot IDs across fixtures.

### [#6487](https://github.com/developing-today/code/issues/6487) · sign only what receipt would accept — one sign-time refusal contract for pkgs/id

**Closing-audit candidate.** The direct record reports seven signer filings landed at its November 17 audit and specifies a sign/receive refusal contract. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17_design_id_sign_time_refusal.md)

**Next action.** Verify current public signing entry points share each corresponding receipt invariant. Demonstrate a previously invalid creation fails before signing; preserve valid legacy vectors.

### [#2898](https://github.com/developing-today/code/issues/2898) · reconstruction is admission — validate authority-bearing inputs where they are rebuilt (pkgs/id)

**User review gate.** The direct proposal requires authority-bearing state to be re-admitted at reconstruction. The issue has a user/review label. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-01_design_id2898.md)

**Next action.** Keep the gate. Assemble the existing six origin decisions and their refusal or exemption evidence for review. An invalid replay must leave prior state intact in each affected path.

## Addressing, graphs and federation

### [#9113](https://github.com/developing-today/code/issues/9113) · ID-shape registry + dispatch; content-addressed hash no-map rule

**Parked.** Part A fixes content and array address boundaries and defines shape dispatch; raw content identity requires no map. The issue has explicit parked status. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md)

**Next action.** Preserve parking. When reopened, verify profile-aware parsing, known and unknown shape behavior, and a raw content lookup with no mapping service before expanding adapters.

### [#9119](https://github.com/developing-today/code/issues/9119) · bare-string shorthand + default-index configuration; streaming interfaces + max-items

**Contract specified.** Part A defines scoped bare-string shorthand and bounded streaming beside explicit addressing. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md)

**Next action.** Pin two conflicting default indices and an overwritten default mid-page. Confirm explicit addressing bypasses shorthand and continuation respects the original index and max-items.

### [#9122](https://github.com/developing-today/code/issues/9122) · numeric registry namespaces over the byte system

**Contract specified.** The numeric namespace distinguishes integers, decimals and binary floats, with per-kind canonicalization and stable total ordering. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md)

**Next action.** Compare a reference numeric sort with encoded key order for signed extremes and equivalent representations. Preserve type identity across serialization and lookup.

### [#9125](https://github.com/developing-today/code/issues/9125) · triple encoding bake-off — compressed binary vs JSONB vs normalized byte-key maps

**Claimed, preserve owner.** The real-engine bake-off exists and distinguishes its measurements from the serialization toy. Multiple candidate representations were evaluated and no universal winner was asserted. The snapshot records an active claim. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/real-encoding-bakeoff.md)

**Next action.** Leave implementation to its owner. Cite the actual workload/result file and note the chosen operational index separately from claims about other workloads.

### [#9126](https://github.com/developing-today/code/issues/9126) · graph query language selection + visualization

**Query core established.** Part A §5 defines bounded query and visualization; main supplies the typed query. #7612 separately requires application navigation order and visibility. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md)

**Next action.** Exercise a shared visible fixture through triple selection, the typed navigation view and keyboard/canvas inspectors. Evaluate alternative query syntaxes as frontends to the existing core.

### [#9129](https://github.com/developing-today/code/issues/9129) · versioning — succeeds triples + carried-forward comments

**Contract specified.** Part A §8 fixes predecessor-to-successor edges and carries forward comments through a view with original version attribution. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md)

**Next action.** Test a fork and a later merge with an unreadable ancestor. Check the signed comment target remains intact and the view never leaks the ancestor.

### [#9132](https://github.com/developing-today/code/issues/9132) · delegation + cross-signed identity ceremony

**In review.** The subject-specific decision covers delegation and cross-signing within existing identity semantics. The issue is in review. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-21T04-30-00Z_design_id_delegation_cross_signing_ceremony/2026-11-21T04-30-00Z_design_id_delegation_cross_signing_ceremony.md)

**Next action.** The reviewer should trace one delegated, revoked and cross-signed identity. A reciprocal signature proves only its declared binding; test authority at each consuming action.

### [#9142](https://github.com/developing-today/code/issues/9142) · implicit-trust sync — convergence toward 1:1 + conflict reports

**Implementation premise partly met.** Part B bounds implicit-trust convergence by scope and capacity. B07 conflict reports and the persistent federation record store already exist on pinned main. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md)

**Next action.** Check partition, conflicting updates and restart against a recorded frontier. Show unrelated scopes make progress and inspectable conflict evidence survives recovery.

### [#9144](https://github.com/developing-today/code/issues/9144) · federated query fan-out — prioritized remotes, trusted relay, max-hops

**Recorded design.** The subject-specific fanout record and Part B define local-first selection, prioritized remotes, bounded relay grants and signed provenance. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17T15-40-00Z_design_id_federated_query_fanout/2026-11-17T15-40-00Z_design_id_federated_query_fanout.md)

**Next action.** For a local hit, offline remote, and relayed result, compare authority, hop/fanout/byte budgets, partial completion and provenance on both hosted and native entry points.

### [#9145](https://github.com/developing-today/code/issues/9145) · relay duplicate + loop handling — inform, short-circuit, dedup, loop-inform

**Blocked, exact reason unverified.** Part B distinguishes duplicate, replay, cycle and changed binding. The issue carries queue/blocked and status/blocked, with the precise current gate unchecked in this pass. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md)

**Next action.** Read latest owner/blocker evidence before claiming. Once released, check in-flight duplicate coalescing, cached results, exhausted replay windows and distinct signed loop evidence.

### [#9147](https://github.com/developing-today/code/issues/9147) · reputation system + public claims/dispute log

**Contract specified.** B26 is a research record. The governing Part B record defines local evidence classes and redacted claims, replies and deterministic escalation. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/B26-research-reputation-abuse.md)

**Next action.** Use a disputed claim, signed reply and later correction to check decay, visibility and review. Recompute one owner's local outcome without changing another owner's policy.

### [#9148](https://github.com/developing-today/code/issues/9148) · takeover resistance + acknowledgment-based inconsistency detection

**Research plus phased contract.** Part B assigns emergency quarantine to V2 and attribution to V4. Absence of an acknowledgment is insufficient where no signed completeness claim exists. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md)

**Next action.** Compare missing acknowledgment, contradictory signed evidence and a partial recipient list. Show exactly which case permits local quarantine and which leaves attribution unresolved.

### [#6062](https://github.com/developing-today/code/issues/6062) · ID-graph publication format — Atlas objects + provenance/lineage relations as ID-graph objects

**Format landed.** The archive is a direct format decision. The projector emits advised note, document, relation and batch attestations from canonical Atlas rows. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-09T07-20-00Z_design_atlas_id_publication/2026-11-09T07-20-00Z_design_atlas_id_publication.md)

**Next action.** Use #11182–#11186 as the implementation and audit units. Require full-unit atomic visibility, exact supersession and replay from pinned Atlas rows before asserting parity.

## Worlds, Roc and permission prompts

### [#5955](https://github.com/developing-today/code/issues/5955) · design program: worlds — advanced Roc libraries, capabilities, permissions, dynamic user functions, licensing (2026-11-04 brief)

**Program synthesis.** The cited record designs one child capability model. It cannot settle the entire worlds program of permissions, function loading, presentation and licensing. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T09-15-00Z_design_id_capability_model/2026-11-08T09-15-00Z_design_id_capability_model.md)

**Next action.** For each statement in the worlds brief attach its child decision, a running host or fixture, or an outstanding decision. Retain the ABI pin across the library and host chapters.

### [#5959](https://github.com/developing-today/code/issues/5959) · capability-change notices on reconnect — expansion and sideways changes

**In review.** The November 23 record specifies consent on reconnect for expansions and sideways changes. The issue is already in review. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-23T05-02-30Z_design_id_capability_change_notices/2026-11-23T05-02-30Z_design_id_capability_change_notices.md)

**Next action.** Check the reviewer’s last note. Verify the saved grant baseline, declined reconnect and explicit acceptance of the new version using the same diff in each client.

### [#5961](https://github.com/developing-today/code/issues/5961) · permission-prompt precedents — Play Store, Android/iOS runtime permissions, OAuth scopes, extension permissions, TCC/portals

**Research recorded.** The primary research survey covers platform consent patterns. The reconnect-notice and permission UX documents already consume its conclusions. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-12T20-58-00Z_research_id_permission_precedents/2026-11-12T20-58-00Z_research_id_permission_precedents.md)

**Next action.** Link each claimed design lesson to its consumer. Exercise a scoped expansion, a refusal and the ability to inspect prior consent in native and web flows.

### [#5964](https://github.com/developing-today/code/issues/5964) · dynamic user functions — server-compiled DLLs, hot-swap into running worlds without world recompile

**Recorded design.** The direct design selects server-built per-function wasm32 modules, initialized and switched at a tick boundary. A failed candidate leaves the published version running. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16T13-00-00Z_design_id5964_dynamic_user_functions/2026-11-16T13-00-00Z_design_id5964_dynamic_user_functions.md)

**Next action.** Map compile, interface, init and snapshot failures onto separate fixtures. Demonstrate an in-flight old call and a new-tick call retain the correct version and grant set.

### [#5968](https://github.com/developing-today/code/issues/5968) · tick & intent data model — inputs, state updates, per-user function views, intent sources

**Related decisions available.** The mapped record covers per-user function views. Sibling records cover server invariants and cooperative timeouts. None of these links alone establishes closure of the tick-and-intent parent. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T10-26-00Z_design_id_per_user_function_mapping/2026-11-08T10-26-00Z_design_id_per_user_function_mapping.md)

**Next action.** Write one ordered tick transcript for simultaneous inputs from human and bot, bounds, visibility filtering, and replay of the same state digest across host profiles.

### [#5970](https://github.com/developing-today/code/issues/5970) · user presentation assets — shaders, sprites by ID, filters, per-world gating, graceful degradation

**Child evidence available.** The shader record covers uploaded rendering assets. #6010 separately fixes semantic-cell obligations and references a #5970 umbrella decision. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T00-57-08Z_design_shader_asset_format/2026-11-08T00-57-08Z_design_shader_asset_format.md)

**Next action.** Recover the umbrella record from the issue before drafting. Test one missing or denied sprite, one rejected shader and a complete glyph fallback without changing simulation state.

### [#5971](https://github.com/developing-today/code/issues/5971) · image/video conversion tooling — kitty graphics protocol and video-to-ASCII

**Dedicated survey unverified.** No directly mapped archive decision for the image/video conversion survey. The issue explicitly separates Kitty graphics/media from the earlier keyboard protocol work.

**Next action.** Check issue comments for the research. If absent, compare pinned candidate tools on one still, one short motion fixture and an unsupported terminal; record license, cost, legibility and placement without adopting a codec prematurely.

### [#5979](https://github.com/developing-today/code/issues/5979) · SPDX identifiers and license-detection tooling for the classifier

**Related decision only.** The pipeline proposal treats scanner outputs as evidence. It references the research but does not by itself prove the full requested SPDX/tool comparison has been completed. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-05T17-24-09Z_design_id_license_detection_pipeline/2026-11-05T17-24-09Z_design_id_license_detection_pipeline.md)

**Next action.** Locate the research decision in discussion, then pin SPDX list/tool versions and compare declared, detected and unknown results over the same sample. Keep provenance and disagreement visible.

### [#6003](https://github.com/developing-today/code/issues/6003) · feature-object library attachment — runtime attach/detach of library behavior to entities without world recompilation

**Dedicated design unverified.** No direct archive mention was found. The prior-art child discusses feature objects. The issue specifically distinguishes attachable behavior from per-user functions. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T09-14-46Z_research_id5988_prior_art_bot_games_programmable_worlds/2026-11-08T09-14-46Z_research_id5988_prior_art_bot_games_programmable_worlds.md)

**Next action.** Check discussion first. Specify authority at attach, capability intersection, activation boundary, in-flight calls and detach-state fate. Decide whether a runtime mechanism is justified on those tests.

### [#6010](https://github.com/developing-today/code/issues/6010) · semantic cell/tag presentation model — one tagged state, many renderers

**Recorded design.** The direct decision extends the existing cell abstraction with semantically tagged projected cells and bindings per tier. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-17_design_id6010_semantic_cell_tag_model.md)

**Next action.** Compare identical semantic cells through glyph, sprite and native rendering. Verify keyboard-playable fallback and absence of owner-only identity in another viewer's projection.

## Application and identity programs

### [#5676](https://github.com/developing-today/code/issues/5676) · Currency scaling — autoscaling service, Redis → multi-Turso → TigerBeetle path

**Child design set.** The parent calls for double-entry invariants through scaled storage. Child design records cover migration and scaling choices. Backend promotion requires measured evidence. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16_design_id5826.md)

**Next action.** Compare identical receipt/ledger effects across adapters and replay through a migration cutover. Preserve existing currency code, measure capacity and operational cost before promotion.

### [#5677](https://github.com/developing-today/code/issues/5677) · ID notes, message board, and multiplayer editing v1

**Child design set.** The edit/fork design is one child, alongside approval and search-ranking contracts. Main already has visible note link and backlink projections. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-09T03-57-04Z_design_id_edit_fork_graph/2026-11-09T03-57-04Z_design_id_edit_fork_graph.md)

**Next action.** Trace a note posted to a board, forked, discussed and viewed through a restricted account. Use #7612 for navigation parity and the existing approval contract for edits.

### [#5679](https://github.com/developing-today/code/issues/5679) · Markup & styling model for programs-as-websites — semantic markup, style-override mapping

**Child decisions available.** The child record establishes the vocabulary. The sibling records cover overrides, native controls and target composition. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md)

**Next action.** Run one form with keyboard focus, validation and accessible labels through the terminal, native and web semantic mappings. List any unavailable action with its defined equivalent.

### [#5683](https://github.com/developing-today/code/issues/5683) · residency & boundaries framework — opt-in evidence, sanctions, and intranet-mode capabilities

**Recorded framework.** The parent record and evidence/appeals children define opt-in boundary policies. Signup/login execution is only one enforcement point. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08_design_id5683.md)

**Next action.** Exercise enabled and disabled policy, conflicting evidence, appeal and retention across local and hosted paths. Attach evidence from the same policy version and disclose only permitted detail.

### [#5684](https://github.com/developing-today/code/issues/5684) · identity, login & delegation — simplicity-first key custody, security-code recovery, and pseudo-login identities

**Assigned, preserve owner.** The recovery-code child specifies one part of a wider login and delegation program. Setup ceremony and pseudo-identity decisions must also agree. The snapshot records work/assigned. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-08T10-03-00Z_design_id_recovery_codes/2026-11-08T10-03-00Z_design_id_recovery_codes.md)

**Next action.** The current owner should inspect the end-to-end lost-device, recovery, delegation and pseudo-login paths. Record separately which keys or accounts each flow can recover.

### [#5753](https://github.com/developing-today/code/issues/5753) · fork a Rust KDL stack for ID — universal + multiple annotations, upstream archaeology, fork-base selection (extends #5688)

**Partial decision.** The direct §5 record selects the kdl-rs v6.7.1 fork base, resolves questions 1–3 and records recommendations for 4–5. [Pinned evidence](https://github.com/developing-today/code/blob/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16_decision_id5753_kdl_fork_section5.md)

**Next action.** Keep upstream-compatible syntax vectors beside extension vectors for structured multiple annotations. Separate the remaining recommendations from adopted decisions before implementation.

## Coverage check

The report generator compared the distinct issue numbers in these sections against the label-filtered census. Both sets contain 41 numbers, with zero missing, extra or duplicate entries. The archive-reference index has zero explicit issue-number matches for #4766, #5971, #6003, #11419. Direct issue-to-design links and source-code checks supplied additional evidence where available. Markdown mentions alone were never treated as issue closure, claim clearance or implementation.

## Two archive gaps developed into proposals

### Terminal still images and motion for #5971

Chafa documents Kitty, sixel, iTerm and symbol output, including multiplexer handling. Its development page identifies the library license as LGPLv3 or later. Kitty specifies capability queries for frame loading, animation control and composition, so static images and animation require distinct entries in the host capability profile. mpv documents its true-color terminal output and Kitty output as alternative playback baselines. FFmpeg's license depends on the selected libraries and build configuration. Record the exact artifact and configuration before considering redistribution. The particular “IDF video-to-ASCII” tool mentioned in the issue remains unidentified. [Chafa formats](https://hpjansson.org/chafa/man/) · [Chafa license](https://hpjansson.org/chafa/development/) · [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) · [mpv output drivers](https://mpv.io/manual/stable/) · [FFmpeg license inventory](https://ffmpeg.org/doxygen/trunk/md_LICENSE.html)

| Host and source | Proposed conversion and placement | Acceptance fixture |
| --- | --- | --- |
| Still image, pixel-capable terminal | Decode with a bounded host adapter and send a resized image after negotiating protocol and transport. Cache an authorized derivative keyed by source, resize parameters and converter version. | Render a known palette image locally and through an SSH-like byte stream. Verify dimensions and colors within declared conversion tolerance. |
| Motion-capable terminal | Decode bounded frames on the media-capable host. Cap frame rate, displayed size and buffered frames. Where supported, encode explicit frame durations and defer playback timing to the terminal. | Display frames A/B/C at uneven durations. Count decoded bytes and check progression, cancellation and release of the terminal image. |
| Symbol-only or monochrome terminal | Derive a symbol frame within the viewport. Reduce frame rate before losing essential information. Provide a still description and explicit playback control when movement cannot be shown accessibly. | Compare the meaningful sequence and controls against the pixel host; verify output respects declared ASCII-only and size limits. |

Upload-time generation is appropriate for authorized, reusable thumbnails and low-resolution previews. Client-side adaptation handles terminal size, glyph/color capability and current privacy policy. An unknown, denied or oversized asset returns its semantic fallback. Preserve distinct rights to read the original and preview. No converter was installed, benchmarked or selected as a dependency.

### Runtime behavior attachment for #6003

The LambdaMOO user guide documents feature objects that add commands and can be added or removed by a player. This substantiates the command-discovery precedent. The issue's linked getting-started page did not expose those commands in the reviewed text. [LambdaMOO guide, feature objects](https://brn227.brown.wmich.edu/Barn/files/docs/ygm.pdf)

**Proposed ID contract.** A binding records entity ID, signed library revision, namespaced exported verbs, compatible state schema and owner-authorized requirement set. At attach, resolve the existing library closure and compare requested capabilities with the entity, world, operator and actor grants. If a required capability is missing, refuse before publishing the binding. Optional capabilities become explicit unsupported results. A successful operation activates at a deterministic tick boundary and contributes to the world's event history. Attach and detach retries share their original request ID. Namespace collisions produce an explicit choice rather than silently overriding a verb.

A detach removes the binding from future dispatch. Existing pure invocations may finish against their pinned revision. A host effect rechecks live authority before committing. Persist per-binding state as a reference with a declared retention rule; reattachment or upgrade may import it only through a compatible schema or explicit migration. A failed migration preserves the preceding binding and state. Developer tooling should expose the provider of each verb, version, requested permissions and detach effects. Model one successful attach, consent refusal, two libraries exporting the same verb, a detach during an effect, and failed upgrade. These are proposed extensions to #5964's wasm32 publication model. Their adoption still depends on reconciling the issue's latest discussion and demonstrating a consumer. [Pinned #5964 design](https://github.com/developing-today/code/tree/4cfceaa8afc08cf2cea88b9d733a2ace14c8ad5a/doc/2026-11-16T13-00-00Z_design_id5964_dynamic_user_functions)
