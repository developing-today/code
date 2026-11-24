# BA-W07 verification and release evidence record

Issue: [#9371](https://github.com/developing-today/code/issues/9371) (BA-W07).
Parent: [#9359](https://github.com/developing-today/code/issues/9359).
Scope: BA-076–077, BA-090–091, BA-093–096, BA-101–102.
Second-pass verification recorded 2026-11-24 against main at
`012af4875e7f87d28e3361f27146724f539bea82`. This record distinguishes
**local**, **merged** and **deployed** states; nothing below claims a
production promotion that has not happened.

## First-pass verdict

The first pass genuinely landed the application and the release design
on main: the tracker implementation (PR #9379, merged 2026-11-13), the
native-cf packaging fix (PR #9383), the component designs under
`design/`, the external Turso/gateway adapter under `external/`, and
the staged protected release workflow (PR #12563, merged 2026-11-24).
Two first-pass claims did not hold up:

1. `external/README.md` cites `node external/test.mjs` as the validation
   for the adapter's signed authentication, replay prevention, forged
   identity stripping and atomic rollback. That file never existed in
   git history: the root `.gitignore` `test.*` pattern silently
   swallowed it. The external boundary was therefore untested while the
   README claimed coverage. Fixed by this pass (see below).
2. The requirement ledger rows for BA-095/BA-096 still say no
   application commit, design PR or main merge is established. That was
   true at intake and is false now; the dispositions below supersede
   those rows.

No unmerged first-pass work remains: the
`dot/bedside-and-away-implementation` branch has no commits beyond main
except a janitor state-preservation commit.

## What this pass added

- `external/test.mjs` — the test the README already described: a
  Hrana-shaped fetch harness over native SQLite covering value encoding,
  atomic batch rollback, conditioned-step skipping, signed Worker
  authentication (unsigned, wrong-key, stale, wrong-origin and tampered
  calls all rejected with 401), nonce replay rejection, forged identity
  header stripping, cross-principal isolation, and the loopback gateway
  read/write/search path with denied cases (missing login, unlisted
  login, cross-origin write → 403) and a gateway restart that preserves
  data and write access. It is a fixture, not live Turso evidence.
- `tests/restore-rehearsal.mjs` — a fixture-level backup/restore
  rehearsal over the full migration set: a logical backup restored into
  a fresh database preserves store IDs, item counts, item placements,
  note annotations, memberships and event versions exactly (full
  row-level equality), the restored database serves members, denies
  non-members and accepts the next versioned write, and the rehearsal
  guard refuses to restore an older backup over newer data.
- `scripts/test.mjs` now runs `external/test.mjs` as part of `npm test`.
- Root `.gitignore` gained `!pkgs/bedside-and-away/external/test.mjs`
  beside the existing `!pkgs/mcpx/scripts/test.sh` exception, so the
  file cannot be silently swallowed again.

## Per-requirement dispositions (BA-W07 scope)

| BA    | Disposition | Evidence |
| ----- | ----------- | -------- |
| BA-076 | Satisfied (merged) | Dedicated Turso database behind the private gateway/Worker; no Atlas code path anywhere in the package. `worker/BACKEND.md`, `external/README.md` topology. |
| BA-077 | Satisfied as designed (merged) | Provider placement defined: Turso is the single authoritative placement; D1 is retained as a local-compatible surface, not a second production writer; no dual-write or synchronization promise (`design/release.md`). Durable Objects explicitly not used. Old Sites D1 and new Turso inventoried as separate installations (`external/README.md` step 7). |
| BA-090 | Partial | Private Tailscale route with the real backend exists (loopback gateway verifying Tailscale identity, `external/gateway/proxy.mjs`; gateway path exercised in `external/test.mjs`). Full visual/accessibility parity with the prior aesthetic is **not verified**: the UI carries a viewport meta and minimal ARIA, and the integrated VM test itself disclaims browser/layout QA. Parity verification remains open and is not claimed. |
| BA-091 | Partial — blocked at promotion (hands item) | Working version is merged; the protected Actions release path is merged (PR #12563). No Actions upload or promotion has run: the `bedside-and-away-version-upload` and `bedside-and-away-production` GitHub Environments are not yet configured with required reviewers and production-scoped secrets/variables. Until they are, the manual native-cf operator path remains the live route. Exact remaining step below. |
| BA-093 | Deferred / optional (explicit) | ChatGPT surface is absent by design. No session/cookie passthrough is implemented anywhere; the surface requires a supported provider capability and its own design. Not counted as fulfilled. |
| BA-094 | Satisfied for the code path (merged) | Native `cf` only: no Wrangler configuration exists in the package; the release workflow installs the `cf` version pinned by `pkgs/cf/default.nix` (1.0.0-beta.20) and secrets are referenced by name from environment-scoped storage, never committed (`external/README.md` steps 2–3). Secret rotation remains an explicit manual operational action; no automatic rotation is claimed. |
| BA-095 | Satisfied by this record | Built/not-built and merged/deployed states are reported here and on #9371; the stale ledger rows are superseded as noted above. |
| BA-096 | Satisfied (merged) | Application source is in this repository at `pkgs/bedside-and-away` since PR #9379 (2026-11-13), with subsequent merged PRs including #9383 and #12563. |
| BA-101 | Satisfied | Spec issue #9359 and design intake PR #9361 (merged) review every statement via the requirement ledger (BA-001–102). |
| BA-102 | Satisfied for the work graph | The design mill produced `WORK-GRAPH.md`, eight linked work items (#9365–#9372) and the component designs under `design/`; implementation proceeded per workstream. Sibling workstreams carry their own acceptance and remain open. |

## Acceptance criteria mapping

| Criterion | Evidence | State |
| --------- | -------- | ----- |
| Restore rehearsal preserves IDs, counts, placements, annotations and event versions | `tests/restore-rehearsal.mjs` (this pass) | Fixture-verified; a live backup rehearsal is **not** claimed (release checklist step 3 forbids claiming one from a fixture) |
| Unauthorized direct Worker calls fail | `external/test.mjs` part B: unsigned/wrong-key/stale/wrong-origin/tampered → 401; the promote job also verifies live unsigned rejection after a real promotion | Fixture-verified; live check pending first promotion |
| Retries do not duplicate writes | `tests/backend.mjs` duplicate event/steering/variant/history cases; nonce one-use in `external/test.mjs` part B | Verified |
| Old/new client compatibility is bounded | Additive migrations only (0001 adds a constant-default column); release checklist step 5 requires API-before-UI only when old clients remain compatible, else a maintenance window | Bounded by process; no live old-client run claimed |
| Rollback cannot silently overwrite newer data | Restore guard in `tests/restore-rehearsal.mjs`; rollback rules in `design/release.md` step 7 and `design/identity-access.md` | Verified at fixture level; operational rule documented |
| Gateway restart/read/write/search and denied cases pass | `external/test.mjs` part C | Fixture-verified |
| No unrelated route changes, public access or Atlas ingestion | Release workflow validates the manifest (workers.dev on, preview URLs off, no custom routes) and never applies triggers; the Worker serves only signed `/api/*`; no Atlas integration exists in the package | Verified in code and workflow; live route state changes only via the blocked promotion step |
| Every BA ID has evidence or explicit disposition | Table above for BA-W07 scope; sibling scopes are owned by #9365–#9370 and #9372 | Complete for BA-W07 |

## Verification run

Remote dispatch on the code commit `fd6ff71c2` (this record's own doc
commit excluded): fleet queue run
[45371500215](https://github.com/developing-today-forks/fleet/actions/runs/45371500215),
conclusion **success**. The executor image ships no Node, so the run
fetched the official Node v24.22.3 tarball and ran
`node scripts/test.mjs` with it: **all 25 test files passed**,
including the six new PASS lines from `external/test.mjs` (adapter,
worker, gateway) and `tests/restore-rehearsal.mjs` (guard, rehearsal,
post-restore operation). Two earlier dispatches are part of the record:
run 45369385528 failed before any test executed (the bootstrap exported
`PATH` inside a subshell, so `node` was still missing — a command bug,
not a test result), and run 45370448142 was an environment probe that
established the executor has nix/jq/curl/bun but no Node.

## Remaining blockers (exact steps)

1. **Production promotion (hands item, repo admin + credential custody).**
   In repository settings, create the GitHub Environments
   `bedside-and-away-version-upload` and `bedside-and-away-production`,
   each with required reviewers. Upload environment: secrets
   `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN`; variables
   `BEDAWAY_TURSO_URL`, `BEDAWAY_GATEWAY_ORIGIN`. Production
   environment: secrets `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN`;
   variable `BEDAWAY_WORKER_URL`. Then dispatch
   `bedside-and-away-release.yml` with action `upload` from main,
   record the reported version ID and full source SHA, and dispatch
   again with action `promote` and those exact values. Application PRs
   and deployment keep distinct receipts: the upload run summary and
   the promotion run summary are the deployment receipts.
2. **Live operational rehearsal.** Release checklist steps 3–6
   (migration rehearsal against the real dedicated database, verified
   live backup restore, private staging smoke, deployed SHA/version and
   unsigned-rejection verification) require the production-shaped
   environment and credentials from step 1 and are not claimed from the
   fixtures in this record.
3. **BA-090 parity.** Visual/accessibility parity verification is
   unclaimed and remains open.
4. **Final BA-W07 acceptance** reconciles when the sibling workstreams
   (#9365–#9370, #9372) close; #9359 stays open until coverage and
   release evidence are reconciled, per its own instruction.
