# GitHub and dynamic compute (P7)

> **DRAFT — drafted by the fleet 2026-11-12 from Drew's reprioritization order; awaiting Drew's ratification. Not Drew's own words.**

- Living document · Status: DRAFT · Program: P7 GitHub + dynamic compute (band 40) · Register entry: X9 · Tracking epic: developing-today/code#8805 · Milestone: "GitHub + dynamic compute" · Label: `epic/github-dynamic-compute`

## Purpose

The fleet's compute is increasingly **dynamic**: GitHub-hosted Actions pools, Blacksmith runners, Codespaces, and ephemeral dispatch surfaces that appear when work exists and vanish when it doesn't. This design states how the fleet uses GitHub and dynamic compute on purpose — which pool runs what, what the finite quotas are spent on, and what is deliberately never automated — so capacity stops being folklore and starts being policy.

## Current state (from the register and goals)

- **X9 — the CI program (2026-10-18)** is the ancestor: nix removed from GHA builds (direct toolchains; flakes stay for local dev — code#286, opencode#49, nix#398, all closed); caching gaps filed and closed (code#288); concurrency groups rolled out everywhere applicable (cancel-in-progress for PR/push CI, group-only for scheduled/bot workflows, release/publish/deploy skipped); **Blacksmith dual-track** decided — GitHub-hosted runners as the default with Blacksmith one comment-swap away (`# RUNNER-SWITCH:` markers), opencode moved to GitHub runners with the Blacksmith parts commented and labelled.
- **Goal `efficient-use-of-codespaces-actions-and-blacksmith`** (measured 2026-11-06, tracking code#6580): Blacksmith is already installed on the developing-today org (free tier, 3,000 weighted minutes/mo; org repos only — personal repos structurally cannot use it); measured build speed is a tie on cache-warm workloads (2m29s vs 2m25s) and ~1.4× on a cold CPU-bound compile (29.86s vs 42.25s); **queue latency is the real win** — org GitHub-hosted setup jobs waited 34.6 min unassigned while Blacksmith/personal-pool legs started in 7–12s. Codespaces: Pro gives 180 core-hours/mo + 20 GB storage; ≈177.5 core-hours were unspent at the Nov 6 check; prebuilds on dezren39/fleet cut create from ~27 min cold to ~1–2 min. Actions capacity: public-repo minutes are unlimited/$0 — **concurrency is the budget** (personal Pro 40, each Free org 20, macOS 5 per owner).
- **Setup already wired:** org-run's setup job runs on Blacksmith by default (code PR #6639, proven 7s queue vs 20m25s for the GitHub-hosted leg in the same run). Blocked on one Drew tap: installing Blacksmith + CodeRabbit on developing-today-forks (sudo-mode confirm) would open a second 3,000-min Blacksmith pool.
- **Goal `github-actions-usage-review-trim-to-what-s-needed`**: workflow-by-workflow Actions review once October's usage report lands; exposure is private-repo minutes against Pro's 3,000 included (public usage was ~$18 comped in October).
- **Goal `github-pro-bots-runners-what-can-we-actually-use`**: the standing inventory of Pro benefits and bot/runner free tiers (CodeRabbit, Copilot, Jules, Blacksmith, Cachix vs Actions caching, container-image runners).
- **Standing policy:** dispatch-only — no auto triggers (code#5663); saturation means dispatching existing work wide across pools, never new triggers. Two Drew-side gates recorded in the goals: the `gh` codespace scope grant (blocks live Codespaces verification) and the org Team plan decision ($4/user/mo → org Codespaces settings + prebuilds + 60 concurrent).

## The design

1. **Pools are named, finite, and owned.** The program keeps one living capacity table (this doc): GitHub-hosted personal (40 concurrent, Pro private 3,000 min), GitHub-hosted orgs (20 concurrent each), Blacksmith developing-today (3,000 weighted min/mo, org repos only), Codespaces (180 core-hours/mo, 20 GB), plus the laptop and lab cluster (P9) as the non-GitHub pools. Every pool has a burn-down intent and a hard stop.
2. **Placement rules (from the Nov 6 measurements):** latency-sensitive → personal pool or Blacksmith; bulk/background → org pools; interactive/persistent → Codespaces; CPU-bound cold compiles → Blacksmith benchmark legs; cache-warm builds → wherever queue latency is lowest (speed is a tie). Org-run's setup job stays on Blacksmith by default.
3. **Dual-track stays the runner doctrine (X9):** workflows are written GitHub-hosted-first with Blacksmith as a labelled one-line switch (`# RUNNER-SWITCH:`), so runner choice is a dispatch-time decision, not a rewrite.
4. **Finite pools burn down on purpose, to ~100% by cycle end, on real work:** Blacksmith minutes on critical-path + weekly big-compile benchmark + rare macOS legs; Codespaces core-hours on scheduled clean-room verification of the four repos in the standing fleet codespace (one-codespace discipline for the 20 GB storage cap); private-repo minutes watched against the 3,000 Pro budget. Paid overage on any pool is Drew's decision, never a scheduling accident.
5. **Trim is the other half:** the Actions usage review (goal above) runs workflow-by-workflow and cuts what isn't needed; concurrency groups (X9) stay mandatory on new workflows; the no-auto-trigger rule (code#5663) is absolute.

## Deliverables

- [ ] This design ratified or amended by Drew.
- [ ] The living capacity table (pools, quotas, concurrency, current-cycle usage) maintained in this doc, refreshed monthly from the billing API.
- [ ] Placement rules applied to the fleet's dispatch surfaces (fleet-run / org-run / fleet-dispatch-test) and written into their docs.
- [ ] Actions usage review completed (per the trim goal) with per-workflow keep/cut decisions recorded.
- [ ] Codespaces verification cadence running (weekly clean-room run inside the standing codespace) once the codespace scope grant lands.
- [ ] Blacksmith second pool (developing-today-forks install) proven with a first dispatch, once Drew's sudo tap lands.
- [ ] Runner-switch audit: every workflow in the four repos carries correct concurrency groups and, where relevant, `# RUNNER-SWITCH:` markers.

## Open questions for Drew

1. The two standing gates: complete the `gh` codespace scope grant, and install Blacksmith/CodeRabbit on developing-today-forks (one sudo tap, ~30s)?
2. Org Team plan ($4/user/mo): buy for org Codespaces + 60 concurrency, or keep Free and keep routing around the 20-slot org queue?
3. Blacksmith paid overage: hard stop at the 3,000 free pool-minutes (current rule), or a named monthly cap?
4. Should unspent Codespaces core-hours near cycle end trigger a wider verification matrix automatically (dispatch-only, from the queue), or stay a manual call?
