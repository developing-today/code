# Census A — Retrospective (raw material for the Code Book retrospective rollups)

Programme tracker: [developing-today/code#13905](https://github.com/developing-today/code/issues/13905).
Census compiled 2026-11-23 by Census A, the retrospective lane of the Code Book programme. This folder is its full work product, landed under the work-product-landing rule (Drew, 2026-11-23): the report plus every supplement gathered for it. The working copy also lives at `~/workspace/code-book/census-retrospective.md` in the Muse workspace.

## Contents of this package

- `2026-11-24T00-56-13Z_research_code-book-census-retrospective.md` — this report: the honestly covered period, week-by-week retrospectives W12–W48 with the five fields, month rollups, quarter/year honesty notes, and the capabilities timeline.
- `source-inventory.md` — every source mined, where it lives, what it covers, and the method notes.
- `github-merged-pr-counts-and-samples.md` — the raw GitHub Search outputs behind the report: merged-PR totals per repo per week and the per-week title samples for developing-today/code.
- `doc-dated-records-census.md` — the doc/ census: counts by month, the complete pre-October record list, and the October phase records relied on.
- `memory-completion-extracts.md` — keyword-filtered completion/merge/launch headings extracted from the daily memory notes, per date.
- `cycle-log-inventory.md` — the stewardship cycle-log file inventory and how the census used it.
Every claim below cites a source: an issue/PR number, a file path, or a log entry.
Merged-PR counts are GitHub Search totals (`is:pr is:merged merged:<range>`) read
2026-11-23 via the laptop fleet client (`fleet gh api search/issues`), per repo, per week.

## The honestly covered period — read this first

The sources do not cover the same ground, and this census does not pretend they do.

| Source | What it actually covers | Density |
|---|---|---|
| `doc/` dated records in developing-today/code (laptop checkout `~/git/code/doc`) | 619 entries total; dated records: **6 in 2026-03, 2 in 2026-04, 103 in 2026-10, 452 in 2026-11** (counted from directory names, 2026-11-23) | Sparse design records Mar–Apr; dense from 2026-10-07 |
| Daily memory notes (`~/memory/2026-*.md`) | **2026-09-28 → 2026-11-12 continuous**, then **2026-11-22 and 2026-11-23 only**. Nothing for 2026-11-13 → 2026-11-21. (The programme brief said memory begins ~2026-10-19; the files on disk begin 2026-09-28 — the files win.) | Very dense, but noisy: much of Sep 28 – Oct 18 is 15-minute nix/mcpx watch entries |
| Stewardship cycle log (`…/stewardship-commission…/hidden_files/cycles/`) | Hourly cycles from the inaugural cycle **2026-10-24T07:37:51Z** (`CYCLE-LOG-archive-2026-10-28.md`) through cycle ~475 on 2026-11-23 (`CYCLE-LOG.md`) | The densest *activity* record, but it records oversight, queues and blockers — not feature history |
| GitHub merged PRs (the spine) | developing-today/code: **0 merged PRs found for 2026-03-01 → 2026-09-27**; 2 in W41, then 59 → 304 → 946 → 1,554 → 1,152 → 1,142 → 526 (single day, Nov 23) | Authoritative for Oct–Nov; silent before October |
| The four books (fleet, atlas, id, mcpx) | Written 2026-11-23 as surveys of the then-current state; their roadmap chapters are snapshots dated 2026-11-23 (e.g. atlas book, `design/roadmap-and-issue-map.md`: "a dated snapshot… Where this chapter and #13241 disagree, #13241 is right") | Secondary only; used for corroboration, never as the sole source |

**Verdict on coverage:**

- **Fully supportable, week by week: 2026-10-19 → 2026-11-23 (W43–W48)** — GitHub, memory, cycle log and doc/ all overlap.
- **Supportable but thinner: 2026-09-28 → 2026-10-18 (W40–W42)** — memory + doc/ carry it; GitHub PR counts are near zero, so "what shipped" claims rest on the design records and the daily notes.
- **Design-record only: 2026-03-20 → 2026-04-01 (W12–W14)** — eight dated doc/ records and nothing else. Note the disagreement: the records are *dated* March–April, but the GitHub merged-PR spine shows no merged PRs in that window at all, so these read as design work later committed under its original dates, not as a verifiable March build history. The primary source (GitHub) wins; the weeks below are labelled accordingly.
- **No evidence in any mined source: 2026-04-02 → 2026-09-27 (W14–W39) and 2026-11-13 → 2026-11-21 inside the daily notes** (GitHub and doc/ do cover Nov 13–21; memory does not). Those weeks are stated as uncovered, not padded.

---

## Week by week

### W12 — 2026-03-16 → 2026-03-22 *(design records only)*

- **Biggest things built:** Nothing verifiable as built. Three design/feature records are dated this week: collaborative cursor enhancements (2026-03-20), content modes (2026-03-21), peer discovery (2026-03-22) — `doc/2026-03-20T05-30-00Z_feature_collaborative_cursor_enhancements`, `doc/2026-03-21T05-47-48Z_feature_content_modes`, `doc/2026-03-22T00-00-00Z_feature_peer-discovery`.
- **Hardest challenges / where the time went / capabilities unlocked / what we can do now:** No source supports any of these for this week. Stated plainly rather than reconstructed.

### W13 — 2026-03-23 → 2026-03-29 *(design records only)*

- **Biggest things built:** Records dated this week: file-list UI and metadata tags (both 2026-03-23) and a testing-architecture reference (2026-03-29) — `doc/2026-03-23T00:00:00Z_feature_file_list_ui`, `doc/2026-03-23T12-00-00Z_feature_metadata_tags`, `doc/2026-03-29T00-00-00Z_reference_testing_architecture`.
- **Everything else:** No supporting source. See W12.

### W14 — 2026-03-30 → 2026-04-05 *(design records only; week straddles the gap)*

- **Biggest things built:** Two records dated 2026-04-01: image upload and multi-instance — `doc/2026-04-01T00-00-00Z_feature_image_upload`, `doc/2026-04-01T00-00-00Z_feature_multi_instance`. After 2026-04-01 the record stops entirely until October.
- **Everything else:** No supporting source.

### W15–W39 — 2026-04-06 → 2026-09-27 *(uncovered)*

- One line, honestly: no dated doc/ records, no daily memory notes, and zero merged PRs in developing-today/code were found in any mined source for these 25 weeks. A retrospective cannot say what happened here from these sources, and this census does not guess.

### W40 — 2026-09-28 → 2026-10-04

- **Biggest things built:** The nix/mcpx programme dominated: a 15-minute watch/rebuild loop tracking upstream mcpx, with PRs #301 and #303 merged and #305 rebased on 2026-09-29 alone, and milestone rebuilds on upstream #308, #323 (`memory/2026-09-29.md`, `memory/2026-10-01.md`, `memory/2026-10-02.md` headings). Sol's nix plan was implemented complete on 2026-09-30, and "nix/mcpx program phase 2 COMPLETE" the same day (`memory/2026-09-30.md`).
- **Hardest challenges:** Keeping a Nix-built mcpx current against a fast-moving upstream — the week's texture is rebuild-after-rebuild (upstream #308, #323, #351, #352 all triggered rebuilds; `memory/2026-10-01.md`, `memory/2026-10-02.md`).
- **Where the time went:** The watch loop itself — dozens of near-identical 15-minute check entries per day in the daily notes — plus Nix persistence/symlink design work on 2026-09-30 (`memory/2026-09-30.md`: "Nix persistence — final symlink design", "Nix tarballs deleted + GC tested").
- **Capabilities unlocked:** A continuously rebuilt, Nix-packaged mcpx; a GitHub App installed, repo created and `gh` authenticated (2026-09-30, `memory/2026-09-30.md`).
- **What we can do now:** developing-today/code merged **0** PRs this week (GitHub search) — the week's work lived in the nix/mcpx repos and in setup, not yet in the monorepo.

### W41 — 2026-10-05 → 2026-10-11

- **Biggest things built:** The ID/worlds platform design burst in doc/: Roc worlds architecture (2026-10-07), attenuated capabilities and directory accounts/groups (2026-10-09), then phase records 0 and 1 — baseline, events, storage (2026-10-11) — `doc/2026-10-07T00-20-00Z_architecture_roc_worlds`, `doc/2026-10-09T09-59-07Z_design_attenuated_capabilities`, `doc/2026-10-11T19-04-32Z_phase_0_baseline`, `doc/2026-10-11T22-25-52Z_phase_1_events_storage`.
- **Hardest challenges / where the time went:** Design, not code: world isolation, cross-world access, signed artifacts, petnames/invites and directory sync all get records on 2026-10-10 alone (`doc/2026-10-10*`).
- **Capabilities unlocked:** A phased platform plan (phases 0–8) exists on paper by week's end.
- **What we can do now:** Still mostly prospective — code merged only **2** PRs this week; the forks show the surrounding activity (opencode fork 36 and openchamber fork 48 merged PRs across Sep 28 – Oct 18 combined).

### W42 — 2026-10-12 → 2026-10-18

- **Biggest things built:** ID phases 2–8 landed as records in two days — membership, sync/relays, groups/encryption, tags/documents, invites/pull/CLI, connectivity, open design items (2026-10-12 → 2026-10-13, `doc/2026-10-12*`, `doc/2026-10-13T23-44-27Z_phase_8_open_design_items`) — followed by "platform completion" (2026-10-14) and a run of Roc feature records (roc-json, roc-grid, roc-cap, chat history, records readback, world authoring CLI; 2026-10-15 → 2026-10-16, `doc/2026-10-15*`, `doc/2026-10-16*`). In code, the week is mcpx-heavy: 59 merged PRs, the visible tail (#264–#321) almost all mcpx daemon/session fixes plus CI concurrency groups (code PRs #289, #295, #303).
- **Hardest challenges:** mcpx daemon lifecycle — orphaned upstream sessions, takeover ordering, cancelled contexts (code PRs #285, #296, #299, #307, #320).
- **Where the time went:** mcpx correctness and the first automated issue sweeps: by 2026-10-17–18 sweeps are fixing and merging across code, opencode and openchamber (e.g. code PR #245 merged, #193 closed, 2026-10-17; openchamber docs PR #50 merged, 2026-10-18; `memory/2026-10-17.md`, `memory/2026-10-18.md`).
- **Capabilities unlocked:** Concurrency rollout COMPLETE 2026-10-18 (`memory/2026-10-18.md`); CI concurrency groups to cancel superseded runs (code PR #289).
- **What we can do now:** A swept, self-fixing repo routine exists — issues are filed, fixed and merged by scheduled sweeps, not only by hand.

### W43 — 2026-10-19 → 2026-10-25 *(first fully covered week)*

Merged PRs: **code 304, opencode fork 181, openchamber fork 181** (GitHub search).

- **Biggest things built:** The PM stack's first face — aggregator slices 1–3: live needs-you + quota panels (code PR #892), orchestration section + `/api/orchestration.json` (#900), mobile/needs-Drew tiers/today view (#901), dashboard slice 3 with pluggable pages and session registry (#920). Cold-store-reaper v1, a Google Drive cold-store CLI with pack / resumable upload+verify / range fetch (code PR #923). The claude-code-proxy resident stream-json bridge (code PR #875). The ID TRAIN: PR #776 merged and #643 closed on 2026-10-24 (`memory/2026-10-24.md`).
- **Hardest challenges:** Standing up the operating system around the code — this is the week the stewardship commission was adopted (charter v0.1; inaugural cycle 2026-10-24T07:37:51Z, `CYCLE-LOG-archive-2026-10-28.md`; `memory/topics/stewardship-commission.md`) and the week merge authority and the label/lifecycle protocol were settled (2026-10-24/25, `MEMORY.md`).
- **Where the time went:** Provider/plan archaeology and setup: Turso and Supabase both set up on 2026-10-20, the cluster programme's GitHub structure built (issues #501–#508, 2026-10-20), a fleet usefulness audit and a subscription/model tracker both completed the same day (`memory/2026-10-20.md`); Atlas collector built with full ingest done on 2026-10-21 (`memory/2026-10-21.md`).
- **Capabilities unlocked:** A live PM aggregator reading fleet state; cold storage on Google Drive; an hourly stewardship cycle that has run ever since.
- **What we can do now:** See the whole project's needs, quota and orchestration state in one dashboard — none of that existed on 2026-10-18.

### W44 — 2026-10-26 → 2026-11-01

Merged PRs: **code 946, opencode fork 366, openchamber fork 365, hatch-personal 6** (GitHub search).

- **Biggest things built:** The full queue cutover, approved and executed 2026-10-26 (`memory/2026-10-26.md`). Atlas: canonical merge complete and Turso production publish complete, both 2026-10-26, then Atlas migrated to the laptop, complete + verified 2026-10-28 (`memory/2026-10-26.md`, `memory/2026-10-28.md`). The quota loop closed — quota-current served on `/quota`, code PR #1329 merged 2026-10-27. Gatherer v1 live on the laptop (code#1151, 2026-10-27). pm-dispatch built and deployed 2026-10-30; Meta Muse direct mode built and wired as the sole Meta door the same day; deploy chain completed with generation 515 live (all `memory/2026-10-30.md`). Supabase cut over to muse-dev2, complete 2026-10-30 (`memory/2026-10-30.md`, `memory/2026-10-31.md`).
- **Hardest challenges:** Deploy-chain breakage — the NixOS switch was repeatedly blocked by mcpx breaks (code#2609, #2686; fix #2687 merged but not in the switch source; `memory/2026-10-30.md`), and five failing id tests blocked a deploy until fixed via PR #3211 on 2026-10-31 (`memory/2026-10-31.md`). Quota discipline was rebuilt mid-flight: the caution zone was retired and "burn to 100%, then closed" became the rule on 2026-10-27 (`memory/2026-10-27.md`).
- **Where the time went:** Pool/quota stewardship (snapshot-first runs numbered into the 60s by 2026-10-31, per `memory/2026-10-31.md`) and a placement audit + relocation that completed all batches on 2026-10-28 (`memory/2026-10-28.md`).
- **Capabilities unlocked:** A queue that all later fleet work runs on; Atlas search over a Turso production corpus; a third GitHub App (developing-today-backup, created + live 2026-10-31, `memory/2026-10-31.md`).
- **What we can do now:** Dispatch work through one queue, read live quota on `/quota`, and publish Atlas data to production instead of only staging it.

### W45 — 2026-11-02 → 2026-11-08

Merged PRs: **code 1,554 (the peak week), opencode fork 377, openchamber fork 340, dezren39/fleet 72, hatch-personal 6** (GitHub search).

- **Biggest things built:** laptop-agent, designed + built 2026-11-04 (`memory/2026-11-04.md`), and with it the broker stack: repo-queue + secrets brokers ordered and built the same day (code#5362), then the repo-queue → fleet queue migration completed (`memory/2026-11-04.md`). mcpx consolidation completed 2026-11-04, with `services.openchamber.enable` cascading to opencode + mcpx (`MEMORY.md`). The first lab Cloudflare Tunnel went live 2026-11-06 (lab.security.cab; `MEMORY.md`, home-cluster parent issue #6066). Two Spaces built on 2026-11-07: Projects Overview and AI Plans & Usage (`memory/2026-11-07.md`). The yard reaper landed (code#7093, closed 2026-11-08; code PRs #7820, #7829).
- **Hardest challenges:** The Supabase incident arc — the original muse-dev project was recovered, audited and deleted, closing the arc on 2026-11-02 (`memory/2026-11-02.md`) — and a CI kill + mass-merge wave the same day: 185 PRs merged, main fixed, Actions enabled on all four repos (coordinator report, 2026-11-02, `memory/2026-11-02.md`). A merged PR with a failing id-core test (PR #4099) was escalated to Drew the same morning.
- **Where the time went:** Migration, in every direction: Supabase → Turso waves 0–3 (2026-11-06/07, `memory/2026-11-06.md`, `memory/2026-11-07.md`), repo-queue → fleet, laptop-only execution migration fully closed 2026-11-02 (`memory/2026-11-02.md`).
- **Capabilities unlocked:** Agents can reach the laptop without SSH (laptop-agent); the fleet has its own queue and secrets brokers instead of borrowed ones; the lab is reachable through a public tunnel hostname.
- **What we can do now:** Run the whole multi-agent operation — claim, dispatch, secrets, laptop exec — on fleet-owned infrastructure.

### W46 — 2026-11-09 → 2026-11-15 *(memory covers only to 2026-11-12)*

Merged PRs: **code 1,152, opencode fork 128, openchamber fork 60, hatch-personal 10** (GitHub search).

- **Biggest things built:** From GitHub titles: free-tier provider expansion in the OpenCode config (Groq, NVIDIA NIM, OpenRouter free tiers; Z.ai GLM free door; code PRs #10558, #10561), fleet-queue claim-link rollup over the #9046 link (#10560), quota-plane shadow admission work (#10512, #10531), and id-native-shell's live WorldClient session loop + step-2 Screen renderer (code PRs #10526, #10536). Atlas design records continue daily (deletion propagation, embedding cost ledger, chunk-boundary v2, cold-tier shard format; 2026-11-09 → 2026-11-12, atlas book `design/roadmap-and-issue-map.md` design-record table).
- **Hardest challenges:** The board fix programme: all code merged but the NixOS switch still not landed as of 2026-11-11 (`memory/2026-11-11.md`) — the recurring "merged ≠ deployed" gap.
- **Where the time went:** Provider seating and route tuning — the workhorse pool was live-read on 2026-11-09 and seat changes + Fable seating landed 2026-11-10 (`memory/2026-11-09.md`, `memory/2026-11-10.md`).
- **Capabilities unlocked:** Free-model legs lead the route pools (formalised as the free-first rule on 2026-11-23 from this period's census work, `MEMORY.md`).
- **What we can do now:** A native ID shell renders a live world session (code PR #10536). After 2026-11-12 this week is GitHub-and-doc only — the daily notes stop, and this census does not fill the gap from titles alone beyond what is listed above.

### W47 — 2026-11-16 → 2026-11-22 *(daily notes resume only on 2026-11-22)*

Merged PRs: **code 1,142**; opencode fork 170 and openchamber fork 192 across 2026-11-16 → 2026-11-23 combined; **dezren39/fleet 20** in the same combined window (GitHub search).

- **Biggest things built:** From GitHub titles (code, late-week sample): the fleet machine registry artifact with amd-6 as first record (code PR #12717, for #2179), fleet-queue coordination verbs routed through `route_for` (#12706, for #8581), a durable SQLite coordination replica for the queue (#12692, for #8583), queue-backup Turso → D1 export with restore drill (#12685/#12686, for #8584), and a pm-console owner's switch for Cloudflare (code PR #12712). On 2026-11-22 itself: the queue lane ruling implemented — any agent can claim any job; code#12544 closed via PR #12558 and verified live with a claim as `archer` (`MEMORY.md`); fork sync extended so dezren39/fleet main pushes to all 102 forks (dezren39/fleet PRs #93/#94; first todo issue code#12484, then #12490 with 2 forks failing; `MEMORY.md`).
- **Hardest challenges:** The laptop channel: the host session proxy had been closing CONNECTs since a runtime restart; the working route became a local relay (127.0.0.1:3130 → tailscaled proxy 11056), repaired 2026-11-22, and the broker stack on init was built and cold-tested the same day — all five legs killed, rebuilt, verified (`MEMORY.md`, 2026-11-22 entries). Upstream, microsoft/onnxruntime returning 404 had been failing fleet main builds since 2026-11-18; Drew ruled on 2026-11-21 to publish a public mirror of the pinned 1.31.1 source (`MEMORY.md`).
- **Where the time went:** Recovery and plumbing — channel repair, init/broker hardening, fork-sync token work (GHCR_ORG_PAT reset from the laptop, 2026-11-22, `MEMORY.md`) — plus the start of the agent work loop: Drew's 2026-11-22 order to keep picking up `fleet agent next` items continuously, with the running-lane floor raised through the day (`MEMORY.md`).
- **Capabilities unlocked:** Fleet access verified without SSH (`fleet agent next` run directly from the sandbox, shortlist refilled 8/8, verified 2026-11-21, recorded 2026-11-22; `MEMORY.md`); a Grok seat enrolled (muse-grok, 2026-11-22, `MEMORY.md`).
- **What we can do now:** Any enrolled agent, of any kind, can claim any queue job under any name — the lane model that the Nov 23 burn then ran on.

### W48 — 2026-11-23 → 2026-11-29 *(only 2026-11-23 exists so far; one day)*

Merged PRs on 2026-11-23 alone: **code 526** (GitHub search).

- **Biggest things built:** The book programme itself. Atlas book COMPLETE — 57 chapters, 108,391 words, live on GitHub Pages; PR #13288 merged, tracking code#13251 closed. ID book COMPLETE — 67 chapters, ~126,700 words; PR #13358 merged, code#13332 closed. mcpx book audit + alignment complete, PR #13413 merged (all `MEMORY.md`, 2026-11-23). The four full-investigation programmes were launched and their censuses completed the same day: ID (code#13240), Atlas (code#13241), fleet (code#13272; starter PR #13274), mcpx (code#13264; starter PR #13269). Desktop Commander: self-hosted endpoint built and pipeline-proven, then an OAuth 2.1 front door built + verified end to end (code PRs #13427+, #13488; tracking code#13404; `MEMORY.md`, `memory/2026-11-23.md`). Fleet rollout: fleet-static-bcc8043 published, laptop queue-broker flipped to generation 6, code#13146 closed (`MEMORY.md`). Queue packet repair: all 37 repairable blocked rows repaired → queued (record on code#13191). code#3404's fenced jurisdiction migration control plane landed via PR #13347.
- **Hardest challenges:** The platform execution-risk flag — it re-fired on already-approved work (PR #13097 shepherd attempt, #12977/openchamber#2458 checkout) despite Drew's batch approval, and the day's routing doctrine changed three times in response (laptop sessions → varied mechanics → Desktop Commander first; `MEMORY.md`, 2026-11-23 entries; dataset at `~/workspace/flag-observations/`).
- **Where the time went:** Investigation + documentation at programme scale (four investigation programmes, three books finished, ~452 doc/ records dated November by month's count so far) and lane landings under the flag — by 2026-11-23 the agent work loop's floor stood at a minimum of 16 running lanes (`MEMORY.md`).
- **Capabilities unlocked:** Desktop Commander as the first laptop route whose results empirically do not trip the flag (Lane DC1, 2026-11-23, `MEMORY.md`); the IdP hierarchy built 2026-11-23 became the interim primary CA for all other work, under the fleet CA programme (code#9155, in execution on Drew's order; `MEMORY.md`); fleet actions exec and the central compile coordinator were designed and ordered the same day (`MEMORY.md`).
- **What we can do now:** Read three complete, coverage-proven books about the project's own systems; reach the laptop through an OAuth-protected Commander endpoint; issue agent certificates from an in-house CA. None of that was true on 2026-11-22.

---

## Month rollups

### March 2026 *(thin — design records only)*

Six dated doc/ records (W12–W13): collaborative cursors, content modes, peer discovery, file-list UI, metadata tags, testing architecture. No GitHub, memory or log corroboration. The honest March rollup is one sentence: design records exist under March dates; nothing else about March can be verified from the mined sources.

### April 2026 *(thin — two records, then silence)*

Two records dated 2026-04-01 (image upload, multi-instance), then no evidence of any kind until 2026-09-28. April cannot carry the five retrospective fields.

### May – September 2026 *(uncovered, except the last four days of September)*

No mined source covers May through 2026-09-27. September's covered tail is W40 (from 2026-09-28): the nix/mcpx watch-and-rebuild programme and the Nix persistence work. A "September" rollup would be a rollup of four days, and is labelled as such if used.

### October 2026 *(covered 2026-10-05 → 2026-10-31; dense from 2026-10-19)*

- **Biggest things built:** The ID/worlds platform design (phases 0–8, W41–W42); the PM aggregator/dashboard (W43); the queue cutover, Atlas on Turso in production, and the `/quota` loop (W44). October's merged-PR spine: code 2 + 59 + 304 + (946 spanning Oct 26 – Nov 1), with the forks at 181–366 per week in the same span.
- **Hardest challenges:** mcpx daemon lifecycle correctness (W42); deploy-chain/switch blockage from mcpx and id test breaks (W44).
- **Where the time went:** Design records (103 dated October) and quota/pool stewardship.
- **Capabilities unlocked:** Stewardship as a standing hourly institution (from 2026-10-24); a live PM dashboard; production Atlas.
- **What we can do now that we couldn't on Oct 1:** Run work through a fleet queue, watch it and its quota cost live, and search a production corpus.

### November 2026 *(covered 2026-11-01 → 2026-11-23; daily notes missing 2026-11-13 → 2026-11-21)*

- **Biggest things built:** The fleet's own infrastructure — laptop-agent, queue + secrets brokers, repo-queue → fleet migration (W45); the lab's first Cloudflare Tunnel (W46 boundary, 2026-11-06); queue durability (SQLite replica, D1 backup/restore drill, W47); and the documentation capstone — three books completed and four investigation programmes run in a single day (W48, 2026-11-23). November's spine to Nov 23: code ≈ 1,554 + 1,152 + 1,142 + 526 merged PRs by week (W45–W48).
- **Hardest challenges:** The Supabase incident and its migrations (W45); the laptop channel collapse and rebuild (W47); the execution-risk flag regime (W48).
- **Where the time went:** Migrations first (Supabase → Turso, repo-queue → fleet), then recovery plumbing, then documentation.
- **Capabilities unlocked:** SSH-free fleet access for any agent kind; fork sync across 102 forks; an in-house interim CA; Desktop Commander.
- **What we can do now that we couldn't on Nov 1:** Operate the fleet end-to-end on fleet-owned infrastructure, and hand a new reader three complete books instead of a pile of design records.

## Quarter and year rollups — what they can honestly say

- **Q1 2026 (Jan–Mar):** Only W12–W13's six design records fall in this quarter, all in its last two weeks. A Q1 rollup can say "design records dated March exist" and nothing more. Jan–Feb are entirely uncovered.
- **Q2 2026 (Apr–Jun):** Two records dated 2026-04-01; the rest is uncovered. No honest Q2 rollup exists beyond that sentence.
- **Q3 2026 (Jul–Sep):** Uncovered until 2026-09-28. The quarter's only supportable content is W40's first three days.
- **Q4 2026 (Oct–Dec, in progress):** This is the quarter the record actually describes — and only 2026-10-05 → 2026-11-23 of it so far, with the caveats above. A Q4 rollup built from the October and November rollups is honest; anything reaching back into a "year of work" framing is not.
- **Year 2026:** A year rollup cannot honestly be written from these sources. What exists is: eight March–April design records, a 25-week silence, and an exceptionally dense eight weeks (2026-09-28 → 2026-11-23) in which the fleet, Atlas, the PM stack and the books were built out. The year chapter should say exactly that, present the Oct–Nov rollups as the year's verifiable substance, and mark the rest as unrecorded — not as quiet, and not as busy.

---

## Capabilities timeline

Capability → week it landed → citation. "Landed" means the cited source records it working, not merely designed.

- Nix-packaged mcpx under a continuous upstream watch/rebuild loop → **W40** → `memory/2026-09-30.md` ("nix/mcpx program phase 2 COMPLETE"); `memory/2026-10-01.md` (rebuilds on upstream #308, #323)
- ID/worlds platform phased design (phases 0–8) → **W41–W42** → `doc/2026-10-11T19-04-32Z_phase_0_baseline` … `doc/2026-10-13T23-44-27Z_phase_8_open_design_items`
- Automated issue sweeps that file, fix and merge across repos → **W42** → `memory/2026-10-17.md` (code PR #245 merged); `memory/2026-10-18.md` (openchamber PR #50 merged)
- CI concurrency groups cancelling superseded runs → **W42** → code PR #289
- PM aggregator: live needs-you + quota panels → **W43** → code PR #892; orchestration section → code PR #900; dashboard slice 3 → code PR #920
- Google Drive cold store (pack / resumable verified upload / range fetch) → **W43** → code PR #923
- Claude Code resident bridge via claude-code-proxy → **W43** → code PR #875
- Hourly stewardship commission → **W43** → `CYCLE-LOG-archive-2026-10-28.md`, inaugural cycle 2026-10-24T07:37:51Z
- Full queue cutover (one fleet queue) → **W44** → `memory/2026-10-26.md`
- Atlas in production on Turso; canonical merge → **W44** → `memory/2026-10-26.md`; Atlas migrated to laptop → `memory/2026-10-28.md`
- Live quota readback on `/quota` → **W44** → code PR #1329
- pm-dispatch service deployed → **W44** → `memory/2026-10-30.md`
- Meta Muse direct door (sole Meta route) → **W44** → `memory/2026-10-30.md`
- Supabase cut over to muse-dev2 → **W44** → `memory/2026-10-30.md`, confirmed `memory/2026-10-31.md`
- laptop-agent (laptop exec without SSH) → **W45** → `memory/2026-11-04.md`
- Fleet queue + secrets brokers; repo-queue → fleet migration → **W45** → code#5362; `memory/2026-11-04.md`
- mcpx consolidation; openchamber enable cascades to opencode + mcpx → **W45** → `MEMORY.md` (consolidation complete 2026-11-04)
- First lab Cloudflare Tunnel (lab.security.cab) → **W45** → `MEMORY.md` (live 2026-11-06; parent issue #6066)
- Projects Overview + AI Plans & Usage Spaces → **W45** → `memory/2026-11-07.md`
- Yard reaper for the fleet repo yard → **W45** → code#7093; code PRs #7820, #7829
- Free-tier provider doors in the route pools (Groq / NVIDIA NIM / OpenRouter / GLM free) → **W46** → code PRs #10558, #10561
- ID native shell: live WorldClient session + Screen renderer → **W46** → code PRs #10526, #10536
- Fleet access without SSH, verified (`fleet agent next` from the sandbox) → **W47** → `MEMORY.md` (verified 2026-11-21)
- Any-agent / any-name queue claiming → **W47** → code#12544, PR #12558, deployed 2026-11-22 (`MEMORY.md`)
- Fork sync: fleet main → all 102 forks → **W47** → dezren39/fleet PRs #93/#94; code#12490 (`MEMORY.md`)
- Broker stack self-healing on init (cold-tested, five legs) → **W47** → `MEMORY.md` (2026-11-22)
- Queue durability: SQLite coordination replica; Turso → D1 backup + restore drill → **W47** → code PRs #12692, #12685/#12686 (for #8583, #8584)
- Atlas book published (57 chapters, 108,391 words) → **W48** → code PR #13288; code#13251 closed (`MEMORY.md`)
- ID book published (67 chapters, ~126,700 words) → **W48** → code PR #13358; code#13332 closed (`MEMORY.md`)
- mcpx book aligned + published (84 chapters, ~288k words) → **W48** → code PR #13413; code#13264 (`MEMORY.md`)
- Desktop Commander self-hosted endpoint + OAuth 2.1 front door → **W48** → code PRs #13427+, #13488; code#13404 (`MEMORY.md`, `memory/2026-11-23.md`)
- Fleet-static rollout; queue-broker generation 6 → **W48** → `MEMORY.md` (rollout complete 2026-11-23 ~08:25 CST)
- Queue packet repair verb applied to production (37 rows repaired) → **W48** → code#13191 (`MEMORY.md`)
- In-house interim primary CA (IdP hierarchy) → **W48** → `MEMORY.md` (built 2026-11-23; programme code#9155)
- Fenced jurisdiction migration control plane in id → **W48** → code PR #13347 (for #3404)

## Source disagreements and caveats

- **Books vs primary sources:** the atlas book's roadmap chapter explicitly dates itself 2026-11-23 and defers to issue #13241; where its design-record table and the daily notes differ on what *shipped* (e.g. the Nov 8–12 Atlas records are designs, several marked "design only" in the book itself), this census follows the notes/GitHub and counts them as design work, not capability.
- **doc/ dates vs GitHub:** March–April dated records exist with zero merged PRs in the same window (GitHub search, 2026-03-01 → 2026-09-27, code repo). Treated above as design records, not build history.
- **Fork counts:** opencode and openchamber fork merged-PR counts are near-identical in W43 (181/181) and W44 (366/365) as returned by GitHub search; they are reported as returned, not adjusted.
- **Stewardship log scope:** the cycle log proves oversight cadence and blocker history (e.g. the five-item blocked-on-Drew carry still named at cycle ~471, `CYCLE-LOG.md`); it was not used as evidence that any feature shipped.
- **Missing days:** no daily notes exist for 2026-11-13 → 2026-11-21; W46/W47 claims for those days rest on GitHub titles and doc/ records only, and are phrased that way above.
