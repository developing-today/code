# GitHub Actions

GitHub Actions is the estate's CI system of record: **in use, and
proven** — it is where the code repository's checks, deploys and
housekeeping actually run. This chapter records the estate as
counted by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959),
under the programme tracker
[#13905](https://github.com/developing-today/code/issues/13905)).
How Actions combines with Nix is covered in
[CI/CD: Nix and GitHub Actions](ci-cd.md); this chapter is the
Actions estate itself.

## What exists today

- **92 workflow files** sit under `.github/workflows/` in the code
  repository (counted 2026-11-23). The families: per-project suites
  (id, mcpx, pm, fleet, atlas), worker publish/promote/deploy
  workflows, docs-to-Pages workflows, and housekeeping —
  branch-cleanup, stale-run-sweeper, worktree janitor and reaper,
  Bitwarden freshness, flake-inputs update, and a daily flake sync.
- **Runners are GitHub-hosted.** Counted from `runs-on:` lines:
  131 uses of `ubuntu-latest`, 14 of `ubuntu-24.04`, 2 of
  `ubuntu-24.04-arm`, plus matrix and conditional forms. **No
  `self-hosted` runner label appears anywhere in this repository's
  workflows** — the census searched for it and found none.
- **Blacksmith is an option, not the default.** The reusable
  workflow `.github/workflows/org-run.yml` takes a runner input
  choosing GitHub-hosted `ubuntu-24.04` or
  `blacksmith-4vcpu-ubuntu-2404`; its header records the setup job
  defaulting to Blacksmith so matrix expansion happens off the
  GitHub-hosted queue, with Blacksmith scoped to organisation
  surfaces. The fleet placement design widens this into a spill
  cascade — GitHub Actions, then Blacksmith, then Codespaces, then
  queued, never dropped — recorded in the fleet design register
  (`pm-stack/design-register-2026-11-11.md`). The record also keeps
  a caution: a closed fork issue recorded Blacksmith jobs that
  never picked up, with the queue growing unbounded.
- **Actions-exec** — live, short-lived exec sessions inside Actions
  runners, handed off by a coordinator — is
  [#13774](https://github.com/developing-today/code/issues/13774),
  open and in flight, with slices landing on 2026-11-23. The
  surface exists in the fleet book and in the queue broker; its
  design companion is the compile coordinator,
  [#13765](https://github.com/developing-today/code/issues/13765).
- **The wider estate** (workflow counts from the laptop's clones,
  2026-11-23; clones other than the primary may lag main):
  dezren39/fleet 11, developing-today/opencode 30, openchamber 34
  in the upstream clone and 22 in the developing-today clone,
  dezren39/nix 10, developing-today/nix-cache 2, cairn 2,
  zettel 4. The fleet repository's own workflows are the
  sync and placement machinery's CI; fork-sync pushes fleet main
  to roughly 102 forks.
- **Policy** (`.github/ci-policy.md`, owner rulings 2026-11-13 and
  2026-11-16): no pull-request or push CI triggers and no required
  checks; CI runs **nightly on main** (07:17 UTC by default) and on
  manual dispatch; deploys keep their push triggers. Merges never
  wait for CI — main-branch CI is post-merge evidence.

## What the record plans

Actions-exec (#13774) and the compile coordinator (#13765) are the
active build-out: ephemeral runner sessions as a fleet placement
surface, covered further in
[Build-server alternatives](build-servers.md) and
[Automation and automated scaling](automation-and-scaling.md).
The spill cascade is the standing placement direction of record.

## Gaps

- No self-hosted runner fleet exists; the "self-hosted" leg of the
  estate is Blacksmith and Actions-exec instead.
- The #13765 and #13774 designs live only in issue bodies — an
  audit record of 2026-11-24 flags that neither has a `doc/`
  record. That gap is recorded on the issues themselves.

## Sources

Platform census §§1 and 20 and supplements S1 and S11,
2026-11-23, landed as PR #13959.
