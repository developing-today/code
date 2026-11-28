# Phase 12 designs through the research workflow (#534)

Shared package-opening intake for the 11 Phase 12 D1 items on umbrella issue #534, per the merged workflow design doc/2026-11-01_design_id534.md (PR #3912): one concept per D1 plus the phase-0 inventory/issue-link reconciliation, Drew two option-B order recommendations flagged for confirmation, no D1 box ticked and #534 left open.

<!-- workflow:state -->

{
"version": 1,
"id": "2026-11-03T00-05-09Z_research_phase-12-package-534",
"created_at": "2026-11-03T00:05:09Z",
"stage": "intake",
"status": "prepared",
"transcript": {
"path": "transcript.txt",
"sha256": "ac02da879e5cd4bfd0754d6e5ab3a49c81603c819761413f8b5f27f1212d317f"
},
"concepts": [
{
"id": "C001",
"title": "F12.1 declarative world manifest, apply, diff and dry-run delta — D1-019",
"path": "concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md",
"status": "created",
"source_lines": [
[
1,
18
],
[
113,
128
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C002",
"title": "F12.4 capability provider registry with manifested components — D1-022",
"path": "concepts/C002-d1-022-f12-4-provider-registry-manifest.md",
"status": "created",
"source_lines": [
[
19,
23
],
[
130,
140
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C003",
"title": "F12.7 versioned guest ABI spec, plaza_abi_version export and conformance suite — D1-025",
"path": "concepts/C003-d1-025-f12-7-guest-abi-spec-version-conformance.md",
"status": "created",
"source_lines": [
[
24,
28
],
[
166,
176
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C004",
"title": "F12.3 roles v2 builder and moderator scopes with journaled audit — D1-021",
"path": "concepts/C004-d1-021-f12-3-builder-moderator-scopes-audit.md",
"status": "created",
"source_lines": [
[
29,
33
],
[
149,
156
]
],
"priority": "recommended",
"horizon": "now"
},
{
"id": "C005",
"title": "F12.6 durable structured audit log with OpenTelemetry export — D1-024",
"path": "concepts/C005-d1-024-f12-6-structured-audit-log-otel-export.md",
"status": "created",
"source_lines": [
[
34,
38
],
[
178,
180
]
],
"priority": "recommended",
"horizon": "now"
},
{
"id": "C006",
"title": "F12.5 first new providers kv blob http.fetch notify — D1-023",
"path": "concepts/C006-d1-023-f12-5-first-new-providers-kv-blob-http-notify.md",
"status": "created",
"source_lines": [
[
39,
43
],
[
180,
181
]
],
"priority": "recommended",
"horizon": "next"
},
{
"id": "C007",
"title": "F12.8 Rust and AssemblyScript-or-Zig guest SDKs with WIT/component-model decision — D1-026",
"path": "concepts/C007-d1-026-f12-8-guest-sdks-wit-component-model.md",
"status": "created",
"source_lines": [
[
44,
48
]
],
"priority": "contingent",
"horizon": "later"
},
{
"id": "C008",
"title": "F12.9 program upgrade with state migration and rollback — D1-027",
"path": "concepts/C008-d1-027-f12-9-program-upgrade-migration-rollback.md",
"status": "created",
"source_lines": [
[
49,
53
]
],
"priority": "contingent",
"horizon": "later"
},
{
"id": "C009",
"title": "F12.10 fair cross-world scheduling per-account quotas and slow-client back-pressure — D1-028",
"path": "concepts/C009-d1-028-f12-10-fair-scheduling-quotas-backpressure.md",
"status": "created",
"source_lines": [
[
54,
58
]
],
"priority": "contingent",
"horizon": "later"
},
{
"id": "C010",
"title": "F12.14 portals bilateral cross-world messaging by mutual grant — D1-032",
"path": "concepts/C010-d1-032-f12-14-portals-cross-world-messaging.md",
"status": "created",
"source_lines": [
[
59,
63
]
],
"priority": "contingent",
"horizon": "later"
},
{
"id": "C011",
"title": "F12.17 module signature trust list and compiler-version provenance — D1-035",
"path": "concepts/C011-d1-035-f12-17-module-trust-compiler-provenance.md",
"status": "created",
"source_lines": [
[
64,
112
],
[
182,
183
]
],
"priority": "contingent",
"horizon": "later"
},
{
"id": "C012",
"title": "Intake envelope phase-0 process directives ownership and tracker policy — G bucket",
"path": "concepts/C012-intake-envelope-phase-0-process-directives-ownership.md",
"status": "created",
"source_lines": [
[
146,
148
],
[
160,
164
],
[
185,
204
],
[
130,
145
],
[
156,
158
]
],
"priority": "unspecified",
"horizon": "unspecified"
}
],
"dependencies": [
{
"from": "C004",
"to": "C001",
"kind": "recommended-after",
"reason": "Roles v2 research can start on the merged-design reconciliation; if the F12.1 manifest delta (roles/invites, apply rerun semantics) is still open when C004 research runs, the wire-authorization design is re-checked against the landed delta.",
"rework_if_early": "re-check the live install/revoke authorization design against the F12.1 roles/invites delta once it lands."
},
{
"from": "C006",
"to": "C002",
"kind": "recommended-after",
"reason": "First-providers research can start on the catalog/dispatch seam; implementation waits for the registry design, and provider schemas are re-checked against the landed registry seam.",
"rework_if_early": "re-check provider schemas, ceilings and dispatch validation against the F12.4 registry seam once it lands."
},
{
"from": "C006",
"to": "C005",
"kind": "recommended-after",
"reason": "Durable audit should precede external-effect providers; research may start early, implementation must honor audit-first.",
"rework_if_early": "provider audit wiring is re-specified against the audit schema once it lands."
},
{
"from": "C007",
"to": "C003",
"kind": "recommended-after",
"reason": "SDK research can start on the current exports/loader surface; the SDK conformance claim is re-checked against the versioned F12.7 ABI and its suite once specified.",
"rework_if_early": "re-check the SDK conformance claim against the versioned ABI and conformance suite once specified."
},
{
"from": "C008",
"to": "C003",
"kind": "recommended-after",
"reason": "Upgrade/migration research can start on the checkpoint/snapshot model; the migration contract is re-checked against the versioned F12.7 ABI once specified.",
"rework_if_early": "re-check the migration contract and probe validation against the versioned ABI once specified."
},
{
"from": "C010",
"to": "C002",
"kind": "recommended-after",
"reason": "Portal research can start on the bilateral-grant contract; implementation waits for the registry seam, and the portal capability shape is re-checked against it once it lands.",
"rework_if_early": "re-check the portal capability shape against the F12.4 registry seam once it lands."
},
{
"from": "C010",
"to": "C005",
"kind": "recommended-after",
"reason": "Portal research can start on the bilateral-grant contract; implementation waits for the durable audit contract, and dual-world audit entries are re-checked against it once it lands.",
"rework_if_early": "re-check dual-world audit entries against the F12.6 durable audit schema once it lands."
}
],
"issues": [],
"prs": [],
"tasks": [],
"events": [
{
"at": "2026-11-03T00:05:09Z",
"stage": "intake",
"status": "prepared",
"reason": "Exact transcript captured and initial concept files prepared."
}
]
}
<!-- /workflow:state -->

## Files

- [main.md](main.md)
- [transcript.txt](transcript.txt)
- [design.md](design.md)
- [changes.md](changes.md)
- [concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md](concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md)
- [concepts/C002-d1-022-f12-4-provider-registry-manifest.md](concepts/C002-d1-022-f12-4-provider-registry-manifest.md)
- [concepts/C003-d1-025-f12-7-guest-abi-spec-version-conformance.md](concepts/C003-d1-025-f12-7-guest-abi-spec-version-conformance.md)
- [concepts/C004-d1-021-f12-3-builder-moderator-scopes-audit.md](concepts/C004-d1-021-f12-3-builder-moderator-scopes-audit.md)
- [concepts/C005-d1-024-f12-6-structured-audit-log-otel-export.md](concepts/C005-d1-024-f12-6-structured-audit-log-otel-export.md)
- [concepts/C006-d1-023-f12-5-first-new-providers-kv-blob-http-notify.md](concepts/C006-d1-023-f12-5-first-new-providers-kv-blob-http-notify.md)
- [concepts/C007-d1-026-f12-8-guest-sdks-wit-component-model.md](concepts/C007-d1-026-f12-8-guest-sdks-wit-component-model.md)
- [concepts/C008-d1-027-f12-9-program-upgrade-migration-rollback.md](concepts/C008-d1-027-f12-9-program-upgrade-migration-rollback.md)
- [concepts/C009-d1-028-f12-10-fair-scheduling-quotas-backpressure.md](concepts/C009-d1-028-f12-10-fair-scheduling-quotas-backpressure.md)
- [concepts/C010-d1-032-f12-14-portals-cross-world-messaging.md](concepts/C010-d1-032-f12-14-portals-cross-world-messaging.md)
- [concepts/C011-d1-035-f12-17-module-trust-compiler-provenance.md](concepts/C011-d1-035-f12-17-module-trust-compiler-provenance.md)
- [concepts/C012-intake-envelope-phase-0-process-directives-ownership.md](concepts/C012-intake-envelope-phase-0-process-directives-ownership.md)

Transcript SHA-256: `ac02da879e5cd4bfd0754d6e5ab3a49c81603c819761413f8b5f27f1212d317f`. Source bytes are immutable.

<a id="c001"></a>

## C001: F12.1 declarative world manifest, apply, diff and dry-run delta — D1-019

Document: [concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md](concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md).

Intake and reconciliation for the F12.1 world.toml manifest, idempotent apply/dry-run and diff, including the roles/invites gap and partial-apply/rerun semantics. Phase-0 reconciliation against implementation issue #807 (CLOSED 2026-11-02: the body status snapshot is stale).

<!-- workflow:view concept-C001 -->

- **State:** `created` · stage S0 · priority `crucial` · horizon `now`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Referenced by:** [C004](#c004) `recommended-after`
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C001 -->

- **C001.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #807 (F12.1 apply wiring) verified CLOSED on intake day; the body 'not started' snapshot is stale and must not be copied forward.
- **C001.2** `created` · S0 · 2026-11-03T00:07:00Z — Merged-design phase-1 delta scope for F12.1: roles/invites in the manifest model and partial-failure/rerun guarantees; do not re-design completed manifest/apply/diff behavior.

<a id="c002"></a>

## C002: F12.4 capability provider registry with manifested components — D1-022

Document: [concepts/C002-d1-022-f12-4-provider-registry-manifest.md](concepts/C002-d1-022-f12-4-provider-registry-manifest.md).

Intake for the F12.4 provider registry: provider manifests, the catalog/dispatch replacement seam, and gate #838. Reconciles the open state of #1061 with the Phase 12 plan ordering.

<!-- workflow:view concept-C002 -->

- **State:** `created` · stage S0 · priority `crucial` · horizon `now`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Referenced by:** [C006](#c006) `recommended-after`, [C010](#c010) `recommended-after`
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C002 -->

- **C002.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1061 (F12.4 registry) verified OPEN; gate #838 stands as an implementation prerequisite per the merged design.
- **C002.2** `created` · S0 · 2026-11-03T00:07:00Z — Merged-design foundation scope for F12.4: provider manifest/registry seam distinguishing host providers from guest/platform ABI names; resolve gate #838 before implementation.

<a id="c003"></a>

## C003: F12.7 versioned guest ABI spec, plaza_abi_version export and conformance suite — D1-025

Document: [concepts/C003-d1-025-f12-7-guest-abi-spec-version-conformance.md](concepts/C003-d1-025-f12-7-guest-abi-spec-version-conformance.md).

Intake for the versioned plaza_* guest ABI spec, the plaza_abi_version export, and the conformance suite built on fixtures/recordings. Reconciles open #1064 with the #544 verify prerequisite claim.

<!-- workflow:view concept-C003 -->

- **State:** `created` · stage S0 · priority `crucial` · horizon `now`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Referenced by:** [C007](#c007) `recommended-after`, [C008](#c008) `recommended-after`
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C003 -->

- **C003.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1064 (F12.7 ABI) verified OPEN; the #544 verify-substrate prerequisite claim must be re-checked on current origin/main in S1.
- **C003.2** `created` · S0 · 2026-11-03T00:07:00Z — Merged-design foundation scope for F12.7: versioning/compatibility plus a complete conformance contract; verify against the existing id world verify surface.

<a id="c004"></a>

## C004: F12.3 roles v2 builder and moderator scopes with journaled audit — D1-021

Document: [concepts/C004-d1-021-f12-3-builder-moderator-scopes-audit.md](concepts/C004-d1-021-f12-3-builder-moderator-scopes-audit.md).

Intake for F12.3 journaled builder/moderator scopes and per-use audit. Reconciles the merged design finding (role bits and journal records exist; live install still requires the admin token) and open remainder #835 with D1-019 ordering.

<!-- workflow:view concept-C004 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `now`
- **Items:** 2 (2 `created`)
- **Relations:**
  - `recommended-after` [C001](#c001): Roles v2 research can start on the merged-design reconciliation; if the F12.1 manifest delta (roles/invites, apply rerun semantics) is still open when C004 research runs, the wire-authorization design is re-checked against the landed delta.
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C004 -->

- **C004.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: F12.3 remainder #835 verified OPEN; merged design finds role bits and journal records exist while live install still requires the admin token — the delta is the wire authorization path.
- **C004.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.3: the live install/revoke wire-authorization design (builder/moderator acceptance on the session frames and service install path), with a test on the actual protocol acceptance path — not role bits or journal records, which the merged design records as already present.

<a id="c005"></a>

## C005: F12.6 durable structured audit log with OpenTelemetry export — D1-024

Document: [concepts/C005-d1-024-f12-6-structured-audit-log-otel-export.md](concepts/C005-d1-024-f12-6-structured-audit-log-otel-export.md).

Intake for the durable, replayable F12.6 audit log with the OpenTelemetry export as an adapter. Reconciles open #1063 and the F11.7/#553 dependency while keeping audit design parallelizable.

<!-- workflow:view concept-C005 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `now`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Referenced by:** [C006](#c006) `recommended-after`, [C010](#c010) `recommended-after`
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C005 -->

- **C005.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1063 (F12.6 audit) verified OPEN with F11.7/#553 dependency; durable replayable audit facts are separated from snapshots/metrics, OpenTelemetry is an adapter.
- **C005.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.6: the durable, replayable audit event schema with per-event ownership, replay-to-grants test shape, and the OpenTelemetry export specified as an adapter — separate from operational metrics and in-memory CapUsage snapshots.

<a id="c006"></a>

## C006: F12.5 first new providers kv blob http.fetch notify — D1-023

Document: [concepts/C006-d1-023-f12-5-first-new-providers-kv-blob-http-notify.md](concepts/C006-d1-023-f12-5-first-new-providers-kv-blob-http-notify.md).

Intake for bounded durable KV, content-addressed blobs, allowlisted http.fetch, and rate-limited notify. Reconciles open #1062 and carries the unresolved F12.5-vs-F12.6 ordering question with the design option-B recommendation.

<!-- workflow:view concept-C006 -->

- **State:** `created` · stage S0 · priority `recommended` · horizon `next`
- **Items:** 2 (2 `created`)
- **Relations:**
  - `recommended-after` [C002](#c002): First-providers research can start on the catalog/dispatch seam; implementation waits for the registry design, and provider schemas are re-checked against the landed registry seam.
  - `recommended-after` [C005](#c005): Durable audit should precede external-effect providers; research may start early, implementation must honor audit-first.
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C006 -->

- **C006.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1062 (F12.5 providers) verified OPEN; option-B recommendation (F12.6 durable audit before external-effect providers) carried for Drew confirmation, not a research blocker.
- **C006.2** `created` · S0 · 2026-11-03T00:07:00Z — Order question Q1 carried for Drew: F12.5 vs F12.6 — options (A) plan order providers-first or (B) audit-first per D1-023; recommendation B; update the plan sequence if confirmed.

<a id="c007"></a>

## C007: F12.8 Rust and AssemblyScript-or-Zig guest SDKs with WIT/component-model decision — D1-026

Document: [concepts/C007-d1-026-f12-8-guest-sdks-wit-component-model.md](concepts/C007-d1-026-f12-8-guest-sdks-wit-component-model.md).

Intake for the F12.8 research package: Rust and AssemblyScript-or-Zig SDKs equal to the Roc helpers, each with an example world and a passing conformance run, plus the WIT/component-model decision. Reconciles open #1065 and the title/body language discrepancy (TypeScript/Zig vs AssemblyScript or Zig).

<!-- workflow:view concept-C007 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Items:** 2 (2 `created`)
- **Relations:**
  - `recommended-after` [C003](#c003): SDK research can start on the current exports/loader surface; the SDK conformance claim is re-checked against the versioned F12.7 ABI and its suite once specified.
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C007 -->

- **C007.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1065 (F12.8 SDKs) verified OPEN; the title/body language discrepancy (TypeScript/Zig vs AssemblyScript or Zig) and the WIT/component-model decision are recorded as S1 research questions.
- **C007.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.8: a research decision record resolving the title/body language discrepancy (TypeScript/Zig vs AssemblyScript or Zig) and the WIT/component-model direction, with each SDK bound to the same F12.7 conformance suite as its exit check.

<a id="c008"></a>

## C008: F12.9 program upgrade with state migration and rollback — D1-027

Document: [concepts/C008-d1-027-f12-9-program-upgrade-migration-rollback.md](concepts/C008-d1-027-f12-9-program-upgrade-migration-rollback.md).

Intake for versioned snapshots, the optional migrate export, probe-instance validation, journal commit semantics, transient-state disposition, and rollback. Reconciles open #1066 with the F12.7 prerequisite.

<!-- workflow:view concept-C008 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Items:** 2 (2 `created`)
- **Relations:**
  - `recommended-after` [C003](#c003): Upgrade/migration research can start on the checkpoint/snapshot model; the migration contract is re-checked against the versioned F12.7 ABI once specified.
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C008 -->

- **C008.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1066 (F12.9 upgrades) verified OPEN with the F12.7 prerequisite; the design must specify the commit point (round-trip or no commit) and rollback semantics.
- **C008.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.9: versioned migration with the optional migrate export, probe-instance validation, the journal commit point (round-trip or no commit, prior state preserved), transient-state disposition, and rollback — against the checkpoint/snapshot model.

<a id="c009"></a>

## C009: F12.10 fair cross-world scheduling per-account quotas and slow-client back-pressure — D1-028

Document: [concepts/C009-d1-028-f12-10-fair-scheduling-quotas-backpressure.md](concepts/C009-d1-028-f12-10-fair-scheduling-quotas-backpressure.md).

Intake for the hub scheduler, per-account quotas (worlds, storage, fuel/s), fair fuel/CPU sharing, and slow-client back-pressure with the p99 isolation acceptance. Reconciles open #1067 with F11.7/#553 and F11.8/#539 prerequisites.

<!-- workflow:view concept-C009 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C009 -->

- **C009.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1067 (F12.10 scheduling) verified OPEN with F11.7/#553 and F11.8/#539 prerequisites; the p99 isolation threshold is the acceptance anchor.
- **C009.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.10: per-account quotas, fair CPU/fuel sharing at the hub/host seam, and slow-client back-pressure — with the measurable exit that a spinning world raises no other world's p99 by more than 10 percent.

<a id="c010"></a>

## C010: F12.14 portals bilateral cross-world messaging by mutual grant — D1-032

Document: [concepts/C010-d1-032-f12-14-portals-cross-world-messaging.md](concepts/C010-d1-032-f12-14-portals-cross-world-messaging.md).

Intake for portals as a new bilateral guest capability with bounded rate/size and audit entries on both worlds. Reconciles open #1068 and keeps the existing host-level cross-world switch out of scope.

<!-- workflow:view concept-C010 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Items:** 2 (2 `created`)
- **Relations:**
  - `recommended-after` [C002](#c002): Portal research can start on the bilateral-grant contract; implementation waits for the registry seam, and the portal capability shape is re-checked against it once it lands.
  - `recommended-after` [C005](#c005): Portal research can start on the bilateral-grant contract; implementation waits for the durable audit contract, and dual-world audit entries are re-checked against it once it lands.
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C010 -->

- **C010.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1068 (F12.14 portals) verified OPEN; portals are a new bilateral guest capability and the host-level cross-world switch stays out of scope.
- **C010.2** `created` · S0 · 2026-11-03T00:17:00Z — S2 deliverable scope for F12.14: portals as a new bilateral guest capability with bounded rate/size and an audit entry on both worlds — explicitly not a broadening of the existing host-level cross-world switch.

<a id="c011"></a>

## C011: F12.17 module signature trust list and compiler-version provenance — D1-035

Document: [concepts/C011-d1-035-f12-17-module-trust-compiler-provenance.md](concepts/C011-d1-035-f12-17-module-trust-compiler-provenance.md).

Intake for module-signature verification against an admin trust list, compiler-version provenance, the journaled trust decision, and unsigned-module refusal. Reconciles CLOSED #1069 and carries the unresolved F11.10-vs-#541/F11.18 prerequisite question with the option-B recommendation.

<!-- workflow:view concept-C011 -->

- **State:** `created` · stage S0 · priority `contingent` · horizon `later`
- **Items:** 2 (2 `created`)
- **Relations:** none
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C011 -->

- **C011.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation: implementation issue #1069 (F12.17 trust) verified CLOSED on intake day — S1 must check closure evidence before citing it; option-B recommendation (#541/F11.18 gate) carried for Drew confirmation, not a research blocker.
- **C011.2** `created` · S0 · 2026-11-03T00:07:00Z — Order question Q2 carried for Drew: F12.17 prerequisite — options (A) plan F11.10 gate or (B) D1-035 #541/F11.18 gate; recommendation B; update the plan if confirmed.

<a id="c012"></a>

## C012: Intake envelope phase-0 process directives ownership and tracker policy — G bucket

Document: [concepts/C012-intake-envelope-phase-0-process-directives-ownership.md](concepts/C012-intake-envelope-phase-0-process-directives-ownership.md).

Global bucket for the merged-design workflow rules and tracker policy: exactly one shared package with one concept per D1, issue/comment/code reconciliation first, no duplicate implementation issues, independently mergeable designs, dependencies gate implementation not research, and the no-box-ticked-without-artifact-plus-link acceptance rule. Also carries corrected cross-references and the 2026-10-27 queue tier/ownership ordering.

<!-- workflow:view concept-C012 -->

- **State:** `created` · stage S0
- **Items:** 1 (1 `created`)
- **Relations:** none
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C012 -->

- **C012.1** `created` · S0 · 2026-11-03T00:06:00Z — Phase-0 reconciliation record: D1 inventory is exactly the 11 items; implementation map #807(CLOSED) #1061 #1064 #835 #1063 #1062 #1065 #1066 #1067 #1068 #1069(CLOSED) plus #1354-correction (F12.2 stays with #556); ordering tier path-first then Phase 12; two option-B questions carried; no D1 box ticked, #534 left open.

## Issues and PRs

<!-- workflow:view issues-and-prs -->

No issues or PRs are recorded yet. Each one will be listed here under its concept.

<!-- /workflow:view issues-and-prs -->
