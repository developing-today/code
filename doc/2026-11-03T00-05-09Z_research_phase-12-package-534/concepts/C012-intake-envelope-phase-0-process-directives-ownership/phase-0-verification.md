# Phase-0 verification log and intake reading register — task 6507

Owner: intake (queue task 6507, OpenChamber scheduled lane aea12c82-c5af-4e5a-9b66-f8e360f87177).
This file is supporting material for C012. It records facts verified LIVE at intake
(commands below, all read-only) that the transcript alone does not contain, and every
interpretive reading the intake items add on top of the transcript.

## Verification log (all times UTC, 2026-11-03)

- `git fetch -q origin main` in the task worktree; `git rev-parse origin/main` =
  `10e4ec1b7`. Branch `queue/6507-534-phase12-package` tracks this base.
- `gh issue view <n> --repo developing-today/code --json number,state,title,closedAt,url`
  for the implementation-issue map (verified states):
  - #807 CLOSED 2026-10-30T01:01:27Z — id: F12.1 apply wiring.
  - #1061 OPEN — id: F12.4 provider registry.
  - #1064 OPEN — id: F12.7 ABI specification and conformance suite.
  - #835 OPEN — id(world): F14.6 builder and moderator authority (F12.3 remainder).
  - #1063 OPEN — id: F12.6 audit log and OpenTelemetry export.
  - #1062 OPEN — id: F12.5 first new providers.
  - #1065 OPEN — id: F12.8 guest SDKs.
  - #1066 OPEN — id: F12.9 program upgrade.
  - #1067 OPEN — id: F12.10 fair scheduling and per-account quotas.
  - #1068 OPEN — id: F12.14 portals.
  - #1069 CLOSED 2026-11-02T10:58:07Z — id: F12.17 trust and provenance.
- `gh pr view 3912` → MERGED 2026-11-02T15:47:38Z (the workflow design PR).
- `gh issue view 534` → OPEN, no held/user-owned/do-not-merge/do-not-auto-edit labels.
- Code spot-checks on `origin/main` (`10e4ec1b7`):
  - `world.toml` manifest + `apply`/`diff` exist: `pkgs/id/src/cli.rs`
    (`WorldCommand` enum ~:1615), `pkgs/id/src/commands/world.rs`
    (`WorldCommand::Diff` ~:477, `WorldCommand::Apply` ~:493, apply plan/execute
    ~:1097-1160), `pkgs/id/src/world_manifest.rs` (`WorldManifest` ~:30).
  - No `ProviderRegistry`/`provider_registry` and no `plaza_abi_version` anywhere
    under `pkgs/id` (matches the merged design's reconciliation table).
  - Capability surface moved relative to the design's pinned citations: the
    static catalog now lives at `pkgs/id/crates/id-core/src/world_caps.rs`
    (re-exported via `pkgs/id/src/lib.rs:240`); the design's
    `pkgs/id/src/world_caps.rs:*` paths are stale. S1 must re-pin every
    design-table line number on the current base before writing research questions.

## Intake reading register

Inference: every item text in main.md that goes beyond the transcript excerpts
is an intake reading, not a transcript quote. The readings:

- Inference: the issue-body "not started" snapshot for F12.1 (transcript lines
  16, 21, 26 and siblings) is stale — Session C progress (2026-10-26 comments,
  transcript lines 113-128) and the apply-wiring merge (#807 closed
  2026-10-30) supersede it. Carried as: do not re-design completed
  manifest/apply/diff behavior; design the roles/invites delta and rerun
  guarantees instead (C001.2).
- Inference: the "wire authorization path" is the F12.3 design delta — role bits
  and journal records exist per the merged design, the live install/revoke path
  is what remains (C004.1, C004.2).
- Inference: "durable, replayable audit facts, OpenTelemetry as adapter" is the
  F12.6 seam that separates audit from snapshots/metrics and in-memory
  `CapUsage` (C005.1, C005.2). The audit-before-providers recommendation (option
  B, transcript lines 180-181 under C006) applies to provider implementation,
  not to starting provider research.
- Inference: "distinguishing host providers from guest/platform ABI names" is
  the F12.4 registry seam to define (C002.2); gate #838 must be resolved before
  registry implementation.
- Inference: "verify against the existing `id world verify` surface" is the
  F12.7 prerequisite-reconciliation question for S1 (C003.1, C003.2).
- Inference: the two option-B recommendations (audit before providers; pinned
  compiler #541/F11.18 before trust) are reviewable defaults flagged for Drew
  confirmation, not research blockers (C006.2, C011.2, C012.1).

## Known intake blemishes (accepted, not hidden)

- Transcript line 156 (`- **F12.5 ...** = **D1-023** ... tracked here;`) is
  claimed by both C004 (`source_lines` [149,156]) and C012 ([156,158]). The
  overlap is one cross-reference tail line relevant to both the F12.3 concept
  and the envelope; both excerpts remain byte-exact.
- The C012 concept file's own excerpt ranges are non-contiguous ([130,145] then
  [156,158]) because the 2026-10-27 reprioritization comment (lines 130-144) and
  the F12.2-correction tail (lines 156-158) are separated by the F12.3
  cross-reference block owned by C004.
- Wave-1 readiness in design.md is intra-package research readiness (all 12
  concepts ready after the requires→recommended-after correction). External
  implementation gates (#544 verify substrate, F11.7/#553 metrics, F11.8/#539
  bench, #541 pinned compiler, #838 registry gate) are NOT shown done and gate
  implementation per the merged design — never research.
- Finding rebutted: the C011 concept file's excerpt fences are balanced (a
  programmatic fence scan finds no unclosed opener, and `check` passes). The
  `--- DECISION-BEARING ... ---` divider visible inside the first C011 excerpt
  is literal transcript line 111, which falls inside C011's [64,112] range — it
  reads like a section header but is quoted content.
