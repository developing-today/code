# Census A — GitHub merged-PR counts and title samples (raw query outputs)

All queries run 2026-11-23 via the laptop fleet client only:
`~/.local/bin/fleet gh api "search/issues?q=repo:<repo>+is:pr+is:merged+merged:<range>&per_page=1" --jq .total_count`
(GitHub connector not used; sandbox `gh` not used.)

## developing-today/code — merged PRs per period

| Range                                 | Total |
| ------------------------------------- | ----- |
| 2026-03-01..2026-03-31                | 0     |
| 2026-04-01..2026-04-30                | 0     |
| 2026-05-01..2026-09-27                | 0     |
| 2026-09-28..2026-10-04 (W40)          | 0     |
| 2026-10-05..2026-10-11 (W41)          | 2     |
| 2026-10-12..2026-10-18 (W42)          | 59    |
| 2026-10-19..2026-10-25 (W43)          | 304   |
| 2026-10-26..2026-11-01 (W44)          | 946   |
| 2026-11-02..2026-11-08 (W45)          | 1,554 |
| 2026-11-09..2026-11-15 (W46)          | 1,152 |
| 2026-11-16..2026-11-22 (W47)          | 1,142 |
| 2026-11-23..2026-11-23 (W48, one day) | 526   |

Worked example (first query run): merged 2026-10-19..2026-10-25 returned total 304; the per_page=5 sample was #926 "amd: declare the muse user (sops password hash)", #925 "chore(ci): stop Jules and Renovate from bumping the workflow-owned flake inputs", #924 "fix(pm-stack): restore fleet-feed collector units after sandbox recycles", #923 "cold-store-reaper v1: Google Drive cold-store CLI (pack / resumable upload+verify / range fetch)", #921 "config(opencode): comment out cloudflare/openrouter sections; literal proxy ports".

## Other repos — merged PRs per period

| Repo                               | Sep28–Oct18 | W43 | W44 | W45 | W46 | Nov16–23 |
| ---------------------------------- | ----------- | --- | --- | --- | --- | -------- |
| developing-today-forks/opencode    | 36          | 181 | 366 | 377 | 128 | 170      |
| developing-today-forks/openchamber | 48          | 181 | 365 | 340 | 60  | 192      |
| dezren39/fleet                     | 0           | 0   | 0   | 72  | 0   | 20       |
| dezren39/hatch-personal            | 0           | 0   | 6   | 6   | 10  | 15       |

Note: the near-identical opencode/openchamber counts in W43 (181/181) and W44 (366/365) are as returned by GitHub Search; reported unadjusted in the census.

## developing-today/code — title samples per week (per_page=25, sort=created desc; selection as returned)

### W42 (2026-10-12..2026-10-18) — tail of the week's numbers

#321 test(mcpx): force schema read before diagnose in e2e tests · #320 fix(mcpx): honour a cancelled context in inlineDaemon · #318 ci(id): guard the PR check list against flake drift · #317 fix(mcpx): stop --all builds per-daemon clients · #315 fix(mcpx): sync dogfood .opencode plugin ops table · #313 docs(repo): planning folder with per-project roadmaps · #307 fix(mcpx): daemon restart waits for background orphan-session DELETEs · #306 fix(mcpx): honour MCPX_STATE_DIR in daemon discovery · #303 ci: key auto-assign concurrency group by issue/PR number · #299 mcpx: wait for orphan-session DELETEs on orderly exit · #296 mcpx: end orphaned upstream sessions off the takeover path · #295 ci: speed up mcpx and id workflows · #289 ci: add concurrency groups to cancel superseded runs · #285 fix(mcpx): honour a cancelled context in EnsureDaemon · #283 test(mcpx): execid tests use testsupport.TempDir · #281 mcpx: name the resolved key in the degraded-scope log line · #279 mcpx daemons: prune dead records · #277 fix(mcpx): refuse recipe @param holes in mcpx run · #275 mcpx: make <command> --help print help · #273 fix(mcpx): quote zsh completion entries · #271 fix(mcpx): report 'waiting for OAuth sign-in' · #269 mcpx-agy examples + first-served-skill guard test · #267 mcpx: DELETE a removed server's upstream session on takeover · #266 mcpx: notice when CLI and daemon versions differ · #264 mcpx: run node scripts with --experimental-transform-types

### W43 (2026-10-19..2026-10-25)

