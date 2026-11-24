# AUDIT-7 Lane 1 — PR-SWEEP report (2026-11-24)

- Programme: AUDIT-7 (Drew directive, 2026-11-23 ~17:56 CST), seven parallel audit lanes over four repos.
- Umbrella: https://github.com/developing-today/code/issues/13871
- Lane tracking issue: https://github.com/developing-today/code/issues/13872
- Lane: 1 — PR-SWEEP (open PRs)
- Date of record: 2026-11-24T00-40-00Z (folder convention)
- Method: all reads via `fleet gh api` (REST + GraphQL) only; all writes via `~/tools/gh-post`; merges via `fleet gh api -X PUT .../pulls/N/merge` with `merge_method=merge`. No plain `gh`, no GitHub connector. Every PR's body, comments and file list were read (bulk GraphQL, count-confirmed per repo: 190 page entries + 9 + 2 + 1 = 202 entries, 199 unique PRs).

## Counts

|                                                              |                                                                                                                                                                                            |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Examined (open at census, 2026-11-23 ~18:02 CST)             | 199 unique PRs (202 page entries; 3 entries were cross-page duplicates — the two census pages were fetched ~2 min apart while other lanes were opening PRs)                                |
| Merged                                                       | 101                                                                                                                                                                                        |
| Fixed + merged (base retarget after parent landed)           | 2                                                                                                                                                                                          |
| Marked superseded on the PR (duplicate; surviving PR merged) | 3                                                                                                                                                                                          |
| Skipped (hard skip / live activity)                          | 3 (+1 not-open hard skip)                                                                                                                                                                  |
| Blocked (reason + unblocker recorded per PR below)           | 90                                                                                                                                                                                         |
| Open code PRs after sweep                                    | 91 at 2026-11-24T00:47Z, including 6 PRs opened by other lanes during/after the sweep (e.g. #13901, Lane 6's report PR) and minus #13814, closed by its owner after the superseded marking |

Main-branch CI at report time: post-merge runs queued/in progress; no failures observed in the runs visible at 2026-11-24T00:35Z. CI runs on main only in this repo, so the queued main runs are the verification of record for the merge batches.

## Why these PRs had not merged (summary of causes)

1. **No merger, not no merit.** The dominant cause: worker lanes produce PRs and explicitly do not merge ("Worker does not merge", "READY TO MERGE" notes). 103 PRs were CLEAN, check-green and evidence-complete; they were simply waiting for a merge authority pass. This sweep was that pass.
2. **Merge conflicts from main's velocity** (44 PRs DIRTY at census). Main lands dozens of PRs a day; any PR older than a few hours that touches a hot file (hosted-worker package.json, supergateway/grok-cli lockfiles, id book) conflicts.
3. **Same-lockfile dependabot clusters.** Only the first PR in a cluster can merge; the rest conflict the moment it lands (supergateway ×5, grok-cli ×4).
4. **Duplicate PRs for the same fix** (3 pairs found; see below).
5. **Stacked PRs** whose base is another PR's branch (6 found; 2 unblocked by retargeting in this sweep).
6. **Drafts** (26) — WIP, spikes, negative results, and the staged Cloudflare-release family, all correctly unmerged.
7. **Decision-gated** (2): a generator proposal PR that states it does not merge itself, and the Renovate onboarding PR whose merge activates a new bot.

## Duplicates found (convergence)

- code#13814 vs code#13820 — same #13698 fix, same file, same regression test. **#13820 kept** (later, broader verification: 179 vs 16 tests) and merged; #13814 marked superseded on the PR.
- code#13699 vs code#13716 — same #13670 fix, same file, identical +2/−2. **#13716 kept** (explicit `nix-instantiate --parse` verification) and merged; #13699 marked superseded.
- code#13300 vs code#13625 — same babysitter queue-binding change; #13300's own lane comment states #13625 is the rebased landing. **#13625 merged**; #13300 marked superseded.
- Near-duplicate, complementary: code#13714 + code#13718 both fix the muse-bootstrap `timeout` test failure from different sides (fixture PATH list vs fixture symlink + regression test). Both merged cleanly in sequence; no conflict.

## Merged (101) — developing-today/code unless noted

| PR     | Merge SHA                                |
| ------ | ---------------------------------------- |
| #13856 | 744e0d14cf3c08c924f1bad39a75e9bd708fcd50 |
| #13855 | 9e29044684d7dedd6a6702916c4626624894d4fd |
| #13846 | 8bb8a4e7592126843a9c46c3a29f26c4ac4b7da0 |
| #13847 | 589fb922d9efd9999b42c7deb5fee622e253b557 |
| #13808 | d02677c77e607cb0841e4dc59c3c426711d8e5ad |
| #13811 | 616e8b301f9662201d716f30c7d1a2f1ceb7b6a7 |
| #13810 | c1f4ce23ada0ba144ee5b05dd0287702f9bcbb09 |
| #13807 | bf1ff3c9335568bfbaa0a30c3795fb5c374da0c7 |
| #13820 | a1c68af330af28bf009ba371a76c2afbfd30bc19 |
| #13866 | f63c6aa2df234563f6e2a70308604fd33bc35666 |
| #13736 | 7f84716a7b458ed9ac8d8bb5106c5a4e1d7df7ce |
| #13734 | fac42efe79d9ed1576786ecbbc9771cdd63c666e |
| #13718 | f0c7b987759c2416105a424cbd66506aaa15f5c3 |
| #13716 | bc05c722beca94ffcfbe820994711a20e45ec208 |
| #13709 | 38f7de1ebb498a03c4cef8648754b4b34961fd94 |
| #13692 | 927c1fb6c7e5d3aa766709e4a13fb3ff97f280f8 |
| #13688 | 053b16ba55661ae3d4f95df4858ebfd98f77da26 |
| #13653 | b17aa9c10641f9a547b5ff8524c2dc556576b30c |
| #13625 | c5fa0cc35da51e5330febae3687ca37ef4dd3264 |
| #13613 | a7dae458be23a69b7d70bbff5a94a415a1d27df3 |
| #13859 | 27597b8dfcbf94032645886c4dac9369bce6af9b |
| #13841 | b05abeb2a285eee06708dc991ddb7ca7807860ad |
| #13840 | 34a3c9bc2b1a7efa1ee0f9004d9e76941d418605 |
| #13826 | 2d9286db442f5022651f69cb573f40165abcbe1d |
| #13806 | eedfbe90a9902fb23699a12b1be3f5d838d3a9e2 |
| #13783 | 19807f38d5f321ba5f5070f86f90c6408d303b5b |
| #13781 | 370258333093980e078b7143cfe222811dc17f6a |
| #13769 | 83aa2410674073d5c69f09d3abfe1def288e2ff6 |
| #13768 | 013f184f75c802df14fbcbf15dc087bb92051fcf |
| #13756 | 1d964139d48975bb860ca19752bd43399e0efd7f |
| #13753 | cec643bb9b535de5738427ddb201697d310c110c |
| #13750 | 773f29542f7cd191cbb8557898b9d252ec3f778b |
| #13749 | f5bb3f363f8b06deeb9f8172dfcfad9f4347d467 |
| #13746 | 6b467b17a37bf7f3c6af4735355d3673eb63317b |
| #13791 | 46015b77aa22a81a64e79e4a25c4b4fd227cc4f2 |
| #13776 | f5e27ac5ba954ee2ba7fad9fd2a2209b4adcd170 |
| #13676 | 87c480e1e5cfe72c635df637affd5897bc126fbd |
| #13671 | 86dcd768ee59d04b0ac3296ca9664785e794fa93 |
| #13660 | b0c6f0d8723f405af9e62f3806c9b30e2da1d17f |
| #13651 | f2611d8ea943bb2e6927166b867f9462fdd34d81 |
| #13645 | c2a5156d168058c550f8ac50faaeed3f5edf7d77 |
| #13598 | 504cd78677417204e3d3aae44a437e43d22cfea4 |
| #13596 | 281b6757ebaf73d656b7d0eac938b46500591363 |
| #13588 | 776ab5313cbea300d7da04a3ce3aa3a24afd950e |
| #13584 | fab8c4e7c4a5cda5b5eadeda08b6f0a70dcbb491 |
| #13582 | e6527b4af59ba4edb68fcc9fa4574cb1a7d94c1f |
| #13581 | 7a7612a2bd3a406386f3990bead4599558b65862 |
| #13578 | 9f4e8a3fe37658b8844ae4b08c4e1b6a3103c9e7 |
| #13571 | adfb0be28903a9a7b5ca6267e976dd0d92998630 |
| #13569 | b7825f5690a003b921aa23fcdd138de34b2c3785 |
| #13566 | 5a04d45a33da22f1d780acad37e37d68b87b0fb0 |
| #13521 | 6302462919ff3e2a9a28d786a6802a7778b17287 |
| #13505 | cf170f8cc780e493b38a5d8e17b6e0bd4d3db65b |
| #13480 | b424bdc259a48b71b50750b89954a82b624fd765 |
| #13476 | 398f9b949261c19fca88c7177e3644cf86834d60 |
| #13440 | 5d5381c45e31cacc943626b3d93d4f67370b9c17 |
| #13439 | 3ee7c52b588de051b828ead2a26eb165afc4cfb6 |
| #13442 | c899508d26de7007a973acaf1a3b916669983c9d |
| #13433 | 4e83b31605309edee35a457a53143a1f5ce5a8e5 |
| #13432 | ba41881fbd5368ccbf9287dfb552232c21cd92cd |
| #13419 | 38e1ddf52805c398462c68f22fe2f7d96afc0fbd |
| #13417 | 62353e5960eafd4fafc16a24c9c0095772eeefe1 |
| #13402 | d949899cb8db5bb644b68445a862f3d4396f0ccb |
| #13381 | 5490ff4830747b6ec44e4669d38e48933197be81 |
| #13350 | 1380245c92c22b8d1717891a131f15e6ce98cf1d |
| #13325 | 679d5b656ca55baa60f678409ffcb5c2663b568c |
| #13437 | 6289372e574818ac2f4ae5c43bae9044e6529c57 |
| #13275 | 085775b33498375b67d0e82a0c82ac8fd82a612e |
| #13233 | 4d736a83ecffb7a29739228ff752f96213bfec37 |
| #13206 | 6dd0f1774a24266f625d679a56ad95e5abcd5e9d |
| #13187 | 80230fcb2cbf2d9bbad95775468698a89e65bdb9 |
| #13164 | d02f7386f16e08b1932420fc6e0ea7e7a5975f32 |
| #13151 | 14ad32aeaf0649bfa85c05ee6fbc2f34ffe8dd2c |
| #13136 | 4369707bc9c474d04c60b3fb6995190d9d70d852 |
| #13714 | 1a604d4c83051b917b1bbad89d34a2f59ff400c2 |
| #13125 | c9932da06d22965377f1bd17bb7cc93aa290e385 |
| #13116 | 7a031ad4dff8fe49317c6b62da928343c95e6063 |
| #13109 | 2c76a0c15d91c84b45a23df28bd34a3053b55032 |
| #13092 | a1a36f379d426ebda947ce069a4cb7b82181784c |
| #13082 | 83028b280412aa7034b6b11afa7293d0df4ef579 |
| #13070 | a606e061d0d3a1459fb6c3baec1828a248c78477 |
| #13054 | cdbdd6d1d1e872b7d6e96f61416746d8c0117168 |
| #13050 | 0f02cc5507449b17d7a94149d6f8f8c7f24ef4df |
| #13041 | 7d1476414b3298a9cb007bfdd81a8a2cda6a1243 |
| #13032 | abfe7a278b28d30b914ea927df5c397d2f943587 |
| #13030 | 67e8b71734d5a67656b3b48e7710eed75fbcecdd |
| #13012 | 02cb547a379e92129b139b163d4b8cf13901ffe7 |
| #12989 | fb0a7dc73f6801353804ea0ea80e333b27053ba6 |
| #12948 | d6d42dc9b436153e006171454b548e109545e456 |
| #12912 | 5423c77e11f848ad7567e8d0734b74af938cfb3b |
| #12887 | bdf9cb8567888bc6bd4e4c0e2140da442e8921f0 |
| #12861 | 213190bf18e8a2e3b75781f738df48215a2cca68 |
| #12795 | b8c35c0feadd244ac3b7d8c311534062b030ced4 |
| #12793 | 81f24b929242835e5fefdad35c465012d3ce8448 |
| #12771 | 4e625d23df822a2f4830a4e1f2efd8882ebc48f4 |
| #12750 | f54185c763a56ff1fadc22c5cba95e262042a218 |
| #12745 | 0cd74d0ce2b2a2eb26ea3d59fe9351f48ad9b65b |
| #12663 | d0cad9542b0b78f270a2c1377194065835a67a4b |
| #12662 | ae31044390fd2a8fc999a4ba78ffb1ba28b4e202 |
| #12666 | a4632e7cec7f261047b7bd405cfaefd5c4201c2b |
| #12682 | f86b52989c31b0697428eeb09e9af5eeb4f10ece |

## Fixed + merged (2)

- code#12827 — base was the #12793 branch; after #12793 merged, base retargeted to `main` via API, CLEAN, merged ac21bf040e872fea8901ee602ec0b25af96d8e1b. (Its child #12837 was then retargeted the same way and merged 44bf3d6efa6418821b6c2a4389c959689c0dfb65 — counted in the merged table above.)
- code#13580 — base was the #13566 branch; after #13566 merged, base retargeted to `main`, CLEAN, merged a510b9aefceb326da894c132782e857fafc80b0f.

## Superseded — comment posted on the PR (3)

- code#13814 → superseded by merged #13820 (comment 7117805250); since closed by its owner (absent from the open list at 2026-11-24T00:47Z).
- code#13300 → superseded by merged #13625 (comment 7118010958; the PR's own lane comment agrees).
- code#13699 → superseded by merged #13716 (comment 7118022189).
  Disposition for all three: CLOSE as superseded (left open — a sweep lane does not close another lane's PR outright; the reason is recorded on each PR).

## Skipped (3 + 1)

- code#13234 — HARD SKIP (flag-held), untouched.
- code#13237 — HARD SKIP; also not open at census (live state checked), untouched.
- opencode#2788 — live lane activity at lane start (created 2026-11-23T23:56Z, updated 2026-11-24T00:06Z); CLEAN and mergeable — its owner lane merges it.
- code#12943 — bot collector PR (github-actions[bot]) updated 2026-11-23T23:53Z, minutes before the sweep; rolling seed collection, owner = mcpx-seed-collector workflow.

## Blocked (90) — reason + unblocker

### Merge conflicts at census (DIRTY) — unblocker for all: rebase onto current main, re-run the PR's own verification, merge. Owner: originating lane.

code#13850 (115-file formatter drift; also depends on #13826 which has now merged), #13842, #13772, #13738, #13701, #13700, #13639, #13600, #13589, #13567, #13445, #13406, #13397, #13372, #13368, #13313, #13210, #13167, #13155, #13133, #13132, #13127, #13113 (fix verified by oc-code-01 comment; rebase only), #13111, #13093, #13089, #13042 (conflict was package.json-only per author comment), #13037, #12840, #12814, #12756, #12732 (unbuilt locally by design — CI is its verification path), #12627, #12605, #12548, #12539 (also stack parent of #12557).
code#13184 — its own comment names the exact conflict (event_store/ingest.rs decide_in vs main's COMPACTED residue check) and the re-test requirement.
opencode#2629, #2541 (also needs the TS 6.0.2/7.1.0-beta merge decision confirmed at rebase), #2536, #2535, #2534 (dependabot; rebase is dependabot's own job), #2533 (body: was blocked on #2525 red main), #2532 (contains a commit from prerequisite #2525 — rebase after #2525 lands).
openchamber#2535 (1-file stale.yml fix, closes #2534; rebase only).

### Became conflicting during this sweep (a sibling merged first) — unblocker: rebase

code#13829 (conflicts after #13841 landed; both touch id-worlds), #13587 (after #13582/#13791 book changes), #13311 and #13301 (after #13325 landed in hosted-worker), #13436, #13435, #13434 (supergateway lockfile cluster after #13808/#13437 landed; dependabot rebase), #13813, #13812, #13809, #13811-cluster grok-cli lockfile (after #13847 landed; dependabot rebase) — note #13811 itself merged; the grok-cli cluster remainder is #13813/#13812/#13809, #13594 (fleet charter book; overlapping #13584 merged first — rebase and merge; content is docs-only and Codex-reviewed).

### Drafts — unblocker: author marks ready when the stated condition is met

code#13425 (spill-pool model; world-actor follow-up named in body), #13407 (verification run pending per author comment; "PR remains draft"), #13371 (mcpx lint repair; still being updated 2026-11-23T20:20Z by oc-code-08), #12980, #12963 (validation pending laptop access, per body), #12741, #12726 (WIP, parked for capacity per body), #12743 (also DIRTY), #12576, #12569, #12568, #12567, #12566, #12563, #12561, #12554, #12553, #12547 (the staged Cloudflare-release family — drafts by design, manual protected releases), #12510 (also DIRTY), #12496 (DRAFT, negative result, "do not merge" in title — keep as evidence), #12348 (body: keep draft until §10 boundary resolved), #12312 (acceptance needs a live laptop MCP call), #12301, #12038 (also DIRTY; #5850 vocabulary unsettled per body), #9245 (coordinator disposition recorded on the PR 2026-11-23T20:32Z: NOT landed as-is, program dependency lifted), opencode#2499 (spike/proof; production flip is Drew's decision; also DIRTY).

### Stacked — unblocker: land the parent, retarget, merge

code#13102 (parent = component A slice 2 PR, still pending; slice 1 #13054 has now merged), #13060 (parent #13047 is DIRTY; also an unresolved Codex P1 per warden note), #13047 itself is DIRTY (parent of #13060), #12557 (parent #12539 is DIRTY).

### Decision-gated

code#13265 — generator proposal (NEAR_PCT 80→75); body states it does not merge itself and the #1342 controlled A/B gate applies. Unblocker: Drew decision + A/B gate.
code#13858 — dependabot cargo GROUP with major bumps (iroh 1.4→1.5, redb 4→5, comrak 0.59→1.1) across pkgs/id; no build verification in the PR. Unblocker: build/test verification dispatch, then merge.
code#13438 — dependabot npm group for supergateway (express 4→5, zod 3→4 majors) and now conflicting after the individual supergateway PRs landed. Unblocker: dependabot rebase + verification of the major bumps; the individual PRs (#13808, #13437 merged; #13436/#13435/#13434 awaiting rebase) are the lower-risk path and may make this group PR moot.
fleet#1 (dezren39/fleet) — Renovate onboarding; merging ACTIVATES Renovate on the fleet repo. Unblocker: Drew / fleet-owner decision.

### Failing checks

openchamber#2584 — UNSTABLE: check "build and test (macos-15)" FAILURE at head e8f8a98805f49538c2a479353d070f1951167709 (ubuntu-24.04 and windows-2022 SUCCESS). Unblocker: macOS native build/test fix by the #2573 owner work.

### Sequencing

code#12740 — body requires reconciling #12605 and #12732 first (both DIRTY). Unblocker: land those, then this telemetry PR.

## Supplements

Raw gathered material for this lane (censuses, full PR records incl. bodies/comments/files for the census window in all states, digests, queries and fetch scripts, the follow-up issue body) lives in `supplements/` inside this folder — index and provenance, including the /tmp-loss note, in `supplements/SUPPLEMENTS.md`.

## Repeatable procedure (the sweep recipe)

1. Census: `fleet gh api repos/<repo>/pulls?state=open&per_page=100` (+ `page=2` …), count-confirm against GraphQL `pullRequests(states:OPEN){totalCount}`.
2. Bulk state: one GraphQL query per repo page returning title, bodyText, isDraft, mergeable, mergeStateStatus, comments, files, base/head. NOTE: `mergeable` reads UNKNOWN until GitHub computes it — issue the query twice; the second read is authoritative.
3. Read every body + comments. Classify: draft / DIRTY / stacked (base ≠ main) / duplicate (same file + same linked issue) / decision-gated / live-claim / mergeable.
4. Merge in file-disjoint batches via `fleet gh api -X PUT repos/<repo>/pulls/<n>/merge -f merge_method=merge`; record every merge SHA. A 405 means it went dirty mid-batch — stop on that PR, record it, continue the batch.
5. Stacked PRs whose parent just merged: `fleet gh api -X PATCH repos/<repo>/pulls/<n> -f base=main`, re-read mergeable, merge.
6. Duplicates: merge the better-verified one; post a superseded comment on the other naming the surviving PR + merge SHA; do not close it from the sweep lane.
7. Post progress on the lane tracking issue at every slice boundary; final ledger in the dated doc record (this file).

## Follow-up filed

A follow-up issue carries the 90 blocked PRs as rebase-and-merge batches (premise + singular ownership: AUDIT-7 Lane 1, tracking #13872). The pilot-queue enqueue verb exists on the laptop (`fleet queue enqueue <packet> --queue repo-health-code-pilot`), but its packet schema could not be verified from repo sources inside this session, so no queue task was enqueued blind; the follow-up issue is the durable handoff and the coordinator should enqueue from it once the packet shape is confirmed.

## Appendix — lane brief verbatim

> You are LANE 1 — PR-SWEEP of programme AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST): seven parallel audit lanes over the four repos (developing-today/code, developing-today-forks/opencode, developing-today-forks/openchamber, dezren39/fleet), exhaustive — every item in scope, one by one, read in full, no sampling.
>
> UMBRELLA TRACKING ISSUE: developing-today/code#13871 — https://github.com/developing-today/code/issues/13871 — the cross-lane coordination point. File your own tracking issue in developing-today/code first, post its link as a comment on #13871, and post progress on your tracking issue at EVERY slice boundary.
>
> SHARED RULES (binding on all seven lanes):
>
> - GitHub access: the fleet GitHub subcommand ONLY. All reads via `fleet gh api` (the `fleet` binary lives at ~/.local/share/hatch-recovery/bin/fleet — put that dir on PATH); all issue/comment writes via `~/tools/gh-post` (the GitHub App identity). NEVER plain `gh` with the user token, NEVER the GitHub connector. Reason (Drew): direct access burns the personal user API tokens; the GitHub App identity must do this work. If a fleet-gh verb you need does not exist, report that to the coordinator in your return — do not fall back to plain gh.
> - Tracking first: file your own tracking issue in developing-today/code, link it on the umbrella (#13871), and post progress on it at EVERY slice boundary. Check the umbrella + an issue's comments before acting on it — if another lane has claimed it, the lane whose primary scope it is owns the action; the other records a pointer comment on the umbrella, never a duplicate issue or PR.
> - Exhaustive, one by one, read everything. No sampling, no skimming by title. Volume is large — work in slices (per repo, per batch); if a slice exceeds one session, enqueue per-batch sub-work to the pilot queue (`--queue repo-health-code-pilot`, kind `feature_slice` + `payload.slice`, NEVER kind `design`; premise + singular-ownership check first, fresh fleet-gh REST reads, comments per_page=100 count-confirmed) following the lane conventions. A claim comment from another lane is a HARD STOP.
> - Standing authority: merge PRs when verified done (merge authority applies; drafts/conflicted included, fix forward); never force-push main; never close issues for age; file-the-issue in the same pass for anything found. Flag doctrine: if a platform flag trips on a checkout, stop on that checkout, record it, vary the mechanic (worktree off the EXISTING clone, worktree at main then switch ref, vary transport fleet gh/git) before stopping; never re-ask Drew for a permanently-flagging item.
> - Repo work happens in laptop worktrees via ~/tools/laptop-ssh.sh (laptop `amd`, user `user`); NEVER the sandbox for repo work. One clone per repo on the laptop; each piece of work in a worktree off that clone.
> - Deliverables: (a) a dated report + comprehensive log committed to the code repo's doc/ area under the dated-record convention (`doc/<UTC datetime>_<kind>_<name>/` folder with a same-named .md inside, plus a line in doc/README.md), INCLUDING your own prompt verbatim (this whole brief); (b) findings filed as issues/PRs/labels AS YOU GO, not only at the end; (c) where the process is repeatable, a skill for it, committed in the repo if it has a skills home (check first) or documented as a procedure in the report; (d) a final summary comment on your tracking issue.
> - Latitude (rigid requirement to do the work, free choice in outputs): open investigation issues, designs, and bug tickets; open PRs for quick doc fixes and to document findings; create labels; suggest things that aren't top-level projects but could be; suggest where things are converging and could be merged, where there's duplication, which duplicate is best and how they should best be combined.
>
> YOUR BRIEF — LANE 1 — PR-SWEEP (open PRs). Scope: every open PR across the four repos that is NOT currently being worked on (verify: no live claim/lane activity in comments, no active queue task). For each, one by one: read it fully and determine why it hasn't merged (stale? conflicts? failing checks? awaiting review? awaiting a decision? superseded?). Merge it if possible. If the code is correct, or needs only minor fixes, make the fixes (worktree off the existing clone, laptop session) and then merge it. If it cannot merge, record exactly why on the PR (comment) and on your tracking issue, with the unblocker named. Return a per-PR disposition ledger.
>
> HARD SKIPS (absolute — do not touch, record as skipped): PRs #13234 and #13237 in developing-today/code (flag-held). Anything with a live claim from another lane. Program trackers #13240/#13241/#13264/#13272 are issues, not PRs — out of your scope anyway. Known merged-state caution: several PRs were merged earlier today (2026-11-23) — trust live state, not memory.
>
> Your return: your tracking-issue link; the per-PR disposition ledger (PR link, repo, state found, action: merged SHA / fixed+merged / blocked+unblocker / skipped+why); report doc path + merge SHA; counts (examined, merged, fixed, blocked, skipped).
