# Week by week

The weekly record, condensed from Census A with its five fields kept:
biggest things built; hardest challenges; where the time went; capabilities
unlocked; what we can do now. Weeks the sources do not support are stated as
such, in place, at their full length — which is short. Merged-PR counts are
GitHub Search totals read 2026-11-23, per repo, per week.

## W12 — 2026-03-16 to 2026-03-22 (design records only)

- **Biggest things built:** nothing verifiable as built. Three design and
  feature records are dated this week: collaborative cursor enhancements
  (2026-03-20), content modes (2026-03-21), peer discovery (2026-03-22).
- **All other fields:** no source supports any of them for this week.
  Stated plainly rather than reconstructed.

## W13 — 2026-03-23 to 2026-03-29 (design records only)

- **Biggest things built:** records dated this week: file-list UI and
  metadata tags (both 2026-03-23) and a testing-architecture reference
  (2026-03-29).
- **All other fields:** no supporting source. See W12.

## W14 — 2026-03-30 to 2026-04-05 (design records only; the week straddles the gap)

- **Biggest things built:** two records dated 2026-04-01: image upload and
  multi-instance. After 2026-04-01 the record stops entirely until October.
- **All other fields:** no supporting source.

## W15–W39 — 2026-04-06 to 2026-09-27 (uncovered)

One line, honestly: no dated `doc/` records, no daily memory notes, and
zero merged PRs in developing-today/code were found in any mined source for
these 25 weeks. A retrospective cannot say what happened here from these
sources, and this book does not guess.

## W40 — 2026-09-28 to 2026-10-04

- **Biggest things built:** the nix/mcpx programme dominated — a 15-minute
  watch and rebuild loop tracking upstream mcpx, with PRs #301 and #303
  merged and #305 rebased on 2026-09-29 alone, and milestone rebuilds on
  upstream #308 and #323. Sol's nix plan was implemented complete on
  2026-09-30, and "nix/mcpx program phase 2 COMPLETE" the same day.
