
===== code#9245 draft=True mergeable=UNKNOWN state=UNKNOWN files=152 +97365/-0 created=2026-11-13T03:52:37Z updated=2026-11-23T20:32:14Z
TITLE: design(fleet-ca): v0.3 design-mill build-out, traceability and granular backlog
BODY: Summary
Related program: #9155. Draft: the design mission is still in progress; do not close the implementation epic.
source package → statement inventory → additive V1/V2/V3/V4 designs
              → independent challenge/re-review → granular issue packets
              → verified publication receipts and source/test traceability

The branch preserves the stopped run and adds reviewed architecture, custody/freshness/revocation gates, provider/format and future/MTC research. An immutable original-source snapshot is protected from upstream formatting; the formatted upstream files remain unchanged.
Evidence

Before: partial unreviewed drafts, stale packet references and the interrupted worktree.
After: compliant worktree; pushed preservation commits; source/precedence resolutions; independent performance and future-track review receipts.
Original snapshot: five supplied SHA256SUMS entries
COMMENT by coderabbitai at 2026-11-13T03:52:45Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
COMMENT by dezren39 at 2026-11-22T20:10:37Z: Program tending — stalled PR check-in (tracking: #1352)

No activity for 2 day(s); last update 2026-11-20T01:37:27Z.
Waiting on: draft — waiting on its author.
Next step: mark the PR ready for review, or post what the draft is waiting on.
COMMENT by dezren39 at 2026-11-23T20:32:14Z: Coordinator disposition — the dependency on this PR is superseded (2026-11-23)
Under Drew's order of 2026-11-23 (fleet CA program executed end to end; rulings recorded on #9155), the CERT-PROGRAM coordinator records the following disposition for this PR:
This PR is NOT landed as-is, and the program's dependency on its merge is lifted. Reasons, from the PR's own record:

It is a draft whose own summary states the design mission is still in progress, with the final semantic inventory/mapping audit
FILES: doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/README.md, doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/architecture-review.md, doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/designs/architecture.md, doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/designs/federation-infrastructure.md, doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/designs/generators.md, doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/designs/opera

===== code#12038 draft=True mergeable=UNKNOWN state=UNKNOWN files=17 +654/-19 created=2026-11-19T09:53:42Z updated=2026-11-21T05:36:54Z
TITLE: test(id): add initial parser and target goldens (#7919)
BODY: Summary

Add a checked-in parser corpus with stable normalized IR goldens and a harness that runs under �[?2004lcargo test -p id-render�[?2004l.
Add one shared semantic source with emitted structural goldens for web, macOS native, and custom GUI targets.
Document the incremental scope and golden-update policy.

Verification

�[?2004lcargo fmt -p id-render --check�[?2004l
�[?2004lcargo test -p id-render�[?2004l (all tests pass)
�[?2004lgit diff --check�[?2004l

Remaining acceptance work
This is an initial slice, not completion of #7919. The corpus does not yet cover every element kind or assertion role, and target goldens currently exercise only one shared case. The #5850 assertion-role vocabulary is still unsettled; do not invent its inventory. Extend vocabulary/target coverage once that contract is settled. No pixel/screenshot expectations are used.



Open in [code]smith · Need help on
COMMENT by coderabbitai at 2026-11-19T09:53:50Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/id/crates/id-render/Cargo.toml, pkgs/id/crates/id-render/src/lib.rs, pkgs/id/crates/id-render/src/validate.rs, pkgs/id/crates/id-render/tests/conformance/coverage/assertion-roles.txt, pkgs/id/crates/id-render/tests/conformance/coverage/vocabulary-elements.id, pkgs/id/crates/id-render/tests/conformance/parser/README.md, pkgs/id/crates/id-render/tests/conformance/parser/basic.golden, pkgs/id/crates/id-render/tests/conformance/parser/basic.id, pkgs/id/crates/id-render/tests/conformance/parser/

===== code#12301 draft=True mergeable=UNKNOWN state=UNKNOWN files=8 +1993/-51 created=2026-11-22T02:17:08Z updated=2026-11-22T10:00:06Z
TITLE: feat(atlas): add sweep discovery adapters and staging
BODY: Summary
Fixture-backed Atlas sweep discovery and staging across boot records, session logs, chat exports, statistics reports, and final report files.

Discovery adapters classify candidates for the existing deterministic filter; secret, partial, oversized, off-allowlist, and symlink cases are not read/landed.
run_once_with_staging stages accepted candidates by content address before sweep state persists their hashes. A failed stage leaves state retryable; a repeated successful run emits duplicate-content and does not add another staging row.
Completed run manifest decisions are persisted append-only in sweep state.
A supplied existing #6058 embedding-ledger row is checked against the documented schema and batch identity; the sweep report carries its reference and measured embedding fields. The runner does not create a local substitute ledger row.
Read-only host probes sample source/stagi
COMMENT by coderabbitai at 2026-11-22T02:17:15Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/atlas-search/Cargo.lock, pkgs/atlas-search/crates/atlas-core/Cargo.toml, pkgs/atlas-search/crates/atlas-core/src/lib.rs, pkgs/atlas-search/crates/atlas-core/src/stage.rs, pkgs/atlas-search/crates/atlas-core/src/sweep.rs, pkgs/atlas-search/crates/atlas-core/src/sweep_adapters.rs, pkgs/atlas-search/crates/atlas-core/src/sweep_probe.rs, pkgs/atlas-search/crates/atlas-core/src/turso.rs

===== code#12312 draft=True mergeable=UNKNOWN state=UNKNOWN files=1 +22/-0 created=2026-11-22T02:26:26Z updated=2026-11-22T02:26:33Z
TITLE: docs(mcpx): show fleet laptop SSH invocation
BODY: Summary
 pkgs/mcpx/docs/remote.md
+  Fleet NixOS laptop command reaches this VM through mcpx `ssh://`
+  transport using existing SSH authorization; no token or daemon port.
Evidence

Before: remote transport docs covered ssh:// generically but had no fleet-laptop invocation.
After: docs show one-command discovery and MCP-call examples plus an SSH preflight. git diff --check passes.
Still needed for acceptance: execute the preflight and one MCP call from the fleet laptop. This execution VM has no laptop-agent HTTPS proxy configured; its own SSH key is not authorized for the VM.

Merge Danger
Door: two-way.
Blast Radius: documentation only.



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-22T02:26:33Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/mcpx/docs/remote.md

===== code#12348 draft=True mergeable=UNKNOWN state=UNKNOWN files=5 +1700/-118 created=2026-11-22T03:14:49Z updated=2026-11-22T12:53:49Z
TITLE: fix(id): void adverse pending-hold grants (#2723)
BODY: Summary
This draft extends the phase-2 grant flow with native TigerBeetle-shaped pending transfers, scoped adverse-event voiding, and a deterministic sweep for post/void/expiry/timeout recovery. Pending transfer credits go to the service-owned hold account; successful posting does not make value spendable.
Active-state grants remain disabled per the packet’s explicit §10 boundary: phase-4 clawback safeguards are not implemented. The phase-3 state machine now refuses PendingHold → Active, restore rejects active records, and a lost-race post records its attestation for phase 4 rather than mislabeling it as a successful void. This is not yet a complete implementation of §1.3, which requires the clean-post flow to become Active; keep the PR draft and do not claim the task complete until the boundary is resolved in code/spec.
Implemented

Native pending-transfer create, post, void, and author
COMMENT by coderabbitai at 2026-11-22T03:14:57Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/id/src/currency_poc/ledger.rs, pkgs/id/src/currency_poc/tb_ledger.rs, pkgs/id/src/currency_poc/tests.rs, pkgs/id/src/grant_service.rs, pkgs/id/src/grants.rs

===== code#12496 draft=True mergeable=UNKNOWN state=UNKNOWN files=1 +343/-9 created=2026-11-22T10:20:26Z updated=2026-11-22T10:20:33Z
TITLE: id(world): shared sync epochs for journal appends (#557) — DRAFT, negative result, do not merge
BODY: Summary
DRAFT — DO NOT MERGE. This is a negative result, recorded as a PR so the evidence is reviewable.
One slice attempt for the #557 F12.12 p99 criterion, aimed at the input/ack cost in felix's 2026-11-22 profile (PR #12471): after group commit (#8239), durability-before-ack still costs one sync_data per journal append, and at 100 worlds × 20 inputs/s the filesystem's aggregate sync rate was the suspected cap.
This branch routes FileJournal appends through a per-filesystem SyncGate (crates/id-worlds/src/world_store.rs): the writer writes its bytes, registers the next epoch, and waits until a syncfs that started after its registration completes; a failed or unavailable gate sync falls back to the writer's own sync_data, which decides the append exactly as before. The mechanism is correct — the measurements below say syncfs is the wrong primitive on every filesystem measurable from the 
COMMENT by coderabbitai at 2026-11-22T10:20:33Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/id/crates/id-worlds/src/world_store.rs

===== code#12510 draft=True mergeable=UNKNOWN state=UNKNOWN files=49 +9680/-1355 created=2026-11-22T11:46:18Z updated=2026-11-23T10:26:21Z
TITLE: feat(fleet): stage Access-authenticated laptop reads
BODY: Scope
This draft implements the private, outbound-only laptop bridge for four named Fleet reads: queue status, broker health, laptop health, and bounded diagnostics. Desktop Commander/native Codex remains the fallback. There is no shell, arbitrary filesystem access, generic proxy, or credential capture.
The selected source transport is no-WARP HTTPS to the fixed proposed hostname laptop-bridge-staging.security.cab, protected by Cloudflare Access service authentication and independent Ed25519 request signatures. Worker code requires Cloudflare’s directly authenticated ctx.access.aud to match the configured audience and validates the RS256 Access JWT against the exact issuer, claim audience, hostname, JWKS, signature and time claims. The signed common_name selects one unrevoked principal; x-bridge-principal must name that same principal before forwarding. It does not rely on the original C
COMMENT by coderabbitai at 2026-11-22T11:46:27Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/private-laptop-bridge-release.yml, doc/fleet-schedule-registry.md, pm-stack/aggregator/src/bridge-refresh.ts, pm-stack/aggregator/src/bridge-snapshot.ts, pm-stack/aggregator/src/queue.ts, pm-stack/aggregator/src/sources/live.ts, pm-stack/aggregator/test/bridge-refresh.test.ts, pm-stack/aggregator/test/bridge-snapshot.test.ts, pm-stack/private-laptop-bridge/.gitignore, pm-stack/private-laptop-bridge/ACCESS-STAGING.md, pm-stack/private-laptop-bridge/DISCOVERY.md, pm-stack/private

===== code#12539 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +525/-0 created=2026-11-22T12:39:19Z updated=2026-11-23T05:56:11Z
TITLE: id-caps: per-account quota policy and fuel-rate ledger (F12.10 slice 1)
BODY: Refs #1067
Slice
First implementation slice of F12.10 (fair scheduling and per-account quotas). The triage verdicts on #1067 confirmed zero quota/scheduler substrate on main: what exists is per-world ceilings (world_limits: one invocation's fuel, one world's memory), which bound any one world or call but say nothing about an account — one account can still monopolise a hub by spreading work across many worlds or invoking flat-out around the clock.
This slice lands the contract half, in the shape #557 established for the per-world memory budget, in id-caps next to world_limits:

AccountQuotas — validated admin policy: max_worlds, max_storage_bytes, max_fuel_per_second, each nonzero-checked, with proposed-policy defaults (16 worlds, 1 GiB, 100M fuel/s = ten full-budget default invocations per second, sustained).
FuelRateBudget — deterministic fixed-one-second-window ledger with caller-supp
COMMENT by coderabbitai at 2026-11-22T12:39:27Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 24 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-22T12:39:32Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T15:58:32.974663Z
18ef0c0
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
FILES: pkgs/id/crates/id-caps/Cargo.toml, pkgs/id/crates/id-caps/src/account_quota.rs, pkgs/id/crates/id-caps/src/lib.rs

===== code#12547 draft=True mergeable=UNKNOWN state=UNKNOWN files=8 +1885/-0 created=2026-11-22T13:18:27Z updated=2026-11-23T07:46:47Z
TITLE: Validate cf-container-hello Build Output (dry-run only)
BODY: Scope

Preserve the standalone cf-container-hello operator deploy path (npm run deploy remains wrangler deploy) and the existing wrangler.jsonc configuration byte-for-byte.
Add an isolated, lockfile-pinned cf/Wrangler toolchain and CI-only native cloudflare.config.ts for Build Output parity checks.
Add optional manual stage_versions validation, default off and restricted to developing-today/code on main.

Safety boundary
The workflow contains only a cf workers versions create --prebuilt --dry-run command after successful Build Output parity checks. It has no Cloudflare environment, credentials, image publish, Worker upload, or activation step. It claims no staging receipt or promotion.
The :v5 container URI and the other current tag references are not immutable digest pins. The pinned cf build was run credential-free and fails closed because cf/Wrangler requires image references of the f
COMMENT by coderabbitai at 2026-11-22T13:18:34Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/cf-container-hello.yml, pkgs/cf-container-hello/README.md, pkgs/cf-container-hello/cloudflare.config.ts, pkgs/cf-container-hello/release-tools/package-lock.json, pkgs/cf-container-hello/release-tools/package.json, pkgs/cf-container-hello/release-tools/verify-build.mjs, pkgs/cf-container-hello/release-tools/verify-config.mjs, pkgs/cf-container-hello/test/test_workflow_deploy_guard.py

===== code#12548 draft=False mergeable=UNKNOWN state=UNKNOWN files=6 +284/-15 created=2026-11-22T13:18:57Z updated=2026-11-23T05:56:19Z
TITLE: feat(pm-console): refresh open PRs from edge
BODY: Summary

Add opt-in, read-only GitHub open-PR refresh after 60 seconds of laptop silence.
Keep the GitHub token in the Worker secret boundary; expose readiness flags without exposing credentials.
Record successful and failed trusted fresh reads in the console document state.
This is one slice of #12518; it does not enable queue reads or write operations.

Evidence

Before: open PRs remained at the last laptop report while the laptop was away.
After: bun test — 112 pass; npm run build — pass; git diff --check — pass.
Ran on the laptop worktree after merging current origin/main into the branch.

Merge Danger

Low: direct reads are disabled unless GITHUB_READ_ENABLED=on and GITHUB_TOKEN are configured; no write endpoint is added.

Refs #12518



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-22T13:19:05Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 52 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-22T13:19:09Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T00:06:47.171570Z
18d9220
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
FILES: pm-stack/console/edge/README.md, pm-stack/console/edge/src/edge.ts, pm-stack/console/edge/src/github.ts, pm-stack/console/edge/src/worker.ts, pm-stack/console/src/docs.ts, pm-stack/console/test/edge.test.ts

===== code#12553 draft=True mergeable=UNKNOWN state=UNKNOWN files=11 +4109/-1 created=2026-11-22T13:27:47Z updated=2026-11-23T08:48:56Z
TITLE: github-ci: split Worker version upload and promotion
BODY: Summary

Keep upload and promotion manual-only, default-off, and without a push trigger. The pinned cf toolchain stays isolated from the operator Worker package.
The detached environment gates require the exact environment, required reviewers with self-review prevention, disabled administrator bypass, and exactly one typed branch:main policy. The protected job binds only to the environment name emitted by the successful gate.
Upload checks that Cloudflare's first/latest listed Version equals the serving baseline immediately before create, because strict binding inheritance resolves from the latest uploaded Version. It reads the exact Version UUID from the cf create response and binds the readback to that UUID and the exact run attempt.
Promotion requires the exact upload run/attempt receipt and baseline IDs, then verifies the exact create response and latest 100% deployment readback.
Clo
COMMENT by coderabbitai at 2026-11-22T13:27:53Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/worker-deploy-ci/cloudflare.config.ts, .github/worker-deploy-ci/package-lock.json, .github/worker-deploy-ci/package.json, .github/worker-deploy-ci/test/deployment-config.test.mjs, .github/worker-deploy-ci/test/deployment-guard.test.mjs, .github/workflows/github-ci-worker-promote.yml, .github/workflows/github-ci-worker-upload.yml, .gitignore, pkgs/github-ci/worker/README.md, pkgs/github-ci/worker/scripts/deployment-guard.mjs, pkgs/github-ci/worker/test/deploy_workflows_test.py

===== code#12554 draft=True mergeable=UNKNOWN state=UNKNOWN files=11 +2700/-8 created=2026-11-22T13:30:06Z updated=2026-11-22T14:11:39Z
TITLE: ci(muse): stage handoff Worker versions with cf
BODY: Summary

Replace the combined workflow with separate manual, main-only upload and promotion workflows using the pinned cf CLI and Cloudflare Workers Versions/Deployments APIs. No direct Wrangler deploy is used for routine releases.
Upload runs the exact source on an isolated staging Worker, checks generated config, and exercises the one-use create/claim/replay path with disposable dummy values. It then uploads the same source as a production version without changing production traffic.
Preserve the exact production upload response, complete worker source archive, source SHA, staging proof/config, and current production traffic snapshot in a run-specific artifact. Promotion validates workflow provenance, source ancestry, artifact digests, exact result.id, stage parity evidence, and unchanged serving deployment before promoting the exact version and verifying live traffic at 100%.
Add offl
COMMENT by coderabbitai at 2026-11-22T13:30:13Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/muse-handoff-promote.yml, .github/workflows/muse-handoff-upload.yml, scripts/muse/handoff/README.md, scripts/muse/handoff/worker/.gitignore, scripts/muse/handoff/worker/cloudflare.config.ts, scripts/muse/handoff/worker/package-lock.json, scripts/muse/handoff/worker/package.json, scripts/muse/handoff/worker/release-contract.mjs, scripts/muse/handoff/worker/release-contract.test.mjs, scripts/muse/handoff/worker/stage-smoke.mjs, scripts/muse/handoff/worker/workflow-guards.test.mjs

===== code#12557 draft=False mergeable=MERGEABLE state=CLEAN files=7 +977/-41 created=2026-11-22T13:31:37Z updated=2026-11-23T05:56:26Z
TITLE: id-worlds: charge an installed account fuel-rate budget per update (#1067 slice 2)
BODY: Stacked PR
Base is xaltha/id-f1210-account-quota (PR #12539, slice 1) — this slice builds directly on its FuelRateBudget. Retargets to main when #12539 merges.
What
F12.10 slice 2 — the invocation-boundary half of the slice-1 contract. WorldInstance (the sandbox, the one place a guest invocation's fuel is actually set and metered) gains an optional FuelRateBudget:

set_fuel_rate_budget(...) installs the owning account's budget; fuel_rate_budget() exposes it for reporting.
Every update admission charges the budget the invocation's fuel allowance (limits.fuel — what the store is about to be handed) before the guest runs. A FuelRateRefusal means the invocation never happens, nothing is metered, and the instance is not poisoned — the same call can succeed once the window rolls.
Metered consumption stays in CallMetrics; the two accounts answer different questions (may it run vs what did it bu
COMMENT by coderabbitai at 2026-11-22T13:31:44Z: Important
Review skipped
Auto reviews are disabled on base/target branches other than the default branch.
Please check the settings in the CodeRabbit UI or the .coderabbit.yaml file in this repository. To trigger a single review, invoke the @coderabbitai review command.

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository applied
Commit: Unavailable
Layers: defaults
Review profile: CHILL
Plan: Advanced
Run ID: ba42
COMMENT by chatgpt-codex-connector at 2026-11-22T13:31:48Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T18:41:10.357816Z
d233bb2
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
FILES: pkgs/id/crates/id-caps/src/account_quota.rs, pkgs/id/crates/id-worlds/src/sandbox.rs, pkgs/id/crates/id-worlds/src/world/actor.rs, pkgs/id/crates/id-worlds/src/world/actor/execution.rs, pkgs/id/crates/id-worlds/src/world/handle.rs, pkgs/id/crates/id-worlds/src/world/mod.rs, pkgs/id/crates/id-worlds/src/world/tests.rs

===== code#12561 draft=True mergeable=UNKNOWN state=UNKNOWN files=3 +379/-0 created=2026-11-22T13:35:39Z updated=2026-11-22T13:56:04Z
TITLE: deploy(pm-console): stage protected cf release
BODY: Part of #12521. This draft stages only the Worker deployment slice; it does not close the umbrella issue or include the report endpoint, APK, or Android SDK work.
Changes

The existing .github/workflows/pm-console-edge-deploy.yml is restored to match main exactly. Its current push-to-main Wrangler publisher and rollback input remain the production path.
Adds a separate workflow_dispatch-only cf release workflow, restricted to main, with upload-only and release stages. Neither stage runs on a push or pull request.
Runs a no-secret parity job first. It compares Wrangler and cf dry-run binding names and bundle size, and checks the v1 SQLite Durable Object mapping.
Upload uses the pm-console-edge-upload environment and only CF_PM_EDGE_UPLOAD_API_TOKEN / CF_PM_EDGE_UPLOAD_ACCOUNT_ID. It creates a candidate version tagged with the full source SHA and workflow run, then captures the current 100
COMMENT by coderabbitai at 2026-11-22T13:35:47Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/pm-console-edge-cf-release.yml, pm-stack/console/edge/cloudflare.config.ts, pm-stack/console/edge/wrangler.config.ts

===== code#12563 draft=True mergeable=UNKNOWN state=UNKNOWN files=3 +264/-1 created=2026-11-22T13:38:46Z updated=2026-11-22T13:38:54Z
TITLE: feat(bedside): add staged protected cf release workflow
BODY: Summary

Add a manual-only GitHub Actions release path for the existing Bedside & Away external Worker, using native cf Versions and the version pinned by pkgs/cf/default.nix.
Keep upload and production promotion in separate protected-environment runs. Upload reports its exact version ID and full source SHA but does not create a deployment. Promotion requires both values and checks the version ID, git-<sha12> tag, full-SHA message, and deployable status before creating a 100% deployment.
Preserve the existing manual native-cf operator fallback until Actions release and cutover evidence are verified. The workflow does not touch Sites, apply routes/triggers, or target preview/test workers.

Related: #9359, #9371.
Checks

Parsed the workflow YAML and confirmed it has only workflow_dispatch and separate upload/promotion environments.
bash -n passed for every workflow shell block.
Generated t
COMMENT by coderabbitai at 2026-11-22T13:38:53Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/bedside-and-away-release.yml, pkgs/bedside-and-away/design/release.md, pkgs/bedside-and-away/external/README.md

===== code#12566 draft=True mergeable=UNKNOWN state=UNKNOWN files=13 +2329/-125 created=2026-11-22T13:41:24Z updated=2026-11-22T13:41:31Z
TITLE: ci(decision-events): add cf release workflow (#9335)
BODY: Summary

Replace the worker's manual Wrangler deploy instructions with a protected, manual GitHub Actions release workflow using cf workers versions create and optional cf workers deployments create promotion.
Pin cf@1.0.0-rc.2, compatible Wrangler 4.163.0, and Node 24.22; add a cloudflare.config.ts matching the existing D1 and Durable Object bindings.
Add offline guards for Build Output parity, exact Worker/D1 target identity, deployable version selection, and preserve the current D1 ID, DO class/storage, and Wrangler v1 migration declaration.
Disable npm run deploy as a manual Wrangler path. The workflow does not run automatically on pushes and promotion defaults off.

Verification

npm ci
bun test (15 passing)
npm run typecheck
npm run test:release
cf workers versions create --dry-run with Node 24.22, followed by npm run check:build-output
actionlint via .github/scripts/check-workflow
COMMENT by coderabbitai at 2026-11-22T13:41:31Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/decision-events-worker-release.yml, pm-stack/decision-events-worker/.gitignore, pm-stack/decision-events-worker/README.md, pm-stack/decision-events-worker/bun.lock, pm-stack/decision-events-worker/cloudflare.config.ts, pm-stack/decision-events-worker/package-lock.json, pm-stack/decision-events-worker/package.json, pm-stack/decision-events-worker/scripts/check-build-output.mjs, pm-stack/decision-events-worker/scripts/check-target.mjs, pm-stack/decision-events-worker/scripts/reso

===== code#12567 draft=True mergeable=UNKNOWN state=UNKNOWN files=8 +2163/-6 created=2026-11-22T13:42:02Z updated=2026-11-22T13:42:08Z
TITLE: github-backup: add protected cf Versions release workflow
BODY: Summary
Adds an opt-in GitHub Actions release path for the existing developing-today-backup Worker while leaving the operator deploy tool unchanged.

upload uses cf workers versions create and stops before traffic changes.
A separate promote dispatch requires the full version UUID, checks the two required secret bindings and the existing GHB_EVENTS KV namespace ID, then promotes exactly that UUID at 100%. It verifies the latest deployment and /health afterward.
The workflow runs only by manual dispatch from main, uses a protected GitHub environment, grants contents: read, and pins checkout/setup-node by SHA plus Node/npm, cf, and Wrangler versions.
Adds the cf TypeScript config generated by cf migrate; the existing Worker name, compatibility date, source entrypoint, and KV namespace are retained. The existing wrangler.toml, deploy.mjs, and deploy.sh remain unchanged.

cf@1.0.0-rc.1 was r
COMMENT by coderabbitai at 2026-11-22T13:42:08Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/github-backup-tests.yml, .github/workflows/github-backup-worker-release.yml, infra/github-backup/docs/RUNBOOK.md, infra/github-backup/worker/cloudflare.config.ts, infra/github-backup/worker/package-lock.json, infra/github-backup/worker/package.json, infra/github-backup/worker/release-version.mjs, infra/github-backup/worker/test/release-version.test.mjs

===== code#12568 draft=True mergeable=UNKNOWN state=UNKNOWN files=6 +705/-504 created=2026-11-22T13:42:11Z updated=2026-11-22T13:42:39Z
TITLE: Add protected GitHub Actions delivery for Atlas Search Worker
BODY: Summary

Add separate, manually dispatched upload and promotion workflows for the already-deployed Atlas Search Worker.
Upload uses cf workers versions create and prints the new version ID without changing traffic.
Promotion requires a specific version ID, checks Cloudflare's deployable-version set, creates the deployment at 100%, reads back the active split, and verifies the existing /health response.
Add cloudflare.config.ts for the current Worker name, workers.dev route, SERVICE_BASE, SQLite QueryCache, and existing secret bindings. Keep wrangler.toml for local dev and Vitest.
Disable the old npm run deploy Wrangler path so production changes go through the explicit GitHub Actions flow.

The workflows only run by workflow_dispatch on main, use atlas-search-production, and never read laptop SOPS or Bitwarden stores. Upload supplies the existing Worker secrets from GitHub environment se
COMMENT by coderabbitai at 2026-11-22T13:42:39Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/atlas-search-worker-promote.yml, .github/workflows/atlas-search-worker-upload.yml, pkgs/atlas-search/worker/README.md, pkgs/atlas-search/worker/cloudflare.config.ts, pkgs/atlas-search/worker/package-lock.json, pkgs/atlas-search/worker/package.json

===== code#12569 draft=True mergeable=UNKNOWN state=UNKNOWN files=10 +650/-304 created=2026-11-22T13:42:25Z updated=2026-11-22T14:11:06Z
TITLE: ci(cold-store-rehydrator): upload with cf, gate promotion on private staging
BODY: Scope
Port only the cold-store-rehydrator Worker deployment from Wrangler to GitHub Actions using the repo-pinned cf CLI and Workers Versions API. The broader #5767 issue originally kept Actions out of scope; this request changes that for this Worker only. It does not modify R2 objects, retention, D1/DO storage, routes, or shared deploy infrastructure.
Ownership check: #12569 is the only open PR matching this Worker; the other open Cloudflare migration PRs target different Workers. Historical #5767 is the broad deployment-tool issue, not a competing implementation of this Worker.
Pre-production safety

Cloudflare says Version URLs are not generated for Durable Object Workers. The upload workflow no longer assigns a public preview alias or probes a Version URL. It creates an immutable version only and confirms the serving deployment did not change.
There is no supported isolated private s
COMMENT by coderabbitai at 2026-11-22T13:42:35Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/cold-store-rehydrator-promote.yml, .github/workflows/cold-store-rehydrator-upload.yml, lib/default.nix, pkgs/cold-store-reaper/worker/README.md, pkgs/cold-store-reaper/worker/cloudflare.config.ts, pkgs/cold-store-reaper/worker/deploy.mjs, pkgs/cold-store-reaper/worker/deploy.nix, pkgs/cold-store-reaper/worker/deploy.sh, pkgs/cold-store-reaper/worker/package.json, pkgs/cold-store-reaper/worker/test/config-parity.test.mjs

===== code#12576 draft=True mergeable=UNKNOWN state=UNKNOWN files=24 +2808/-37 created=2026-11-22T14:14:44Z updated=2026-11-22T14:58:09Z
TITLE: ci(lab-ops-hub): release Worker versions through cf
BODY: Summary

Add separately dispatched cf workers versions create upload and cf workers deployments create promotion workflows, pinned to Node 24.22.0, cf@1.0.0-rc.1, and Wrangler 4.163.0.
Promotion accepts only a numeric upload workflow run ID. It downloads the receipt from that run, checks the exact successful trusted main/workflow/run attempt/source SHA, checks the receipt's Worker/version/tag and the single-version 100% serving baseline, rereads that baseline immediately before promotion, then verifies the resulting 100% deployment.
Use the protected environment lab-ops-hub-cloudflare-release with uniquely named environment variable CLOUDFLARE_LAB_OPS_HUB_ACCOUNT_ID and secret CLOUDFLARE_LAB_OPS_HUB_API_TOKEN. The token needs the Cloudflare Workers Editor role scoped to the individual lab-ops-hub Worker; it is never supplied to pull-request jobs. No GitHub settings or token were changed/
COMMENT by coderabbitai at 2026-11-22T14:14:51Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: .github/workflows/container-images.yml, .github/workflows/lab-ops-hub-promote.yml, .github/workflows/lab-ops-hub-worker-release.yml, .github/workflows/lab-ops-hub.yml, lib/default.nix, pkgs/deploy/default.nix, pkgs/deploy/services.nix, pkgs/lab-ops-hub/.gitignore, pkgs/lab-ops-hub/README.md, pkgs/lab-ops-hub/cloudflare.config.ts, pkgs/lab-ops-hub/container-image.nix, pkgs/lab-ops-hub/deploy.mjs, pkgs/lab-ops-hub/deploy.nix, pkgs/lab-ops-hub/deploy.sh, pkgs/lab-ops-hub/image-push.mjs, pkgs/lab-op

===== code#12605 draft=False mergeable=UNKNOWN state=UNKNOWN files=2 +783/-484 created=2026-11-22T15:23:53Z updated=2026-11-23T06:15:43Z
TITLE: Reuse ID build dependencies and avoid duplicate checks
BODY: What changed

Pass the root flake's pinned Crane input into the ID outputs. NixOS runtime packages build Cargo dependencies through a separate buildDepsOnly derivation, then reuse those artifacts for the ID package.
Keep runtime native libraries to OpenSSL; native-shell checks retain their GUI libraries.
Give ID checks only the setup they use: Cargo, Bun dependencies/assets, and the Roc WASM host are no longer prepared for unrelated formatting, web-only, or shell checks.
Give the NixOS integration and Playwright check derivations focused native inputs.
Turn the ID default output into a link farm over the named checks in just ci. The full flake check also builds named checks, so it now reuses those results rather than running the aggregate CI recipe a second time. Debug and release builds remain in the ID default check without adding extra root-flake checks.
Turn test, test-int, and test-
COMMENT by chatgpt-codex-connector at 2026-11-22T15:24:02Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T06:15:42.401612Z
234db53
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
COMMENT by coderabbitai at 2026-11-22T15:24:12Z: Walkthrough



The ID flake now receives an optional Crane input and can use separate Cargo dependency artifacts for runtime builds. Check derivations select setup stages by check type, and integration-test and E2E derivations declare native tools explicitly.
Changes: ID flake builds and checks



Layer / File(s)
Summary




Crane runtime build support  lib/default.nix, pkgs/id/flake.nix
The root flake passes Crane to the ID flake. The ID flake creates a Crane library and adds a helper that buil
FILES: lib/default.nix, pkgs/id/flake.nix

===== code#12627 draft=False mergeable=UNKNOWN state=UNKNOWN files=8 +88/-27 created=2026-11-22T16:33:14Z updated=2026-11-23T05:56:20Z
TITLE: atlas(worker): key hosted search cache by disclosure context
BODY: Summary
POST /search
  pins context = hosted-per-user
  cache key = query + context + existing config
  forwards context to atlas-search service


Bump the Worker cache-key schema and include disclosure context in its canonical hash material.
Thread the hosted-per-user context through the Durable Object and search service request.
Document the boundary and cover explicit forwarding plus cross-context key separation.

Closes #12622
Evidence

Before: no disclosure context in the Worker cache key or forwarded search request.
After: npm test — 4 files, 24 tests passed; npm run typecheck passed; git diff --check passed.

Merge Danger
Door: Two-way.
Blast Radius: Worker-only. Cache key schema changes invalidate reuse of existing Durable Object entries; subsequent searches repopulate them.



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.




Summary by CodeR
COMMENT by chatgpt-codex-connector at 2026-11-22T16:33:23Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T18:38:38.837162Z
60b6bd9
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
COMMENT by coderabbitai at 2026-11-22T16:33:49Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 1 minute.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository app
COMMENT by dezren39 at 2026-11-22T18:37:05Z: Codex review round answered (2026-11-22 ~12:40 CST): the one open thread — property-based coverage for disclosure-context key isolation — is fixed in 60b6bd9 and the thread is resolved. The new seeded generated test covers 64 (query, depth) pairs across all four disclosure contexts with the key digest as the oracle; the fixed two-context example stays as smoke coverage. Verified by executing the test file under Node in this seat (npm/vitest install fails here, so CI is the first real vitest run)
FILES: pkgs/atlas-search/worker/README.md, pkgs/atlas-search/worker/src/cache_do.ts, pkgs/atlas-search/worker/src/index.ts, pkgs/atlas-search/worker/src/key.ts, pkgs/atlas-search/worker/src/service.ts, pkgs/atlas-search/worker/src/space.ts, pkgs/atlas-search/worker/test/key.test.ts, pkgs/atlas-search/worker/test/worker.test.ts

===== code#12662 draft=False mergeable=UNKNOWN state=UNKNOWN files=7 +102/-5 created=2026-11-22T19:59:58Z updated=2026-11-23T05:56:23Z
TITLE: feat(github-backup): schedule plain mirror sync
BODY: Summary

Add a daily systemd service/timer that syncs all four planned plain Git mirrors and reports failure if any sync fails.
Add strict argument and all-repositories-attempted tests; document install/operation and register the schedule.
Sync all four mirrors and clone-drill each destination against its source mirror HEAD.

Verification

All 21 standalone infra/github-backup/tests/test_*.py files passed.
All four sync runs passed (326/55/605/1135 refs at final run); clone drills matched source default-branch HEAD for all four repos.
systemd-analyze --user verify, bash -n, py_compile, and git diff --check passed. (systemd emitted an unrelated existing OnBootUSec warning for pm-host-health.timer.)

Refs #8584. This does not complete #8584: D1 restore, Cloudflare rehost, unified freshness/lag status, and automatic lag issue filing remain.



Open in [code]smith · Need help on this PR? Tag
COMMENT by chatgpt-codex-connector at 2026-11-22T20:00:32Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T20:02:39.533140Z
cda55ce
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
COMMENT by coderabbitai at 2026-11-22T20:00:33Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 51 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
FILES: doc/fleet-schedule-registry.md, infra/github-backup/bin/plain_mirror_all.py, infra/github-backup/docs/RUNBOOK.md, infra/github-backup/systemd/github-backup-mirror.service, infra/github-backup/systemd/github-backup-mirror.timer, infra/github-backup/systemd/install-units.sh, infra/github-backup/tests/test_plain_mirror_all.py

===== code#12663 draft=False mergeable=UNKNOWN state=UNKNOWN files=1 +0/-4 created=2026-11-22T20:10:30Z updated=2026-11-23T05:56:18Z
TITLE: docs(roadmap): program-tending drift fix (landed forward items)
BODY: Program-tending roadmap drift fix (tracking: #1352). Each removed item is a forward-looking roadmap entry whose referenced issues are all closed; per ROADMAP.md, completed work is recorded in GitHub, not in a Done row. No Done rows were edited.

planning/roadmaps/id.md line 15: | Validate the delivered work                         | in-progress | Phases 9 and 10 were delivered faster than they were verified. The remaining verification-debt inventory gets a proving test or it (refs: #796:closed)
planning/roadmaps/id.md line 17: | Fair scheduling and quotas                          | planned     | A world that spins on fuel cannot raise another world's p99 latency by more than 10 percent; per-account quotas for worlds, storag (refs: #1067:closed)
planning/roadmaps/id.md line 19: | First new providers                                 | planned     | A bounded key-value store, content-address
COMMENT by coderabbitai at 2026-11-22T20:10:38Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 52 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-22T20:10:39Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T20:13:01.025785Z
80c138b
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: planning/roadmaps/id.md

===== code#12666 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +992/-0 created=2026-11-22T20:16:51Z updated=2026-11-23T05:56:25Z
TITLE: feat(route-audit): check pool ordering — OpenAI final reserve (#1173)
BODY: Summary
Closes nothing yet — this is the next slice of #1173 (daily model + route audit). It automates the one pool-ordering rule that is decidable from the tracked config alone: OpenAI as final reserve. New checker bin/check-pool-ordering.mjs resolves each tier's effective pool order (reusing inspectRouteRules) and verifies that, in every ordered chain, an OpenAI-backed pool comes last, with fallback targets checked as separate passes (a tier's regular chain runs first and completely; each fallback is a second pass). Weighted/round-robin mixing is reported as a note (no reserve encoding is visible there), never a finding. Fixtures + the canonical-config check are wired into opencode-config-guard.
Rule reading decided in this slice (decision record on #1173): "final reserve" applies to ordered chains; in weighted/round-robin routes list position is not burn order, so mixing there is unkn
COMMENT by coderabbitai at 2026-11-22T20:16:58Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 47 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-22T20:17:00Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T00:12:28.644317Z
8ce87d8
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
FILES: .github/workflows/opencode-config-guard.yml, bin/check-pool-ordering.mjs, bin/check-pool-ordering.test.mjs

===== code#12682 draft=False mergeable=UNKNOWN state=UNKNOWN files=2 +149/-25 created=2026-11-22T21:07:30Z updated=2026-11-23T05:56:14Z
TITLE: feat(fleet): split failed terminal events out of health totals (#1233)
BODY: Summary
fleet queue health lumped failed terminal events into terminal_events_24h/7d together with completed ones, so epic #1233's criterion "zero dead-in-queue tasks across all four queues over a rolling 7 days" could not be read off health: repo-health-code-pilot showed 2846 terminal events in 7d with no way to tell whether any were failures. This slice adds failed_events_24h / failed_events_7d per queue, computed by failed-only sums in the existing terminal-events query and folded by a new apply_terminal_events (same shape as the #6528 at-cap and #8276 dep-blocked folds). The health table renders new keys with no client change (cmd_health reads columns from the first row).
After this PR, fleet queue health shows failed_events_7d per queue; 0 means the criterion holds over the window. The live broker serves the new columns once this lands and deploys; the instrument, not the measuremen
COMMENT by coderabbitai at 2026-11-22T21:07:38Z: Walkthrough



Queue health entries now include 24-hour and 7-day failed-event counts. The health query counts lease-expiry-at-cap events as terminal failures and applies failure classification to failed events. Tests cover initialization, aggregation, and classification.
Changes: Queue health metrics



Layer / File(s)
Summary




Calculate and fold terminal-event metrics  pkgs/fleet/crates/fleet-queue/src/turso_queue/admin.rs, pkgs/fleet/crates/fleet-queue/src/turso_queue.rs
Health entries ini
COMMENT by chatgpt-codex-connector at 2026-11-22T21:07:47Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-22T23:57:59.317340Z
f68e5df
New commits



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is r
FILES: pkgs/fleet/crates/fleet-queue/src/turso_queue.rs, pkgs/fleet/crates/fleet-queue/src/turso_queue/admin.rs

===== code#12726 draft=True mergeable=UNKNOWN state=UNKNOWN files=4 +771/-0 created=2026-11-23T00:11:12Z updated=2026-11-23T00:11:20Z
TITLE: WIP: Atlas cold-tier shard writer (exporter) — #8227
BODY: Partial implementation of issue #8227 (Atlas cold tier shard writer / exporter), follow-on 1 of 5 from #6042 design.
Implements the §2 container layout, §3 manifest schema, §4 packing validation, optional whole-section zstd span codec, reader (unpack) and reconstruction helpers (reconstruct_version, read_vector) so the writer's own suite wires the §7 cold-roundtrip-verify shape.
Tests: 6 passing (roundtrip none + zstd, checksum corruption, idempotent re-pack, packing rejections).
Remaining for full acceptance: lineage grouping enforcement, proptest coverage, integration with standalone verifier (#8230) once it lands. Parked for capacity; resume recipe in issue comment.
Refs #8227



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T00:11:20Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/atlas-search/Cargo.lock, pkgs/atlas-search/crates/atlas-core/Cargo.toml, pkgs/atlas-search/crates/atlas-core/src/cold_shard.rs, pkgs/atlas-search/crates/atlas-core/src/lib.rs

===== code#12732 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +182/-49 created=2026-11-23T00:17:53Z updated=2026-11-23T05:56:10Z
TITLE: pkgs/id: split dependency compilation out of the Nix build via crane
BODY: Closes #8574
Summary

pkgs/id/flake.nix: pins crane (same rev as the code repo root lock) and adds craneLib plus an id-crane package.
pkgs/id/default.nix: two-tier build when craneLib is passed; monolithic fallback otherwise.

Verification

Not built locally: the foreground nix build .#id-crane exceeded the lane's time budget. CI is the verification path for this PR (see .github/ci-policy.md).
Mirrors the proven pkgs/atlas-search crane wiring (#8560).




Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T00:18:00Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 7 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository ap
FILES: pkgs/id/default.nix, pkgs/id/flake.lock, pkgs/id/flake.nix

===== code#12740 draft=False mergeable=UNKNOWN state=UNKNOWN files=1 +20/-1 created=2026-11-23T00:39:54Z updated=2026-11-23T05:56:21Z
TITLE: ci(id): record Nix runner settings and check duration
BODY: Scope
Add low-overhead Nix runner telemetry (configuration, cores, disk) and elapsed check time to the existing ID nightly. Preserve 300-minute job cap, 270-minute step cap, targeted checks, log artifacts, cache and watchdog cleanup.
Validation plan (requires separate dispatch)
Run id e2e nightly manually on an approved ref with only_checks=nixos-serve, after reconciling #12605 and #12732 and resolving #12605's missing Biome input. Existing 5400s targeted budget; job cap 300m. Expected standard hosted 90-minute compute ~$0.54 at illustrative $0.006/min; maximum five-hour cap ~$1.80, excluding other costs. Compare build/substitution logs and VM test duration. Do not interpret one run as proof of warm-cache gains.
Coordination
No Crane or Nix build changes here; #12605 and #12732 already cover dependency reuse. No CI dispatched. Existing manual/nightly trigger policy retained.
Checks
Revie
COMMENT by coderabbitai at 2026-11-23T00:40:01Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 16 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
FILES: .github/workflows/id-e2e-nightly.yml

===== code#12741 draft=True mergeable=UNKNOWN state=UNKNOWN files=2 +123/-7 created=2026-11-23T00:41:30Z updated=2026-11-23T00:41:37Z
TITLE: fix(pm-dispatch): recover timed-out queue claims
BODY: Summary

Give the dispatcher's three-write /claim-flow a bounded 30-second timeout.
When the response is lost, read the exact task and recover only if it is still claimed by this launch's session; reapply lane/bind and confirm the binding before continuing.
Keep a different or unclaimed task on the existing failure-compensation path.

Evidence
Task 21041 for #12536 recorded the claim at 12:33:49Z and the session bind at 12:34:00Z, followed by cancellation at 12:34:02Z after the client reported a timeout. This was a committed claim whose response arrived too late for dispatch's 15-second outbound deadline.
Verification

bun test test/queue.test.ts — 18 passed.
git diff --check — clean.
Package typecheck not run; no local tsc binary is installed.

Related to #12536.



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T00:41:37Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pm-stack/dispatch/src/queue.ts, pm-stack/dispatch/test/queue.test.ts

===== code#12743 draft=True mergeable=UNKNOWN state=UNKNOWN files=3 +84/-43 created=2026-11-23T00:50:45Z updated=2026-11-23T00:50:52Z
TITLE: fix(queue): make coverage merge-watch packets validator-safe
BODY: Summary

Make merge-watch packets pass the current queue validator: include its required payload.prs reference and use the canonical per-PR dedupe key shared with the warden.
Keep the documented pr_shepherd attempt default so validation is clean.
Preserve every open PR that references an uncovered issue, filing one watch task per PR instead of keeping only the first.

Evidence
A live dry-run of the existing merge-watch packet for #12233/#12565 was rejected by the queue validator: the custom merge-watch key was unrecognized, then the canonical key required payload.prs. The plan issue #12233 currently has three uncovered open slice PRs; the old issue-to-PR map retained only one.
Verification

python3 -I scripts/fleet/queue-reconcile/test_check_coverage.py — 20 passed.
Live fleet queue enqueue /dev/stdin --dry-run accepted triage and repaired merge-watch packets.

Related to #12280 and #495
COMMENT by coderabbitai at 2026-11-23T00:50:52Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: scripts/fleet/queue-reconcile/README.md, scripts/fleet/queue-reconcile/check_coverage.py, scripts/fleet/queue-reconcile/test_check_coverage.py

===== code#12756 draft=False mergeable=UNKNOWN state=UNKNOWN files=4 +68/-15 created=2026-11-23T01:24:27Z updated=2026-11-23T05:56:06Z
TITLE: feat(fleet-queue): served-path mode lines on remaining lifecycle verbs (#8581)
BODY: Queue task 21043 · Refs #12538 (tracking) · Refs #8581, slice of parent design #8575.
#8581 acceptance remainder: a mode line on every verb's output. #12706 wired claim/heartbeat/checkpoint served paths plus the fallback-switch envelope. This PR wires the remaining task-row lifecycle verbs' served paths through the new served_row helper (normal/cloud on a healthy store, selected through route_for): release, lane, bind, wake, block, unblock, dep-recheck, quarantine-recheck, cancel, revive, fail, complete, and park/fail/complete's row_or_ledger outputs (line rides on the row, so .task.mode_line when a lane ledger wraps it).
Out of scope (recorded remainder): health text table, signal local-generation output, outbox envelope passthroughs, query verbs in commands_query.rs, per-leg live drills, lane-runbook cleanup.
Verification: cargo test -p fleet-queue --features queue-broker --lib — 481 p
COMMENT by coderabbitai at 2026-11-23T01:24:35Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 32 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
FILES: pkgs/fleet/crates/fleet-queue/src/client.rs, pkgs/fleet/crates/fleet-queue/src/client/commands_lifecycle.rs, pkgs/fleet/crates/fleet-queue/src/client/mode_report.rs, pkgs/fleet/crates/fleet-queue/src/client/transport.rs

===== code#12814 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +450/-8 created=2026-11-23T03:11:01Z updated=2026-11-23T05:56:01Z
TITLE: feat(id): PWA snapshot store + offline read restore in the browser shell (#10927 slice 1)
BODY: Summary
First slice of #10927 (shells v1 slice 1 — PWA offline read model + catch-up). The browser shell (pkgs/id/web/src/world.ts) held all session state in memory: a reload lost the world entirely, and the reconnect cursor restarted at 0.

New world-snapshot-store.ts: a snapshot store keyed per host + world name, holding {world_id, last applied sequence, participants, rendered view (screen surface or text), view revision, records}. Entries are validated on load with the frame validators' shape discipline (safe non-negative integer sequences/ids, string fields, a surface the renderer accepts); a corrupt or foreign entry is ignored, never rendered and never adopted as the resume cursor. Storage failures are swallowed — the store is best-effort and never breaks the live session.
world.ts writes the store through on every snapshot / event / view (full, patched, form) / completed-records fr
COMMENT by coderabbitai at 2026-11-23T03:11:09Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 5 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository ap
FILES: pkgs/id/web/src/world-offline.test.ts, pkgs/id/web/src/world-snapshot-store.ts, pkgs/id/web/src/world.ts

===== code#12840 draft=False mergeable=UNKNOWN state=UNKNOWN files=1 +8/-0 created=2026-11-23T03:41:59Z updated=2026-11-23T05:55:45Z
TITLE: docs(fleet): register fleet-work-loop-hourly 1h Muse cron
BODY: Refs #4988 (the unified schedule-registry requirement).
The hourly fleet work loop is live as a Muse cron job but was never recorded in the registry — this PR closes that gap (maintenance rule, #4988):

Job id: fleet-work-loop-hourly, owner goal:developing-today-code-issue-resolution, cadence interval@1h, created 2026-11-22.
Every hour (America/Chicago) the xaltha seat checks the fleet legs (repairing via init-ensure if down), checks in-flight work and open PRs, takes the next item via fleet agent next, refills the shortlist below 4 entries, works blockers rather than re-triaging, and reports briefly to the main chat.
Definition file: /home/hatch/workspace/goals/developing-today-code-issue-resolution/crons/hourly/fleet-work-loop-hourly__interval@1h.md.

This PR adds the job's row to the Surface 1 registry (mirroring the openclaw-gateway-death-watch row format from #12705) plus a Surface 
COMMENT by coderabbitai at 2026-11-23T03:42:07Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 48 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
FILES: doc/fleet-schedule-registry.md

===== code#12943 draft=False mergeable=UNKNOWN state=UNKNOWN files=4 +4/-0 created=2026-11-23T05:37:45Z updated=2026-11-23T23:53:41Z
TITLE: chore(mcpx): collected run seeds [skip ci]
BODY: Seed records collected by the mcpx-seed-collector workflow (#1953) from test-seed run artifacts (#1949). One file per seed under pkgs/mcpx/testdata/seeds/v1; the seedstore guard test validates every record. Data only — no code changes.



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T05:37:52Z: Important
Review skipped
Bot user detected.
To trigger a single review, invoke the @coderabbitai review command.

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository applied
Commit: Unavailable
Layers: defaults
Review profile: CHILL
Plan: Advanced
Run ID: d9106ab8-fef0-44f4-923a-fce9819e9aa1




You can disable this status message by setting the reviews.review_status to false in the CodeRabbit configuration file.
U
FILES: pkgs/mcpx/testdata/seeds/v1/2026/11/01a2030d-92a0-7c11-98ac-20d2f2e0fe96.json, pkgs/mcpx/testdata/seeds/v1/2026/11/01a20887-0cc2-7cd3-ab93-223dcc0b5c72.json, pkgs/mcpx/testdata/seeds/v1/2026/11/01a2094a-eb83-7e2d-aeec-266f0b5bba5e.json, pkgs/mcpx/testdata/seeds/v1/2026/11/01a20a10-1b2e-721f-9c04-4e7689e4c465.json

===== code#12963 draft=True mergeable=UNKNOWN state=UNKNOWN files=1 +8/-0 created=2026-11-23T05:57:44Z updated=2026-11-23T05:57:51Z
TITLE: cold-store: record dump size and checksum for atlas diagnostics
BODY: Adds non-sensitive dump byte count and SHA-256 to export logs to correlate atlas reconstruction failures with downloaded artifacts. Existing repair/dedupe/vector-index diagnostics are retained. No change to backup data or export ordering. Not run against live backups; validation and deployment pending laptop access. Related #12000 and #12112.



Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T05:57:51Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pkgs/cold-store-reaper/scripts/export-turso.sh

===== code#12980 draft=True mergeable=UNKNOWN state=UNKNOWN files=6 +331/-6 created=2026-11-23T06:12:21Z updated=2026-11-23T06:12:28Z
TITLE: Add opt-in local counters for Turso store write churn
BODY: Scope

Follow #12964 with internal, default-off measurement of the existing Hrana execute reply. The running service constructs no meter.
Four fixed operation labels only; track affected/read/written coverage and count skipped candidates only where the affected count is valid. No additional requests, SQL, retries, data logging, network sink, routes, or scheduler.
Totals cover observed successes only and are not an invoice or a fleet-wide estimate.

Verification

Existing-dependency offline tsc --noEmit; 32 Bun tests in the transport/store suites. Fixtures cover request/response equivalence, malformed and missing values, diagnostic failure, zero versus unknown, and exact operation/chunk attribution.
No tests against live Turso, no live activation, and no change to deploy settings.




Open in [code]smith · Need help on this PR? Tag @codesmith-bot with what you need.
COMMENT by coderabbitai at 2026-11-23T06:12:28Z: Important
Draft PR not reviewed
Draft PRs are not automatically reviewed by default.

  Trigger a manual review

To automatically review draft PRs, update your CodeRabbit configuration:
reviews:
  auto_review:
    drafts: true




  Autofix · Keep fixing CodeRabbit findings and required CI, and resolving merge conflicts



Thanks for using CodeRabbit! It's free for OSS, and your support helps us grow. If you like it, consider giving us a shout-out.

Share




X
Mastodon
Reddit
LinkedIn
FILES: pm-stack/aggregator/README.md, pm-stack/aggregator/src/remote-stores.ts, pm-stack/aggregator/src/sql-client.ts, pm-stack/aggregator/src/turso-stores.ts, pm-stack/aggregator/src/turso-write-metrics.ts, pm-stack/aggregator/test/turso-write-metrics.test.ts

===== code#13037 draft=False mergeable=UNKNOWN state=UNKNOWN files=4 +316/-3 created=2026-11-23T07:24:33Z updated=2026-11-23T07:26:43Z
TITLE: id: model fenced migration transitions
BODY: Summary
beginMigration(directory, request)
  keep source active; create pending state at epoch E+1

transitionMigration(state, event)
  freeze acknowledgement -> verified snapshot -> switch at E+2 -> activate target
  pre-switch abort -> source at fresh epoch
  post-switch abort -> rejected

Adds a pure transition model and invariant tests. It does not persist state, perform CAS/lease/fencing, or enable production migration; those remain gated on the directory, routing, storage, and hosted mutation interfaces.
Evidence

Before: migration transition model/tests were absent; hosted-worker test registry also omitted several tests already run by npm test.
After: npm test — 100 passed; npm run typecheck — passed; registry JSON and test-path parity check — passed; git diff --check — passed.

Merge Danger
Door: Two-way
Pure state model with no runtime integration or production side effects.
Bla
COMMENT by coderabbitai at 2026-11-23T07:24:40Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 30 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T07:24:47Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T07:26:42.149310Z
f9910e2
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: .github/test-suite-registry.json, pkgs/id/hosted-worker/package.json, pkgs/id/hosted-worker/src/migration-control.ts, pkgs/id/hosted-worker/test/migration-control.test.ts

===== code#13042 draft=False mergeable=UNKNOWN state=UNKNOWN files=6 +532/-3 created=2026-11-23T07:34:47Z updated=2026-11-23T07:37:36Z
TITLE: feat(id): tenancy schema + migration baseline (E1 of #9083)
BODY: Closes #12796
E1 of #9083 (component E, hosted tenancy): the central tenancy DB schema from design §2.1 (doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture) lands as a migration baseline in the new control-plane schema home, pkgs/id/hosted-worker/tenancy/.
What

tenancy/migrations/0001_tenancy_baseline.sql — the §2.1 schema as a Turso/SQLite migration: account (identity registry), binding (account→identity/tenant pointer), tenant_db (database registry), role_grant (#1253 vocabulary), peer_policy (#9106/B31 classes as configuration, not a parallel trust model), publish_record, credential (metadata only), plus a meta table carrying schema_version (the user-do.ts bootstrap pattern).
tenancy/README.md — the plane rules, the invariant, and the migration discipline (numbered, append-only; E2/E4/E5 add files here).
test/tenancy-schema.test.ts — guard suite (below), wired into npm test 
COMMENT by coderabbitai at 2026-11-23T07:34:55Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 18 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T07:34:59Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T07:37:35.300762Z
511c3a2
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
COMMENT by dezren39 at 2026-11-23T07:36:46Z: Main advanced under this PR (bridge suites from #3818 and the provisioning gate from #3403 both edited the hosted-worker test line), which made this PR conflict in package.json only. Resolved by merging origin/main (d0d599ed3): the test line now carries every suite from main plus test/tenancy-schema.test.ts. Full merged suite re-run locally: 102/102 pass, tsc --noEmit clean.
Registry observation for the #3818 / #3403 lanes (not fixed here, out of this PR's scope): test/provisioning.test.ts and t
FILES: .github/test-suite-registry.json, pkgs/id/hosted-worker/README.md, pkgs/id/hosted-worker/package.json, pkgs/id/hosted-worker/tenancy/README.md, pkgs/id/hosted-worker/tenancy/migrations/0001_tenancy_baseline.sql, pkgs/id/hosted-worker/test/tenancy-schema.test.ts

===== code#13047 draft=False mergeable=UNKNOWN state=UNKNOWN files=29 +0/-23451 created=2026-11-23T07:36:53Z updated=2026-11-23T07:39:15Z
TITLE: chore(repo-root): remove stale slice-1 status files and committed session transcript (#12136)
BODY: Refs #12136 — the removal slice. The issue's third fix bullet (a guard check rejecting new root *COMPLETE* / *SUMMARY* files) is not in this PR, so this does not close the issue.
What was removed (29 files, 23,451 lines, all at the repo root)
26 stale slice-1 status documents:
FINAL_SLICE1_COMPLETION_REPORT.md, IMPLEMENTATION_COMPLETE.md, IMPLEMENTATION_COMPLETE_SLICE1.md, IMPLEMENTATION_COMPLETE.txt, IMPLEMENTATION_STATUS.md, IMPLEMENTATION_SUMMARY.md, OPENCODE_CLAIMANT_SLICE1_COMPLETE.md, OPENCODE_CLAIMANT_SLICE1_SUMMARY.md, OPENCODE_SLICE1_COMPLETE.md, README_SLICE1_COMPLETION.md, SLICE1_COMPLETE.txt, SLICE1_COMPLETION_CONFIRMATION.md, SLICE_1_COMPLETION_REPORT.md, SLICE1_COMPLETION_REPORT.md, SLICE1_COMPLETION_SUMMARY.md, SLICE1_COMPLETION_SUMMARY.txt, SLICE1_FINAL_SUMMARY.md, slice1_final_verification.md, SLICE1_IMPLEMENTATION_COMPLETE.md, SLICE1_IMPLEMENTATION_CONFIRMATION.md, SLIC
COMMENT by coderabbitai at 2026-11-23T07:37:02Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 18 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T07:37:05Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T07:39:14.966888Z
bbee41f
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: FINAL_SLICE1_COMPLETION_REPORT.md, IMPLEMENTATION_COMPLETE.md, IMPLEMENTATION_COMPLETE.txt, IMPLEMENTATION_COMPLETE_SLICE1.md, IMPLEMENTATION_STATUS.md, IMPLEMENTATION_SUMMARY.md, OPENCODE_CLAIMANT_SLICE1_COMPLETE.md, OPENCODE_CLAIMANT_SLICE1_SUMMARY.md, OPENCODE_SLICE1_COMPLETE.md, README_SLICE1_COMPLETION.md, SLICE1_COMPLETE.txt, SLICE1_COMPLETION_CONFIRMATION.md, SLICE1_COMPLETION_REPORT.md, SLICE1_COMPLETION_SUMMARY.md, SLICE1_COMPLETION_SUMMARY.txt, SLICE1_FINAL_SUMMARY.md, SLICE1_IMPLEMENT

===== code#13060 draft=False mergeable=MERGEABLE state=CLEAN files=6 +210/-0 created=2026-11-23T07:43:40Z updated=2026-11-23T17:19:51Z
TITLE: feat(ci): guard against new root *COMPLETE* / *SUMMARY* status files (#12136)
BODY: Refs #12136 — the guard slice, stacked on PR #13047 (the removal slice). Lands after #13047: this PR's base is xaltha/12136-root-cleanup, and the guard is green only on a tree where the stale root files are gone. Once #13047 merges and this PR's base retargets to main, #12136's third fix bullet is done and the issue can close.
What this adds

.github/scripts/check-root-status-files.sh — fails, naming each offender, when a file at the repository root has a name containing COMPLETE or SUMMARY (case-insensitive). Root only: the same names inside a directory (e.g. the pm-stack/opencode-claimant/ copies) are untouched. Strict one-optional-positional argument contract per #3352.
.github/scripts/check-root-status-files.test.sh — contract tests in the check-fleet-lock.test.sh style (fixture roots in a temp dir).
.github/workflows/root-status-guard.yml — nightly + dispatch venue modeled on produc
COMMENT by coderabbitai at 2026-11-23T07:43:46Z: Important
Review skipped
Auto reviews are disabled on base/target branches other than the default branch.
Please check the settings in the CodeRabbit UI or the .coderabbit.yaml file in this repository. To trigger a single review, invoke the @coderabbitai review command.

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository applied
Commit: Unavailable
Layers: defaults
Review profile: CHILL
Plan: Advanced
Run ID: 5e8e
COMMENT by chatgpt-codex-connector at 2026-11-23T07:43:53Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T07:45:52.471535Z
3cf36fb
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
COMMENT by dezren39 at 2026-11-23T17:19:51Z: agent-note:v1
lane: lane-aaj (stale-PR warden shepherd, deferred merge-ready list)
outcome: not merged — two blockers

Stacked base still open: this PR's base is xaltha/12136-root-cleanup (PR #13047), which is still open and unmerged as of 2026-11-23T17:20Z. The PR body states the guard is green only on a tree where the removal slice has landed, and that it lands after #13047.
Unresolved review finding: the Codex review at head 3cf36fb reports unresolved findings, including a P1 on .github/workf
FILES: .github/ci-policy.md, .github/scripts/check-ci-policy-agreement.sh, .github/scripts/check-root-status-files.sh, .github/scripts/check-root-status-files.test.sh, .github/test-suite-registry.json, .github/workflows/root-status-guard.yml

===== code#13089 draft=False mergeable=UNKNOWN state=UNKNOWN files=18 +8895/-8527 created=2026-11-23T08:03:58Z updated=2026-11-23T08:06:27Z
TITLE: pm-stack: split render.ts (8,515 lines) into render-*.ts view modules (#13051)
BODY: What
Split pm-stack/aggregator/src/render.ts (8,515 lines) by view into ten
render-*.ts modules, each under the 2,500-line production limit, and remove
render.ts's grandfather entry from .github/scripts/check-production-line-limit.sh
in the same PR, per the guard's convention.
render.ts is now a barrel that re-exports every public name, so all existing
callers (server.ts, error-boundary.ts, the test suites) keep importing the
same names from ./render.ts unchanged. No behavior change: the split is a
mechanical move of contiguous regions; cross-module references became imports
and a handful of previously private shared helpers were export-promoted in
their home module. Module bodies verified verbatim against the original file
(line-multiset identical modulo export prefixes).
Module map



module
lines
contents




render-core.ts
1,461
shared primitives: esc/fmt/relTime, layout + nav + sear
COMMENT by coderabbitai at 2026-11-23T08:04:07Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 55 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T08:04:11Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:06:26.768432Z
fc08943
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: .github/scripts/check-production-line-limit.sh, pm-stack/aggregator/src/render-aa.ts, pm-stack/aggregator/src/render-alerts.ts, pm-stack/aggregator/src/render-attention.ts, pm-stack/aggregator/src/render-board.ts, pm-stack/aggregator/src/render-core.ts, pm-stack/aggregator/src/render-dns.ts, pm-stack/aggregator/src/render-program.ts, pm-stack/aggregator/src/render-quota.ts, pm-stack/aggregator/src/render-today.ts, pm-stack/aggregator/src/render-views.ts, pm-stack/aggregator/src/render.ts, pm-sta

===== code#13093 draft=False mergeable=UNKNOWN state=UNKNOWN files=9 +3573/-3558 created=2026-11-23T08:06:08Z updated=2026-11-23T08:08:17Z
TITLE: refactor(pm-aggregator): split server.ts (5,299 lines) under the production line limit (#13052)
BODY: Closes #13052
What
pm-stack/aggregator/src/server.ts was 5,299 lines and grandfathered in .github/scripts/check-production-line-limit.sh. This PR splits it along its existing seams (wiring, contexts, pages, routes) with no behaviour change, and removes the server.ts grandfather entry from the guard in the same PR, per the guard's convention.
Module map



Module
Lines
Contents




server.ts
1,813
Read routes, the dispatch, fetchHandler, bootstrap + timers


server-wiring.ts
412
Config, stores, source selection, consumers, source-side sinks, fail-soft facet fallbacks


server-contexts.ts
809
Cached page/API assemblies (board, roadmap, today, overview, attention, program) + health assembly, provenance priming, SLO summaries


server-pages.ts
716
Standalone token pages: Needs-Drew answer page (#8568), decision-set pages (#8766), PWA manifest


server-write-routes.ts
744
Report-record resolu
COMMENT by coderabbitai at 2026-11-23T08:06:17Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 53 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T08:06:19Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:08:17.059445Z
2f9a5b8
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: .github/scripts/check-production-line-limit.sh, pm-stack/aggregator/src/server-alert-routes.ts, pm-stack/aggregator/src/server-contexts.ts, pm-stack/aggregator/src/server-decision-routes.ts, pm-stack/aggregator/src/server-pages.ts, pm-stack/aggregator/src/server-respond.ts, pm-stack/aggregator/src/server-wiring.ts, pm-stack/aggregator/src/server-write-routes.ts, pm-stack/aggregator/src/server.ts

===== code#13102 draft=False mergeable=MERGEABLE state=CLEAN files=9 +1139/-3 created=2026-11-23T08:10:05Z updated=2026-11-23T17:19:53Z
TITLE: feat(pm-service): hosted read API parity — projects/item/roadmap/releases per #9060 §6 (component A slice 3, #12828)
BODY: Component A slice 3 (#12828): the hosted read API, served from the tenant DB in the #9060 §6 shapes.
Base: stacked on xaltha/12826-hosted-collector (slice 2, PR pending). The API reads pm_items.item_json, which is slice 2's ProjectPoolItem shape, and the genealogy derivation runs over slice 2's pool.ts types. Slice 1 alone defines the tables but not the pool semantics the API serves, so this branch bases on slice 2; it lands after #13054 (slice 1) and the slice-2 PR.
Endpoints (read-only; the store layer issues SELECTs only; no token is read, minted, or persisted on this path — E §2.3):

GET /api/projects.json — projects[]: every registry board with its latest snapshot audit, plus the merged pool as items. A board with no landed snapshot serves snapshot: null, audit: null — honest absence, never fabricated.
GET /api/item/<id>.json — one pool item by ref, with ancestors[] + release + the 
COMMENT by coderabbitai at 2026-11-23T08:10:12Z: Important
Review skipped
Auto reviews are disabled on base/target branches other than the default branch.
Please check the settings in the CodeRabbit UI or the .coderabbit.yaml file in this repository. To trigger a single review, invoke the @coderabbitai review command.

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository applied
Commit: Unavailable
Layers: defaults
Review profile: CHILL
Plan: Advanced
Run ID: 09c1
COMMENT by chatgpt-codex-connector at 2026-11-23T08:10:18Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:16:30.370156Z
5ddfc6e
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
COMMENT by dezren39 at 2026-11-23T17:19:53Z: agent-note:v1
lane: lane-aaj (stale-PR warden shepherd, deferred merge-ready list)
outcome: not merged — stacked base still open
This PR's base is the branch xaltha/12826-hosted-collector (component A slice 2), not main. Per the PR body, the slice-2 PR is still pending, and slice 1 is PR #13054, which is still open and unmerged as of 2026-11-23T17:20Z. The API in this PR reads the pool shape that slice 2 defines, so it cannot land ahead of its stack. The warden's merge-ready tag reflects mergeab
FILES: .github/test-suite-registry.json, .github/workflows/pm-stack-tests.yml, pm-stack/pm-service/README.md, pm-stack/pm-service/package.json, pm-stack/pm-service/src/read-api/api.ts, pm-stack/pm-service/src/read-api/genealogy.ts, pm-stack/pm-service/src/read-api/releases.ts, pm-stack/pm-service/src/read-api/store.ts, pm-stack/pm-service/test/read-api.test.ts

===== code#13111 draft=False mergeable=UNKNOWN state=UNKNOWN files=12 +482/-2 created=2026-11-23T08:16:39Z updated=2026-11-23T08:18:39Z
TITLE: id(tui-shell): record the TUI capability statement (shells v2 slice 2, #12818)
BODY: Refs #12818.
The slice
#12818's first deliverable is the TUI's capability statement (decision D3 in the v2 outline, doc/2026-11-23T03-17-14Z_design_more_shells_v2/, §2): the claimed subset recorded before implementation starts. This PR lands only that deliverable; feature completion (the full text-native workflow, conformance on the recorded session, behavioral completion checks) remains open on #12818.
Following the pattern of the #10928 slice (PR #12884), in the hardware shell's style as the outline prescribes:

New crate pkgs/id/crates/id-tui-shell — a workspace member but not a default member, like the other shells — whose first recorded content is the statement. Later slices build the frame layer and presentation on it here.
capability-statement.json — sends chat, form_result, input, join, paste, records, resize; presents error, event, form_result_status, records, snapshot, view (er
COMMENT by coderabbitai at 2026-11-23T08:16:46Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 42 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T08:16:48Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:18:38.530331Z
7b58cdf
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: .github/scripts/id-pr-checks.txt, .github/test-suite-registry.json, pkgs/id/Cargo.lock, pkgs/id/Cargo.toml, pkgs/id/crates/id-tui-shell/Cargo.toml, pkgs/id/crates/id-tui-shell/README.md, pkgs/id/crates/id-tui-shell/capability-statement.json, pkgs/id/crates/id-tui-shell/src/lib.rs, pkgs/id/crates/id-tui-shell/tests/capability_statement.rs, pkgs/id/flake.nix, pkgs/id/just-recipes.json, pkgs/id/justfile

===== code#13113 draft=False mergeable=UNKNOWN state=UNKNOWN files=2 +61/-0 created=2026-11-23T08:23:45Z updated=2026-11-23T11:49:40Z
TITLE: fix(fleet-agent): map oc-* lanes to their family's registered worker
BODY: What
registered_lane_worker in pkgs/fleet/crates/fleet-agent/src/next.rs had no mapping for the oc-* scheduled-lane families, so fleet agent next --lane oc-code-02 claimed with --worker oc-code-02 and the broker refused every claim (lane oc-code-02 belongs to worker repo-worker-code, not oc-code-02). The lane starved deterministically.
Fix
Hardcoded mapping, extended in place — the same shape as the existing families (the broker's lane registry is a DB table; this function is the agent-side mirror of its canonical bindings, pinned by the #6853 oracle test, so there is no runtime derivation to extend):

registered_lane_worker strips an oc- prefix and defers to a new oc_lane_worker.
A letter suffix under oc- resolves exactly like the bare lane (oc-not-code-a → not-code-a → repo-worker-nix), by recursion into the existing mapping.
The scheduled lanes' numeric suffix (oc-code-02) resolves by
COMMENT by coderabbitai at 2026-11-23T08:23:53Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 35 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T08:24:01Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:25:46.994903Z
38b1d6f
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
COMMENT by dezren39 at 2026-11-23T11:49:40Z: Independent verification from scheduled lane oc-code-01 (laptop seat):

Reproduced the defect on the installed fleet build (rev b8bbcc0): fleet agent next --agent oc-code-01 --lane oc-code-01 --repo developing-today/code --filter 'repo = code' reports the queue rung rejected with claim_repo_task: lane oc-code-01 belongs to worker repo-worker-code, not oc-code-01, then exits 5. Same shape as #13073's oc-code-02 repro, so this is lane-family-wide, not lane-specific.
Verified this PR's fix on branc
FILES: pkgs/fleet/crates/fleet-agent/src/next.rs, pkgs/fleet/crates/fleet-agent/src/tests.rs

===== code#13127 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +10/-10 created=2026-11-23T08:41:06Z updated=2026-11-23T08:42:47Z
TITLE: docs(pm-stack): E7 — user→tenant in #3811-era docs
BODY: Closes #12802
E7 documentation pass (component E of #9078, design §11 item 7): apply the §9 vocabulary change — #3811's "user" becomes "tenant" — in the #3811-era docs, and verify the tenancy pointer in planning/architecture/README.md.
Scope and rename rule
"user" → "tenant" only where "user" is the tenancy unit, in the living docs of the #3811 data plane:

pkgs/id/hosted-worker/README.md — "per-user Durable Object" → per-tenant; "a minimal per-user store" → per-tenant; bridge section "this per-user store" → per-tenant (3 lines)
doc/2026-11-01_jurisdiction_aware_user_store_design.md — the routing spec the package implements (reconciled by #5199): title "hosted user stores" → "hosted tenant stores"; "A user row" → tenant row; "empty user store" → tenant store; "touch user state" → tenant state; "for all users" → all tenants; "a user's jurisdiction" → a tenant's; "other users may reference
COMMENT by coderabbitai at 2026-11-23T08:41:15Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 18 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T08:41:17Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:42:46.840883Z
f47ed4a
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: doc/2026-11-01_jurisdiction_aware_user_store_design.md, pkgs/id/hosted-worker/README.md, planning/architecture/README.md

===== code#13132 draft=False mergeable=UNKNOWN state=UNKNOWN files=4 +559/-1 created=2026-11-23T08:55:12Z updated=2026-11-23T08:57:45Z
TITLE: feat(atlas-search): D1 hot-cache admission policy — heat, heavy-query, tenant quotas, eviction (#13103)
BODY: Follow-on 3 of 6 under #9134 — the admission policy from §3 and §5 of the D1 tenant hot-cache design (doc/2026-11-23T08-06-00Z_design_atlas_d1_tenant_hot_cache, merged via PR #13098).
What landed
New pure policy module pkgs/atlas-search/worker/src/admission.ts — no imports from the store (follow-on 1, #13100) or the freshness evaluator (follow-on 2, #13101); its entry facts mirror the design §5 schema fields so follow-on 4 (read-path integration) wires it to whatever store lands. Sibling slices were built in parallel from main; this slice does not depend on their branches.

Heat counters — HeatTracker: a key becomes eligible after heatRequests requests inside heatWindowMs (defaults 2 / 10 min). In-memory per isolate; heat is evidence, not state of record.
Heavy-query immediate admission — first request admits when source latency > 250 ms or rows-read > 10,000 (strict inequalities, per th
COMMENT by coderabbitai at 2026-11-23T08:55:23Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 4 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository ap
COMMENT by chatgpt-codex-connector at 2026-11-23T08:55:27Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:57:44.689050Z
8a9c431
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: .github/workflows/atlas-search-worker-tests.yml, pkgs/atlas-search/worker/README.md, pkgs/atlas-search/worker/src/admission.ts, pkgs/atlas-search/worker/test/admission.test.ts

===== code#13133 draft=False mergeable=UNKNOWN state=UNKNOWN files=3 +576/-0 created=2026-11-23T08:55:13Z updated=2026-11-23T08:56:51Z
TITLE: impl(atlas): D1 hot-cache freshness evaluator — per-class watermark/TTL rules (#13101)
BODY: Follow-on 2 of 6 under #9134 (Atlas D1 tenant hot cache design, merged via PR #13098), implementing §1 + §8 item 2 in pkgs/atlas-search/worker. Based on main — no code dependence on follow-on 1 (#13100, the D1 store).
What landed

src/freshness.ts — the pure freshness evaluator. No I/O, no clock reads: the caller supplies now and the watermark read result.

Class A (content-addressed payloads): the hash is the version — fresh on hash match at any age, watermark state irrelevant; a hash mismatch (or an entry with no hash to prove identity) is never served.
Class B (query result sets): fresh iff the entry's source_watermark still equals the tenant's current watermark and age ≤ TTL_B (default 60 s) — whichever fails first, so a watermark advance is stale even inside the TTL.
Class C (tenant aggregates): fresh iff age ≤ TTL_C (default 15 min) and no invalidation event for the tenant post-dat
COMMENT by coderabbitai at 2026-11-23T08:55:22Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 4 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository ap
COMMENT by chatgpt-codex-connector at 2026-11-23T08:55:26Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T08:56:51.005140Z
52315ba
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: pkgs/atlas-search/worker/README.md, pkgs/atlas-search/worker/src/freshness.ts, pkgs/atlas-search/worker/test/freshness.test.ts

===== code#13155 draft=False mergeable=UNKNOWN state=UNKNOWN files=9 +991/-126 created=2026-11-23T09:23:45Z updated=2026-11-23T09:29:45Z
TITLE: feat(fleet): gh sync verbs for scripted flows (#10973)
BODY: Step 2.2 of #9016 — the Queue/GitHub sync verbs. Refs #10973.
What landed
Eight typed fleet gh sync-* shadow verbs (new module pkgs/fleet/crates/fleet/src/gh_sync.rs): each declares its whole command line, validates it strictly (the #3352 contract — undeclared arguments are usage errors), and rewrites to the canonical gh call. The rewrite is applied in gh::run before the close guard, the closure layer and the metadata create path parse the arguments, so a sync call inherits the step-1 token rotation, the retry-safety rule, value-free errors and the audit trail exactly as the hand-written passthrough call it replaces — and the caller's stdout contract stays gh's own output, byte-exact.



Verb
Canonical call




sync-labels --repo R <issue|pr> N
<kind> view N --json labels --jq [.labels[].name]


sync-view --repo R <issue|pr> N
<kind> view N --json labels,state,title


sync-comment --repo
COMMENT by coderabbitai at 2026-11-23T09:23:54Z: Warning
Review limit reached
You've used all free OSS reviews for now. Wait for the free limit to reset to keep reviewing this public repository.
Next included review available in 48 minutes.
Check out review usage here.

View limit details



Limit details: You’ve used the included review currently available.
Learn how review limits work.
Review configuration:

 Run configuration




Source mode: Auto
Repository: Unavailable
Branch: Unavailable
File: No configuration file from this repository a
COMMENT by chatgpt-codex-connector at 2026-11-23T09:23:57Z: Codex Review Summary: ✅ Completed
This comment shows the latest Codex review activity on this pull request.



Review
Status
Commit
Review trigger




📝 Code Review
✅ Completed 2026-11-23T09:29:44.907471Z
d142839
PR opened



 ℹ️ About Codex in GitHub

Your team has set up Codex to review pull requests in this repo. Reviews are triggered when you

Open a pull request for review
Mark a draft as ready
Comment "@codex review" or "@codex security review".

Codex reacts with 👀 while any review is run
FILES: pkgs/fleet/crates/fleet-registry/src/lib.rs, pkgs/fleet/crates/fleet/src/gh.rs, pkgs/fleet/crates/fleet/src/gh_sync.rs, pkgs/fleet/crates/fleet/src/main.rs, pkgs/fleet/docs/verbs.md, pm-stack/dispatch/src/github.ts, pm-stack/opencode-claimant/claimant.py, pm-stack/opencode-claimant/launch_one.py, pm-stack/opencode-claimant/test_claimant.py
