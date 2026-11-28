# Release workflow for per-app version tags — who cuts the tags, how "tested" is attested, where the refusal lives

Issue: [#11177](https://github.com/developing-today/code/issues/11177)
(follow-on to [#5790](https://github.com/developing-today/code/issues/5790)
§5; the deploy-tool identity ledger, §5's other area, was
[#5766](https://github.com/developing-today/code/issues/5766), closed).
Status: **design record — the workflow of record for cutting per-app
release tags under the ratified #5790 scheme.** It authorizes no tag
by itself: the first tags wait for the implementation gate named in
§7. It does not re-decide the scheme — the numbering, derivation, and
bump discipline are #5790's, ratified by Drew's 2026-11-06 rulings
and already documented in `pkgs/fleet/README.md` ("Tag and release
procedure"). This record decides only what #5790 §5 left open: the
actor, the trigger, the attestation, and the refusal.

Governing decisions consumed, not re-opened:

- **#5790 §1/§3.9 (the ratified hybrid)**: per-app `<app>/vX.Y.Z`
  tags on the exact tested commit; repo tag
  `repo/v<YYYYMMDD>` (plus `.N`) at release points; loose-v0 bump
  discipline; derivation by git-describe (#8566, landed PR #8630).
  §3.9 names the accepted tooling cost this record discharges: "the
  deploy tool can refuse an untagged release build."
- **Drew's 2026-11-06 rulings** (recorded on #11177 and in the fleet
  README procedure): short app-name prefixes for all apps, names
  unique; apps start at `0.1.0` and stay in 0.x; repo prerelease
  tags on deploys allowed; main deploys are prereleases; **releases
  are tag builds only**.
- **#5766 (deploy tool)**: `pkgs/deploy/deploy.py` is the estate's
  deploy actor — registry of targets, per-target append-only
  generation logs (source rev, artifact, timestamp, actor), and
  `deploy status` reading what a target runs from the deployed
  binary's own version line (#5790 §1 rule 5). The release workflow
  extends this tool; it does not grow a second release actor.
- **The release gate (#1782 Phase 3)**:
  `.github/scripts/release-gate.sh` already judges a release SHA
  against the main-gate — the watched workflow set in
  `.github/workflows/main-broken.yml` (`nix ci`, `id`, `mcpx`,
  `mcpx conformance`, `mcpx nix build`, `go modules build`,
  `identity build`, `map build`, `id bench`) — fail-closed, and is
  already run by the goreleaser and mcpx publish paths. This record
  adopts its verdict as the attestation's core instead of inventing
  a second definition of "tested".
- **mcpx decision 0005 + `mcpx-release.yml`**: the one existing
  per-app release pipeline. Its tag scheme is the ratified one
  (`mcpx/vX.Y.Z`); its automatic tag-push publishing was **disabled
  by the owner 2026-11-13**, leaving deliberate manual dispatch as
  the publish path. This record reads that as the standing
  preference it is: releases are deliberate acts, not merge
  side-effects.

Verified state at writing (2026-11-23, origin/main `36673c7af`):

- The derivation is landed and documented (fleet README,
  `derived_version.rs`); the README's procedure section states the
  scheme's rules but names **no actor** ("a release is cut by
  tagging…"), **no attestation evidence**, and **no refusal**.
- `pkgs/deploy/deploy.py` has **no release or tag logic at all**
  (verbs: deploy, rollback, status, list, restore). Nothing in the
  estate refuses an untagged release build today.
- The only per-app tag on the repo is `mcpx/v0.1.0`, cut under
  decision 0005's own pipeline. There is **no `fleet/v*` tag and no
  `repo/v*` tag**: the first release under the ratified scheme has
  not happened.
- The watched main-gate set contains **no fleet workflow**;
  `fleet-rust-tests` runs on main but is judged by no gate. The
  attestation in §3 must therefore name per-app evidence explicitly,
  not just point at the gate.

## 1. The decision in one paragraph

**The deploy tool cuts the tags.** An operator (human or agent
lane) invokes a new `release` verb in `pkgs/deploy/deploy.py`
against a commit on `main`. The tool verifies the commit is on
`main` and that the attestation gate (§3) is green for that exact
commit, checks the requested version against the ratified
discipline, cuts an **annotated** tag whose message carries the
attestation record (§4), records the release in its own ledger
beside the generation logs, and — at a release point — cuts the
repo tag naming the app tags cut with it (§5). CI never creates
tags; it only builds from them (releases are tag builds only).
Unaided manual `git tag` is not a release path: it produces a tag
with no attestation and no ledger record, and §6's refusal treats
release claims without attestation as no release at all.

## 2. Actor and trigger

- **Actor**: `pkgs/deploy/deploy.py`, new verb
  `deploy release <app> [--version X.Y.Z] [--commit <sha>]`
  (default commit: current `origin/main` head). One actor for every
  app — the same tool that deploys the app and keeps its identity
  ledger (#5766) is the tool that mints its release identity.
- **Trigger**: deliberate operator invocation. Not a merge hook,
  not a schedule, not a tag-push side-effect. A tag is a claim
  ("this commit was tested together"); claims are made by an
  accountable invocation that can be refused, never as a
  side-effect of unrelated work. (No schedule is created, changed,
  or retired by this record; `doc/fleet-schedule-registry.md` is
  untouched.)
- **Preconditions the tool checks before cutting anything**
  (any failure refuses, §6):
  1. The commit is reachable from `origin/main`.
  2. The attestation gate (§3) is green for that exact commit.
  3. The version obeys the discipline: the prefix is the app's
     short deploy name (the name `deploy` and `--version` use —
     #5790 open question 1, answered 2026-11-06), unique across
     apps; the first tag for an app is `0.1.0`; versions stay in
     0.x; minor for a feature release, patch for a fix (the
     operator states which; the tool computes the next version
     when `--version` is omitted and refuses a requested version
     that is not exactly that next minor or next patch).
  4. No tag of that name exists pointing at a different commit.
     The same name at the same commit is a duplicate request and
     refuses as already-released.
- **What the tool does, in order**: verify preconditions → compose
  the attestation record (§4) → create the annotated app tag →
  create the annotated repo tag if this invocation is a release
  point (§5) → push the tags → append the release record to the
  deploy ledger → print the tags and the gate verdicts. A failure
  after the app tag is created but before the push leaves only a
  local tag, which the tool reports and deletes on the next
  invocation's precondition pass; nothing is half-published
  silently.

## 3. Attestation — what "tested" means

A tag claims its commit was **tested together**. The evidence is
the main-gate verdict for that exact commit, in two parts:

1. **The watched set.** Every workflow in the main-gate set
   (`.github/workflows/main-broken.yml`'s `workflow_run.workflows`
   list, kept in sync with `release-gate.sh`'s
   `WATCHED_WORKFLOWS`) reports a completed `success`, judged by
   exactly the rules `.github/scripts/release-gate.sh` already
   implements: the release SHA must be the head of `main` at
   judging time or an ancestor the gate's non-push rule accepts
   (since 2026-11-16 the watched workflows run on the nightly
   schedule, so the judged run is the latest completed run on
   `main` whose head SHA is the release commit or an ancestor of
   it); an older run never judges newer code for a workflow that
   did run on the commit.
2. **The app's own suite, when the watched set doesn't cover it.**
   The watched set is repo-level; it contains no fleet workflow.
   For an app whose test workflow is outside the watched set, that
   workflow's latest completed `success` run on `main` at the
   release commit (or the gate's ancestor rule) is also required —
   for fleet, `fleet-rust-tests`. The implementation (§7) records
   each app's extra evidence workflow in the deploy registry
   beside its targets, so the requirement is data, not a special
   case in the verb. An app with no test workflow outside the
   watched set needs nothing beyond part 1; an app with no green
   evidence at all cannot be released.

"Tested together" is thereby a checkable statement, not a vibe:
the named runs either concluded `success` over that commit's tree
or the tag does not exist.

## 4. Where the attestation is stored

Three places, in descending order of authority:

1. **The annotated tag message** — the record of authority. It
   travels with the tag, survives loss of every tool's state, and
   is auditable from git alone. Machine-readable block:

   ```
   release-attestation:v1
   app: <short-app-name>
   version: X.Y.Z
   commit: <full-sha>
   gate: main-gate + <app-evidence-workflow or "none">
   run: <workflow-name> <run-id> success <head-sha>
   run: … (one line per judged workflow)
   actor: <deploy-tool identity / operator>
   tool: deploy.py <derived version>
   at: <UTC timestamp>
   ```

2. **The deploy ledger** — an append-only release log in the
   deploy state dir, beside the per-target generation logs
   (#5766 §4.6 shape: one JSONL record per release with app,
   version, commit, tag, gate run ids, actor, timestamp).
   `deploy status` reads it to answer "what releases exist, on
   what evidence" next to its existing per-target deployed-version
   report. The ledger is a cache of the tag messages, never the
   other way round: if the two ever disagree, the tag wins and
   the disagreement is itself reported.
3. **The artifact** — the release build is made from the tag, and
   the built binary must report the clean version (`Derived
   version: X.Y.Z`; fleet README procedure step 3). A release
   whose artifact reports a `-dev` string is a failed release,
   recorded as such in the ledger, not quietly re-tagged.

## 5. The repo tag

At a **release point** — one `deploy release` invocation, or one
invocation naming several apps released together — the tool also
cuts `repo/v<YYYYMMDD>.<minor>.<patch>` per the ratified rule
(date is the major segment; a point including any app feature
increments minor and resets patch, a fix-only or repeat point
increments patch), annotated with the list of app tags cut at
that point and the same attestation block. The repo tag names
"the repo as it stood, tested together" and is the checkout-able
answer to #5790 open question 2 (kept, per the 2026-11-06
rulings). **Repo prerelease tags on main deploys are a different
thing**: allowed by the rulings, cut by the deploy path as
before, and never minted by `deploy release`.

## 6. The refusal rule — where it lives

#5790 §3.9's accepted cost, made concrete. The refusal lives in
**the deploy tool**, at two points:

1. **At deploy time (the §3.9 refusal).** `deploy` refuses to
   perform a deploy that claims release status without an
   attested tag. A deploy claims release status when the
   artifact it would activate self-reports a clean `X.Y.Z`
   version for its app. The tool then requires a tag
   `<app>/vX.Y.Z` at the artifact's source commit **whose
   attestation block verifies** (names that commit, carries gate
   run lines, and parses as `release-attestation:v1`). Missing
   tag, tag at a different commit, or a tag without a verifiable
   attestation block → refusal with a named error, before any
   activation step. Prerelease deploys are untouched by this
   rule: a `-dev.N` artifact from `main` deploys as a prerelease
   (Drew's ruling), and `deploy status` shows it as one — it is
   never presented as, or promoted to, a release.
2. **At release time.** `deploy release` itself refuses on any
   failed precondition in §2 (commit not on `main`, gate not
   green for the exact commit, version violating the discipline,
   tag name taken by a different commit). Fail-closed throughout:
   an attestation the tool cannot read is an attestation that
   does not exist.

The existing CI-side refusal stays as the second line for the
pipelines that publish outside the deploy tool: `release-gate.sh`
already refuses goreleaser/mcpx publishes from a red or
non-`main` SHA. This record does not move or weaken it.

## 7. The first tags — recorded gate decision

**Decision: no per-app tag is cut ahead of the tool.** Cutting
`fleet/v0.1.0` by hand today would mint exactly the object this
workflow exists to prevent — a release claim with no attestation
block and no ledger record — and would pre-empt the refusal in
§6 (the first tag would be its first counterexample). Under
#11177's acceptance, this section is the explicit recorded
decision: **the first real per-app release tags wait for the
implementation of this workflow in the deploy tool** (the
follow-on implementation issue filed with this record: the
`release` verb, the ledger, the §6 deploy-time refusal, the
registry field for per-app evidence workflows, and a pointer line
in the fleet README procedure naming this record). The expected
first release under the workflow is `fleet/v0.1.0`, cut by the
tool against a `main` commit whose gate is green at cutting time.

One grandfathering note: `mcpx/v0.1.0` already exists, cut under
decision 0005's pipeline before this workflow. It stands as
mcpx's `0.1.0`; mcpx's *next* release follows this workflow, and
the mcpx pipeline's tag-consumption (build and publish from the
tag, gate-checked) is already compatible with it.

## 8. What this record does not do

- It does not change the numbering scheme, the derivation, or the
  bump discipline (#5790, ratified; out of scope by #11177's own
  framing).
- It does not build the `release` verb — that is the follow-on
  implementation issue (§7). Until it lands, the interim state is
  honest: apps deploy from `main` as prereleases, and nothing in
  the estate claims to be a release.
- It does not touch the deploy registry's target/identity shape
  (#5766, closed) beyond naming one new registry field (per-app
  evidence workflow, §3) for the implementation to add.
- It creates, changes, and retires no schedule on any surface.