- **Hardest challenges:** keeping a Nix-built mcpx current against a
  fast-moving upstream — the week's texture is rebuild after rebuild
  (upstream #308, #323, #351 and #352 all triggered rebuilds).
- **Where the time went:** the watch loop itself — dozens of near-identical
  15-minute check entries per day in the daily notes — plus Nix persistence
  and symlink design work on 2026-09-30.
- **Capabilities unlocked:** a continuously rebuilt, Nix-packaged mcpx; a
  GitHub App installed, a repo created and `gh` authenticated (2026-09-30).
- **What we can do now:** the code repo merged 0 PRs this week — the week's
  work lived in the nix/mcpx repos and in setup, not yet in the monorepo.

## W41 — 2026-10-05 to 2026-10-11

- **Biggest things built:** the ID/worlds platform design burst in `doc/`:
  Roc worlds architecture (2026-10-07), attenuated capabilities and
  directory accounts and groups (2026-10-09), then phase records 0 and 1 —
  baseline, events, storage (2026-10-11).
- **Hardest challenges / where the time went:** design, not code — world
  isolation, cross-world access, signed artifacts, petnames and invites,
  and directory sync all received records on 2026-10-10 alone.
- **Capabilities unlocked:** a phased platform plan (phases 0–8) exists on
  paper by week's end.
- **What we can do now:** still mostly prospective — the code repo merged
  only 2 PRs this week; the forks show the surrounding activity (36
  OpenCode-fork and 48 OpenChamber-fork merged PRs across Sep 28 – Oct 18
  combined).

## W42 — 2026-10-12 to 2026-10-18

- **Biggest things built:** ID phases 2–8 landed as records in two days —
  membership, sync and relays, groups and encryption, tags and documents,
  invites, pull and CLI, connectivity, open design items (2026-10-12 to
  2026-10-13) — followed by "platform completion" (2026-10-14) and a run of
  Roc feature records. In code, the week is mcpx-heavy: 59 merged PRs, the
  visible tail almost all mcpx daemon and session fixes plus CI concurrency
  groups.
- **Hardest challenges:** mcpx daemon lifecycle — orphaned upstream
  sessions, takeover ordering, cancelled contexts.
- **Where the time went:** mcpx correctness and the first automated issue
  sweeps: by 2026-10-17–18, sweeps were fixing and merging across code,
  opencode and openchamber.
- **Capabilities unlocked:** concurrency rollout complete 2026-10-18; CI
  concurrency groups to cancel superseded runs (PR #289).
- **What we can do now:** a swept, self-fixing repo routine exists — issues
  are filed, fixed and merged by scheduled sweeps, not only by hand.

## W43 — 2026-10-19 to 2026-10-25 (first fully covered week)

Merged PRs: code 304, OpenCode fork 181, OpenChamber fork 181.

- **Biggest things built:** the PM stack's first face — aggregator slices
  1–3 (live needs-you and quota panels, PR #892; the orchestration section,
  #900; mobile, needs-Drew tiers and today view, #901; dashboard slice 3
  with pluggable pages and a session registry, #920). Cold-store-reaper v1,
  a Google Drive cold-store tool with pack, resumable verified upload and
  range fetch (PR #923). The claude-code-proxy resident bridge (PR #875).
  The ID train: PR #776 merged and #643 closed on 2026-10-24.
- **Hardest challenges:** standing up the operating system around the code —
  this is the week the stewardship commission was adopted (charter v0.1;
  inaugural cycle 2026-10-24T07:37:51Z) and the week merge authority and
  the label and lifecycle protocol were settled.
- **Where the time went:** provider and plan archaeology and setup — Turso
  and Supabase both set up on 2026-10-20, the cluster programme's GitHub
  structure built (issues #501–#508), a fleet usefulness audit and a
  subscription and model tracker completed the same day; the Atlas
  collector built with full ingest done on 2026-10-21.
- **Capabilities unlocked:** a live PM aggregator reading fleet state; cold
  storage on Google Drive; an hourly stewardship cycle that has run ever
  since.
- **What we can do now:** see the whole project's needs, quota and
  orchestration state in one dashboard — none of that existed on
  2026-10-18.

## W44 — 2026-10-26 to 2026-11-01

Merged PRs: code 946, OpenCode fork 366, OpenChamber fork 365,
hatch-personal 6.

- **Biggest things built:** the full queue cutover, approved and executed
  2026-10-26. Atlas: canonical merge complete and Turso production publish
  complete, both 2026-10-26, then Atlas migrated to the laptop, complete
  and verified 2026-10-28. The quota loop closed — quota-current served on
  `/quota` (PR #1329, merged 2026-10-27). Gatherer v1 live on the laptop
  (2026-10-27). pm-dispatch built and deployed 2026-10-30; the Meta Muse
  direct mode built and wired as the sole Meta door the same day; the
  deploy chain completed with generation 515 live. Supabase cut over to
  muse-dev2, complete 2026-10-30.
- **Hardest challenges:** deploy-chain breakage — the NixOS switch was
  repeatedly blocked by mcpx breaks, and five failing ID tests blocked a
  deploy until fixed via PR #3211 on 2026-10-31. Quota discipline was
  rebuilt mid-flight: the caution zone was retired and "burn to 100
  percent, then closed" became the rule on 2026-10-27.
- **Where the time went:** pool and quota stewardship (snapshot-first runs
  numbered into the 60s by 2026-10-31) and a placement audit and relocation
  that completed all batches on 2026-10-28.
- **Capabilities unlocked:** a queue that all later fleet work runs on;
  Atlas search over a Turso production corpus; a third GitHub App
  (developing-today-backup, created and live 2026-10-31).
- **What we can do now:** dispatch work through one queue, read live quota
  on `/quota`, and publish Atlas data to production instead of only staging
  it.

## W45 — 2026-11-02 to 2026-11-08

Merged PRs: code 1,554 (the peak week), OpenCode fork 377, OpenChamber fork
340, dezren39/fleet 72, hatch-personal 6.

- **Biggest things built:** laptop-agent, designed and built 2026-11-04,
  and with it the broker stack: repo-queue and secrets brokers ordered and
  built the same day, then the repo-queue to fleet queue migration
  completed. mcpx consolidation completed 2026-11-04. The first lab
  Cloudflare Tunnel went live 2026-11-06 (`lab.security.cab`). Two Spaces
  built on 2026-11-07: Projects Overview and AI Plans and Usage. The yard
  reaper landed (closed 2026-11-08).
- **Hardest challenges:** the Supabase incident arc — the original muse-dev
  project was recovered, audited and deleted, closing the arc on
  2026-11-02 — and a CI kill plus mass-merge wave the same day: 185 PRs
  merged, main fixed, Actions enabled on all four repos. A merged PR with a
  failing id-core test (PR #4099) was escalated to Drew the same morning.
- **Where the time went:** migration, in every direction — Supabase to Turso
  waves 0–3 (2026-11-06/07), repo-queue to fleet, and the laptop-only
  execution migration, fully closed 2026-11-02.
- **Capabilities unlocked:** agents can reach the laptop without SSH
  (laptop-agent); the fleet has its own queue and secrets brokers instead
  of borrowed ones; the lab is reachable through a public tunnel hostname.
- **What we can do now:** run the whole multi-agent operation — claim,
  dispatch, secrets, laptop exec — on fleet-owned infrastructure.

## W46 — 2026-11-09 to 2026-11-15 (daily notes cover only to 2026-11-12)

Merged PRs: code 1,152, OpenCode fork 128, OpenChamber fork 60,
hatch-personal 10.

- **Biggest things built:** from GitHub titles — free-tier provider
  expansion in the OpenCode configuration (Groq, NVIDIA NIM, OpenRouter
  free tiers; the Z.ai GLM free door), the fleet-queue claim-link rollup,
  quota-plane shadow admission work, and id-native-shell's live WorldClient
  session loop plus its step-2 Screen renderer. Atlas design records
  continued daily (deletion propagation, the embedding cost ledger,
  chunk-boundary v2, the cold-tier shard format; 2026-11-09 to 2026-11-12).
- **Hardest challenges:** the board fix programme — all code merged but the
  NixOS switch still not landed as of 2026-11-11: the recurring "merged is
  not deployed" gap.
- **Where the time went:** provider seating and route tuning — the
  workhorse pool was live-read on 2026-11-09 and seat changes plus Fable
  seating landed 2026-11-10.
- **Capabilities unlocked:** free-model legs lead the route pools (formalised
  as the free-first rule on 2026-11-23 from this period's census work).
- **What we can do now:** a native ID shell renders a live world session.
  After 2026-11-12 this week is GitHub-and-`doc/` only — the daily notes
  stop, and this book does not fill the gap from titles alone beyond what
  is listed above.

## W47 — 2026-11-16 to 2026-11-22 (daily notes resume only on 2026-11-22)

Merged PRs: code 1,142; the OpenCode fork 170 and OpenChamber fork 192
across 2026-11-16 to 2026-11-23 combined; dezren39/fleet 20 in the same
combined window.

- **Biggest things built:** from GitHub titles (code, late-week sample) —
  the fleet machine registry artifact with amd-6 as first record, fleet-queue
  coordination verbs routed through `route_for`, a durable SQLite
  coordination replica for the queue, queue-backup Turso-to-D1 export with
  a restore drill, and a pm-console owner's switch for Cloudflare. On
  2026-11-22 itself: the queue lane ruling implemented (any agent can claim
  any job; #12544 closed via PR #12558 and verified live with a claim as
  `archer`), and fork sync extended so fleet main pushes to all 102 forks.
- **Hardest challenges:** the laptop channel — the host session proxy had
  been closing connections since a runtime restart; the working route
  became a local relay, repaired 2026-11-22, and the broker stack on init
  was built and cold-tested the same day: all five legs killed, rebuilt,
  verified. Upstream, a 404 from microsoft/onnxruntime had been failing
  fleet main builds since 2026-11-18; Drew ruled on 2026-11-21 to publish a
  public mirror of the pinned source.
- **Where the time went:** recovery and plumbing — channel repair, init and
  broker hardening, fork-sync token work — plus the start of the agent work
  loop: Drew's 2026-11-22 order to keep picking up `fleet agent next` items
  continuously, with the running-lane floor raised through the day.
- **Capabilities unlocked:** fleet access verified without SSH (`fleet agent
  next` run directly from the sandbox, verified 2026-11-21); a Grok seat
  enrolled (2026-11-22).
- **What we can do now:** any enrolled agent, of any kind, can claim any
  queue job under any name — the lane model the Nov 23 burn then ran on.

## W48 — 2026-11-23 to 2026-11-29 (only 2026-11-23 exists so far; one day)

Merged PRs on 2026-11-23 alone: code 526.

- **Biggest things built:** the book programme itself. The Atlas book
  completed — 57 chapters, 108,391 words, live on GitHub Pages (PR #13288,
  tracker #13251 closed). The ID book completed — 67 chapters, about
  126,700 words (PR #13358, #13332 closed). The mcpx book audit and
  alignment completed (PR #13413). The four full-investigation programmes
  were launched and their censuses completed the same day (ID #13240, Atlas
  #13241, fleet #13272 with starter PR #13274, mcpx #13264 with starter PR
  #13269). Desktop Commander: a self-hosted endpoint built and
  pipeline-proven, then an OAuth 2.1 front door built and verified end to
  end (tracker #13404). The fleet rollout: fleet-static published, the
  laptop queue broker flipped to generation 6, #13146 closed. Queue packet
  repair: all 37 repairable blocked rows repaired and queued (#13191). The
  fenced jurisdiction migration control plane landed in ID (PR #13347, for
  #3404).
- **Hardest challenges:** the platform execution-risk flag — it re-fired on
  already-approved work despite Drew's batch approval, and the day's
  routing doctrine changed three times in response (laptop sessions, then
  varied mechanics, then Desktop Commander first).
- **Where the time went:** investigation and documentation at programme
  scale — four investigation programmes, three books finished, about 452
  `doc/` records dated November by the month's count — and lane landings
  under the flag; by 2026-11-23 the agent work loop's floor stood at a
  minimum of 16 running lanes.
- **Capabilities unlocked:** Desktop Commander as the first laptop route
  whose results empirically do not trip the flag; the IdP hierarchy built
  2026-11-23 became the interim primary CA for all other work; fleet actions
  exec and the central compile coordinator were designed and ordered the
  same day.
- **What we can do now:** read complete, coverage-proven books about the
  project's own systems; reach the laptop through an OAuth-protected
  Commander endpoint; issue agent certificates from an in-house CA. None of
  that was true on 2026-11-22.