#926 amd: declare the muse user · #925 chore(ci): stop Jules/Renovate bumping workflow-owned flake inputs · #924 fix(pm-stack): restore fleet-feed collector units · #923 cold-store-reaper v1 · #921 config(opencode): comment out cloudflare/openrouter sections · #920 impl(dashboard): slice 3 — pluggable pages, DNS register, comments, session registry (#906) · #919 fix(id-web): safe blob content types (#908) · #915 config(opencode): tier route pools over per-model plan pools · #914 ci: flake-inputs-update hand-off · #912 chore(deps): update openchamber + opencode pins · #909 fix(id): carry /logs seconds in SSH command mode (#819) · #907 ci: scheduled flake-inputs-update workflow · #905 fix(id): repair NixOS VM checks · #904 config(opencode): reconcile deployed config into repo · #901 feat(aggregator): slice 2 — mobile, needs-Drew/orchestrator tiers, roadmap horizons, today view · #900 feat(aggregator): orchestration section + /api/orchestration.json · #892 feat(aggregator): slice 1 — live needs-you + quota panels · #891 fix(pm-stack): connect-repo.sh SyntaxError (#877) · #889 fix(home): dead opencode env vars (#698) · #888 fix(id-web): codec round-trip · #887 config(opencode): cline-pass provider rename · #885 docs(opencode): muse provider comment · #878 fix(id-web): img attribute codec (#872, #873) · #875 claude-code-proxy: resident stream-json bridge (#480) · #874 ci(id): PR flake checks as parallel matrix (#869)

### W44 (2026-10-26..2026-11-01)

#4075 fix(id): discovery rejects apps whose program.source does not exist (#4064) · #4058 feat(pm-dashboard-backup): per-step run ledger (#1854) · #4054 feat(aggregator): roadmap annotations (#1284) · #4053 feat(aggregator): bulk triage mode (#1277) · #4049 fix(proxy-supervise): validate config (#1952) · #4048 feat(id): signed board listings + native-host replication (#3806) · #4046 fix(id): tag-key decoders (#1832) · #4045 fix(id/server_config): reject blank declaration keys (#1824) · #4032 fix(pm-dispatch): symlink confinement (#3999) · #4031 fix(id): bound directory mail runtime (#1788) · #4020 pm-stack(grist): scripted Choice-option updates (#1287) · #4014 fix(pm-stack): restart ready-for-agent · #3996 fix(quota-gatherer): bound provider concurrency (#1268) · #3988 docs: pm-stack forecasting design (#1256) · #3987 feat(aggregator): comments queue view (#1210) · #3985 fix(aggregator): error boundary, fail-soft (#1211) · #3971 feat(id): signed themes + layered board views (#2813) · #3968 fix(id): roc-test recipe via bash (#813) · #3956 feat(id): chat presence heartbeat over gossip (#2809) · #3953 test(id): world snapshot determinism across architectures · #3943 feat(aggregator): report collector outcomes · #3941 feat(id): chat.message.v1 shape + causal projection (#3799) · #3938 fix(aggregator): Kaneo creation time (#1197) · #3936 feat(aggregator): /roadmap drill-down (#1164) · #3935 feat(aggregator): epic rollups (#1164)

### W45 (2026-11-02..2026-11-08)

#7857 docs(id): board moderation hooks decision (#5861) · #7851 docs(id): visual-feature cost ranking (#6012) · #7848 doc(id): thread resolution decision (#5874) · #7847 mcpx v2: daemon picker tool (#2841) · #7842 fix(atlas-search,fleet-search): fail loudly on unset HOME (#5640) · #7836/#7822 fix(id): duplicated ChainExt fields (main build break) · #7835 fix(aggregator): OpenCode Go quota attribution · #7834 fix(opencode-claimant): handed-off tasks bind + execute · #7832 openclaw: remove claude ACP agent (code#7098) · #7830 config(opencode): OpenCode Go account developing-today-0 · #7829/#7820 yard reaper (#7093) · #7827 fix(id/web): validate /api/upload shape (#7823) · #7824 fix(agent-backup): snapshot Claude Code credentials (code#7098) · #7817 feat(amd): wvkbd on-screen keyboard tied to Sunshine · #7816 fix(fleet): two main build breaks · #7815 fix(id/web): validate /api/images payload · #7810 fix(atlas-search): proxy env in via-service client (#5597) · #7809 docs(atlas): Presidio NER pass design (#6048) · #7793 fix(mcpx/plugin): sync dogfood .opencode copy (#7791) · #7790 ci(scripts): provider retirement gate suite (#7789) · #7779 docs(id): client surfaces parity decision (#5820) · #7776 fix(aggregator): plan-aware Turso limits · #7774 doc: backup/recovery/exit decision for serverless storage (#5869)

### W46 (2026-11-09..2026-11-15)

#10570 fix(opencode-config): unseat groq-free-plan (#10562) · #10566 fix(openclaw): drop copilot from runtimePlugins · #10565 fix(mcpx): sweep build temp dirs (#10554) · #10564 config: restore _claude/_muse/_commandcode doors · #10561 config: Z.ai (GLM) free-tier door · #10560 fleet-queue: claim-link rollup over #9046 link (#10357) · #10558 Enable Groq, NVIDIA NIM and OpenRouter free tiers · #10557 config: rename system1 → systemone; beam-plan fronts workhorse · #10556 config: un-disable _commandcode · #10555 fix(aggregator): orchestration visible with blocked dependencies · #10553 fix(opencode-config): spark/sol rungs, luna subscription-first · #10552 nixos: sandbox-link-tunnel (code#10356) · #10550 fix(opencode-config): mimo free leg leads its plan · #10548 fix(id): unbreak main checks (#8904) · #10543 atlas/fleet-search: extract server feedback DTOs (#10446) · #10536 feat(id-native-shell): live WorldClient session loop (#788) · #10535/#10532 fix(pm-stack): kaneo-sync-guard heal (#10528, #10527) · #10531 feat(fleet): normalize attested quota windows · #10530 doc: pm-kaneo-sync-guard timer in fleet schedule registry (#10529) · #10526 feat(id-native-shell): step-2 Screen renderer (#788) · #10520 fix(dependabot): bun ecosystem for pm-stack · #10514 chore(deps): fork pin updates · #10512 feat(fleet): exact quota applicability for shadow preflight · #10509 aggregator: orchestration reader flip (#10504)

### W47 (2026-11-16..2026-11-22)

#12717 fleet: machine registry artifact, amd-6 first record (#2179) · #12715 fix(pm-console): refuse bad reports unread · #12714 fix(mcpx): seed records on one line · #12713 registry: remove issue-sweep-rotating row (#2138) · #12712 feat(pm-console): owner's switch for Cloudflare · #12711 feat(archer-slack): Socket Mode collector + #2137 design record · #12710 fix(id): shell contract — join.device_input nullable · #12709 feat(pm-stack): outcome attribution projector (#2072) · #12708 id: note policies survive host restart (#3819) · #12707 test(id): first app PoC — sticky notes + did-it tracker (#3790) · #12706 feat(fleet-queue): coordination verbs through route_for (#8581) · #12704 fix(id): reconnect resume retains baseline (#12648) · #12702 design(quota-plane): Stage 3 predictive quota operations (#1177) · #12693 fix(main-broken): serialize filing per Breakage-ID (#1782) · #12692 feat(fleet-queue): durable SQLite coordination replica (#8583) · #12691 feat(pm-stack): dashboard comments → shared_comments (#1744) · #12690 fix(fleet-agent): agent next claims with 4-hour lease (#12618) · #12689 feat(pm-stack): quota gatherer writes babysitter pool-state feed (#1338) · #12688/#12687 claimant/babysitter verdict rounds (#1187, #1338) · #12686/#12685 queue-backup: Turso → D1 export + restore drill (#8584) · #12684 fix(fleet): keep ureq out of connect binary (D7 size gate) · #12683 fix(ci): production line guard green · #12680 fix(ci): id-check full tier timeouts/watchdog (#12658)

### W48 (2026-11-23 only)

#13870/#13868/#13867/#13865/#13863 migrate mcpx/ID/fleet/CNCF/Atlas investigation research from hatch-personal · #13869 bootstrap: pin fleet-static-f56dc7f (tiered actions-exec, code#13774) · #13862 impl(id): WebSocket/WSS + HTTPS fallback adapter (#8215) · #13861 docs: dated-vs-living taxonomy, doc/living/ home · #13857 fix(pkgs/cf): vendor package.json (#13853) · #13854 fleet actions exec: tiered lease-extension policy (code#13774) · #13848 doc: p2p composition options addendum (#13779) · #13845 doc: Turso → D1 migration audit + programme design (#13844) · #13838/#13836 opencode: openai-payg-free alias (code#13677) · #13837 feat(id-native-shell): client.notify executor (#10930) · #13835 id: LocalEvents holds node store (#5534) · #13834 mcpx script timeout oracle in non-reaping containers · #13832 feat(id-store): block-content projections (#8218) · #13830 research(id-atlas): store ownership + identity seams · #13828 docs(planning): roadmap files for fleet and Atlas (#13497) · #13827 doc: CNCF landscape research record · #13825 id: read-only petname list review surface (#4974) · #13823 docs: Rust practices record · #13819 id: grant expiry bound + grantor veto · #13818 doc(schedule-registry): main-ci watch revived (#10917)
