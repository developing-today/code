# Lineage manifest — every issue linked to epics, boards, and labels

Produced 2026-11-12 from Drew's directive: "fix the lineage so that this work and
all work is linked to one or more epics and dedicated project boards and labels."
Companion to pm-stack/design-tracking-manifest-2026-11-11.md and
pm-stack/design-reprioritization-2026-11-12.md. Method: `gh` CLI only; every
write below was verified by reading it back (in several cases twice — the
sub-issues API paginates at 30 by default, which masked results on first read
twice during this pass and was caught both times).

## The taxonomy (standing)

- **Epics.** An epic is an issue carrying the `epic` label. Membership is
  two-sided: the member carries the epic's `epic/<name>` label ("all issues of
  this effort carry it") AND, wherever the epic issue is unambiguous, the member
  is a **native sub-issue** of the epic issue. Program epics from the
  2026-11-12 reprioritization: `epic/id-program`, `epic/fleet-orchestration`,
  `epic/mcpx-platform`, `epic/opencode-platform`, `epic/pm-stack`,
  `epic/atlas-program`, `epic/lab-cluster`, `epic/github-dynamic-compute`,
  `epic/turso-cloudflare`, `epic/saas-service`, `epic/storage`.
- **Boards.** Org GitHub Project "Developing Today project" (developing-today
  org Project #4) with Area/Status/Horizon/Kind fields; Area is the split
  dimension and dedicated boards are filtered views over the one project
  (pattern from code#910). Lab-cluster boards are the exception and belong to
  the Clustered-Computing org (Drew, 2026-11-12).
- **Labels.** `epic` + `epic/<name>` (epic), `app/<name>` / `area/<name>`
  (area), `kind/<name>` (type), `design/<name>` (design effort),
  `effort/xs|s|m|l`, plus status/queue/source families. New labels are created
  only to mirror an existing fleet label (same name/color/description).

## BOARD STATUS — BLOCKED on a token scope grant (needs Drew)

Every Project-board read and write in this pass failed with
`INSUFFICIENT_SCOPES`: the automation `gh` token carries
`admin:org, gist, repo, user, workflow` and **no `project` / `read:project`
scope**. Labels, sub-issues, and epic creation are done; board membership,
Area/Status/Kind field values, and the dedicated ID-program view could not be
set or even verified. Remedy (interactive, Drew only):

```
gh auth refresh -s read:project,project
```

Follow-up pass owed once granted: (1) add the crate-split chain (#2056, #3195,
#3447, steps) and every epic issue in this manifest to Project #4 with Area set,
and set Project `Kind=Epic` on each epic item (per `pm-stack/README.md` and the
epic-intake template); (2) create/verify the ID-program dedicated view on Project #4;
(3) fleet-wide board-membership audit against the epic taxonomy below; (4)
lab-cluster boards in the Clustered-Computing org.

## Phase 1 — the pkgs/id crate split (complete except boards)

Native sub-issue chain, verified via both each parent's `sub_issues` list and
each child's `parent_issue_url`:

```
#2056 (ID epic) → #3195 (build-time review) → #3447 (crate split)
  → #4403 #4405 #4406 #4407 #4408 #4409 #4411 (steps 1–7)
```

Before this pass, #3447 and #3195 had **zero** native sub-issues and #2056's
children were only #2026–#2054 — the split was linked in prose only.

Labels created (mirroring the fleet `epic/*` / `design/*` families):
`epic/id-crate-split` (5319e7) and `design/id-crate-split` (1d76db). Verified
label sets after the pass: #2056 = `app/id, epic, epic/id-platform,
epic/id-program`; #3195 = `app/id, epic/id-program, epic/id-crate-split,
design/id-crate-split`; #3447 = same + `epic`; all seven steps = `app/id,
epic/id-program, epic/id-crate-split, design/id-crate-split`. Lineage recorded
in comments on #3447 (issuecomment-6832720755) and #2056
(issuecomment-6832721881).

## Phase 2 — per-repo results

### developing-today/code — 1,070 → 1,073 open (concurrent fleet activity during the run)

| Coverage            | Before      | After       |
| ------------------- | ----------- | ----------- |
| ≥1 `epic/*` label   | 873 (81.6%) | 929 (86.6%) |
| `app/*` or `area/*` | 629 (58.8%) | 628 (58.5%) |
| `kind/*`            | 666 (62.2%) | 732 (68.2%) |
| All three           | 436 (40.7%) | 439 (40.9%) |
| Epic-marker issues  | 42          | 46          |

Writes (all read back): 67 mechanical `kind/*` labels (legacy `bug`/
`enhancement`/`design`/`documentation` → prefixed forms); 2 native links
(#5536, #1240 → #2056; the other unique-epic clusters were already fully
parented — notably 30 id-platform children sit under closed epic #6312);
4 new epics for coherent unplaced clusters ≥8, children labeled + natively
linked (54 total, 0 failures):

| Epic                            | Label                    | Children |
| ------------------------------- | ------------------------ | -------- |
| #8976 main-broken CI reports    | `epic/main-broken`       | 20       |
| #8977 crate splits (other pkgs) | `epic/crate-splits`      | 16       |
| #8978 daily maintenance         | `epic/daily-maintenance` | 10       |
| #8979 scheduled tasks           | `epic/scheduled-tasks`   | 8        |

**Ambiguous epic labels in code** (native linking not possible without Drew's
pick of a single home epic): multiple epic-marker carriers — `epic/lab-cluster`
(11), `epic/fleet-orchestration` (8), `epic/pm-stack` (7), `epic/id-program`
(6), `epic/mcpx-platform` (5), `epic/pm-stack-coherence` (2),
`epic/opencode-platform` (2). Labels with **no epic issue at all**:
`epic/atlas-program` (68 issues), `epic/github-dynamic-compute` (28),
`epic/design-register` (27), `epic/turso-cloudflare` (16), `epic/storage` (11),
`epic/atlas` (8), `epic/saas-service` (6), `epic/fleet-tool` (6),
`epic/pool-routing` (5), `epic/fleet-apps` (2), `epic/mcpx` (2). Creating
program epic issues for the zero-carrier labels — atlas-program first (68) —
is the single highest-leverage follow-up.

### developing-today-forks/opencode — 66 → 67 open

No epic issue existed despite a coherent 39-issue `epic/opencode-platform`
cluster. **Created #2319 "epic: opencode fork — platform work"** (`epic` +
`epic/opencode-platform`, read back twice); **38 of 39 members natively
linked** (verified: exactly 38 sub-issues at per_page=100). #1557 already
parented under #1684 — not re-parented. 5 mechanical `kind/*` labels applied
(kind coverage 57 → 62). No labels needed mirroring — the repo already carries
the fleet taxonomy. Unplaced: 27 (appendix B).

### developing-today-forks/openchamber — 67 → 68 open

Same shape: `epic/opencode-platform` label (26 members) but no epic issue.
**Created #2153 "Epic: OpenCode platform"** (`epic` + `epic/opencode-platform`);
**24 native sub-issues linked**, read back exact (#1744/#1747 skipped — already
parented under #1808). 3 mechanical `kind/ci`/`kind/bug` labels (kind coverage
62 → 65). Unplaced: 41 (appendix C), including a flagged decision: 8
pr-shepherd superseded-PR closure chores were NOT epic'd unilaterally (not a
coherent deliverable), and #2097/#2098 look like duplicates (same PR #2037).

### dezren39/hatch-personal — 21 open (inventory only; personal repo)

Epic coverage is near-total via labels (19/21 carry `epic/*`) but the taxonomy
is a thin slice: 22 labels total, `epic/*` + a single `kind/enhancement`, **no
`app/*` or `area/*` at all**, and the "epics" are label groupings / shipped-
design tracking homes (`epic/atlas` 15 issues, `epic/design-register` 10) with
no in-repo parent tracker issue. Unplaced: #10 (hatch-autocommit wedge —
fleet/autocommit infra, no matching epic), #12 (repo hygiene notes).
Recommendation: mirror the `app/*`/`area/*`/`kind/*` families here and file one
tracker issue per label epic when Drew wants this repo fully lined up.

### dezren39/darwin-nix — 38 open (FROZEN — inventory only, no writes)

Full fleet taxonomy already present (160 labels). Real tracker epics exist:
#973 (SaaS deploys, `epic/saas-service`), #672 (2026-10-19 reliability review,
`epic/mcpx`), #1473 (hermetic harness, `epic/github-dynamic-compute`).
Tallies: epic-labeled 17/38, kind 27/38. The gap is 21 issues with no epic
label, concentrated in the recent `area/scripts` bug batch (#1828–#1843,
#1759–#1836 range) — candidates noted per issue in the run inventory
(epic/storage, epic/opencode-platform, epic/fleet-orchestration,
epic/github-dynamic-compute) but NOT applied: the repo is frozen and several
mappings are genuinely ambiguous. Unfrozen triage can place most of them from
the recorded candidates.

## Unplaced remainder (for Drew)

- **code: 144 open issues currently carry no `epic/*` label** (appendix A;
  fresh pull at manifest time). Of these, #6093 and #1815 are natively parented
  (#6185, #1817) despite no epic label. The dominant cluster is the **pkgs/id
  backlog: 40 issues** (37 carry `app/id`) — heterogeneous backup/chat/web/
  libsql/world work with no single effort theme. It was deliberately NOT
  force-assigned to `epic/id-program`; it needs ID-program triage (split into
  effort epics vs. assign).
- **opencode fork: 27** (appendix B) — an 8-issue design/façade cluster
  (#587, #800, #1146, #1677, #1678, #1680, #1684, #1823) that looks
  platform-shaped but carries no epic label; labeling designs by inference was
  judged too aggressive for this pass.
- **openchamber fork: 41** (appendix C).
- **hatch-personal: 2** (#10, #12). **darwin-nix: 21** low-confidence (frozen).

## Appendix A — code: open issues with no `epic/*` label (144, at manifest time)

- #8975 perf(id/app-manifest): avoid quadratic route collision scans
- #8973 queue: raise default max_attempts for new tasks to 1000 (Drew order 2026-11-12)
- #8970 fix(id/serve-proxy): bound credit grants to flow-control limits
- #8951 fix(id): validate serve-proxy frame stream and sequence semantics
- #8950 main CI failure: developing-today/code — main broken watcher (run 43264623316)
- #8947 ci-fix: nix ci failing on main
- #8945 ci(workflow-lint): ci-policy agreement contract test fixtures are stale after the #7089 re-arm
- #8943 id(backup): enforce a passphrase floor in create_backup
- #8938 id: web and native-shell form widgets for form_result (#2340 follow-up)
- #8936 nixos: amd host build fails — openclaw runtimePlugins has unsupported id memory-wiki (switch blocked since gen 554)
- #8932 id: wire a shipped form presenter + presenting-device manifest for form.* (#2340 follow-on)
- #8925 openai.alternate final reserve burned +16pp weekly in ~1h, unattributed (steward run 316, 2026-11-11)
- #8900 fleet-stalling: main mcpx test failures block clean PR checks (run 43191539255)
- #8899 pm-stack(opencode-usage-export): reject duplicate value options instead of silently retargeting output
- #8887 fleet gh: one passthrough that rotates personal + App tokens for every fleet gh call
- #8880 fix(id/web): reject malformed bracketed IPv6 Origin authorities
- #8863 id(world): program install writes an unrestorable event at sequence MAX
- #8852 id(logging): define a bounded retention or rotation policy for ID_LOG_FILE
- #8851 id: boundary enforcement inputs — declaration residence, evidence sources, verifier (#7787 parts 2-4 prerequisite)
- #8846 id(web): avoid reporting success when restoring an absent file
- #8843 id(envelope): Envelope::signed accepts a from that can never verify — refuse a sender/key mismatch at sign time (gap from #6488)
- #8842 id(backup): sign_upload signs envelopes every store refuses — mirror validate_envelope at sign time (gap from #6488)
- #8815 fix(id-sync): prevent forged archive refs from deleting another file's archive
- #8803 id(backup): warn on long single-character passphrases
- #8801 pkgs/id: probe_attestation_sign test does not compile on main — sign_claim gained a GenesisAgreement parameter
- #8798 id(libsql): limit half-open circuit breaker to one recovery probe
- #8795 pkgs/id: web lib does not compile on main — crate-split visibility left 5 unresolved/private sites in src/web
- #8794 Route the remaining duplicate-key gate sites through parse_json_without_duplicate_keys (Phase 4 follow-on of #6521)
- #8787 Attribution: declarative auditd for signal syscalls on the laptop (follow-up to #8777)
- #8773 id(repl): bound shell-capture output and execution time
- #8756 pkgs/id: probe_attestation_sign test target does not compile on main (sign_claim gained &GenesisAgreement arg)
- #8743 id(chat): scope join-event selection to the requested group
- #8729 opencode.service ExecStart store path was garbage-collected under the running server — any restart is a dead server
- #8716 id/web: search-panel accessibility test fails on main (role=search / aria-label)
- #8620 pkgs/id: frames byte_refusal test fails deterministically on main (RequestTooLarge vs BudgetExhausted)
- #8605 needs-drew: close the alert incident when the question is answered
- #8574 pkgs/id: split dependency compilation out of the Nix build with crane (code#8392 follow-up)
- #8545 id: turso_client pipeline chunks lose the session (rde_incoming vanishes mid-load)
- #8539 id(libsql): accept chunk extensions and reject incomplete HTTP chunk framing
- #8538 id(serve): seitan admission vs durable world directories — AccountVerified lands in the server-config journal, not the live world directory (Codex P1 follow-up to #8529)
- #8524 id(web): stale saves replace the current filename without archiving its version
- #8509 id is failing
- #8495 id(web): distinct colon-containing tags collide in home feed aggregation
- #8459 id(world): concurrent journal handles can acknowledge appends to a replaced inode
- #8428 lab-ops-hub(container): symbol-host parity conflict — #588 category check refuses what #659 test and the Worker accept (😀.com)
- #8427 openclaw: off-box gateway-death watch — surface a dead gateway in the main chat (#6167 §5)
- #8383 id(web): bootstrap redirect can navigate to another origin for double-slash paths
- #8376 id(sanctions): bind decisions to the declared active list and its matches
- #8360 id(web): collect link reference definitions indented relative to a list item's content column
- #8354 id: frames byte-refusal test fails on main (RequestTooLarge vs BudgetExhausted) — blocks id build + nixos-integration check
- #8316 id: world_bench fails at restore — journal event at sequence 1 uses participant 1 without INPUT scope
- #8314 ACP adapter: unanswered permission request leaks pendingClientRequests entry and is never replied (#4707 audit gap b)
- #8313 ACP adapter: serve-side sessions are never deleted — sessions map has no close path (#4707 audit gap a)
- #8306 fleet-search parity guard red on clean main: atlas-search and fleet-search server.rs have drifted (blocks clean CI signal for search PRs)
- #8290 Remove the inert proxy-supervisor fossil unit files from the laptop user unit dir (from #2775)
- #8221 home-resolution contract: disposition the Python expanduser / Path.home() surface against #5762
- #8220 home-resolution contract: disposition the os.homedir() surface against #5762
- #8182 power-guard: release() forgets a still-running systemd-inhibit child and logs success — sleep stays blocked with no cleanup retry (PR #8025 review finding)
- #8173 id crate test failure blocks NixOS deploys from main: version_probe_records_the_reported_version records empty version
- #8163 _openai provider: self-contained main OpenAI door that does it all without Codex (Drew top priority)
- #8152 id(roc-world): platform host build.zig fails on installed Zig 0.17 (Optimize.Debug removed) — host.wasm cannot be built
- #8148 Weekly housekeeping cleanup
- #8094 Android client design: a consumer-specific grant denial suspends the provider capability for everyone (PR #7826 review finding)
- #8064 id DoS catalogue: deadline scenarios cannot fit the fixed 30s attack profile — SSH inactivity timeout is 10 minutes (PR #7738 review finding)
- #8057 currency gateway: Balance/History reads rejected for frozen/sunset currencies (PR #7650 review finding)
- #8056 currency gateway: registration cap u128 lacks the fixed-width u128_bytes CBOR adapter (PR #7650 review finding)
- #8043 Close three false Globalping incidents on /alerts (2026-11-09 05:41 CST fabricated-result posts; services verified healthy)
- #8012 test(id-web): search-panel role=search/aria-label test fails on clean main
- #7970 id: replication admission — accept does not check per-entry version requirements (Phase 3 audit gap from #4593)
- #7931 op-env.sh: resolve bws from PATH and fail loudly naming the step — hardcoded ~/tools/bin/bws does not exist on the laptop (#6829 fix)
- #7892 id(world): reject invalid chat.history count types
- #7880 meta-exporter: restore the #2789 plain-decimal gate lost in the #6333 re-home (#3005 grammar drift)
- #7841 id(web/main): /api/tags response shape-trusted in loadFileTags — a wrong-shape 200 shows the file as untagged, leaves the previous file's tags on screen, or renders '[object Object]' pills
- #7831 id(web/world): /api/world/host-view and /api/world/invite responses shape-trusted — a wrong-shape 200 shows raw TypeErrors or stores a fake 'undefined' capability
- #7811 id(web/home): home feed refresh shape-trusts /api/home/feed — a wrong-shape 200 wipes the rendered lists and surfaces raw TypeErrors
- #7557 id(web/markdown): #3746's code-span tracking never resets — one unmatched backtick disables newline-destination protection for the rest of the document
- #7297 Unowned PR batch from branch disposition (2026-11-07)
- #7098 Session error wave 2026-11-07: _claude OAuth credential blanked at 19:02 CST — 49 sessions dead, ongoing
- #7011 investigate: CPU/memory load over time + parallelism sweet spot on the sandbox VM
- #6992 pm-dispatch: POST /v1/sessions and /v1/tasks answer HTTP 502 — daily upstream sync dead since 2026-11-04
- #6985 queue: task 11164 claimed by two lanes concurrently (code-a and code-x) — claim atomicity violated
- #6936 github-ci(worker): HMAC gate fails open when GITHUB_WEBHOOK_SECRET is unset — unsigned POSTs are processed and written to Turso
- #6935 DIY backup api transport: platform schemas (auth/storage/realtime/vault) deliberately excluded — recorded decision
- #6853 queue: claim_repo_task rejects lane openchamber-a (unknown lane_id) — OpenChamber hook worker cannot claim
- #6829 laptop op-env.sh: 1Password service-account token fetch from Bitwarden fails (JSON parse of empty output)
- #6823 id(web): no property test for the footnote-reference plain-text serialization invariant (PR #6427)
- #6797 infra: laptop /home at 100% — fleet worktree cargo targets fill the disk, builds die with ENOSPC
- #6767 fleet(secrets-broker): serves stale secret value long after BWS rotation — read-through cache never invalidates
- #6534 power-guard: a dead systemd-inhibit child is never noticed — held stays true forever, engage() is a no-op, and the machine is silently unguarded
- #6464 Queue backend outage 2026-11-06: muse-dev2 inaccessible (544 → 522 → 503 → 400), all queue verbs blocked
- #6462 clippy-lint gate red since 2026-11-02: 571 deny-level sites in pkgs/id (campaign)
- #6384 deploy(deploy.py): duplicate JSON keys in the registry are silently discarded at load — an invalid first definition is never validated
- #6181 security(identity): keep passwords and JWTs out of authentication logs
- #6163 id: render portable board themes safely in web and terminal (#5525)
- #6135 id(currency_poc): InMemoryLedger accepts and posts zero / int-max account and transfer IDs real TigerBeetle refuses — the reference backend is a broken oracle for ID validity
- #6126 pm-aggregator-tests workflow: auto triggers BLOCKED on runner strategy (dispatch-only by Drew's ruling 2026-11-05)
- #6118 muse-proxy attestation aliasing (shell/bash renames) has collision, leak, and bypass modes
- #6093 pkarr-worker: test suite unrunnable from a clean checkout — lockfile pins vitest 3.2.7 vs package.json ^4.0.0 (npm ci refuses), and vitest 4 breaks the pool-workers runner (0 tests run); no workflow runs it either
- #5947 lab-ops-hub: container validator refuses emoji/symbol IDN hosts the Worker accepts — pinned WHATWG-alignment test fails on main
- #5860 fleet(secrets-broker): SECRETS_BROKER_TTL_S=0 makes the vault-refresh loop spin — ~110 full bws refreshes/sec, no startup validation
- #5740 worktree-reaper: fail closed when /proc cannot be scanned before removal
- #5696 design: eBPF on the laptop (privileged host) — observability deferred with triggers, data-path acceleration rejected
- #5477 laptop(amd): SDDM login never unlocks GNOME Keyring - enable pam_gnome_keyring so the keyring-held gh token survives reboots
- #5447 chore(deps): remove the nixpkgs-25 regex-pcre pkg-config/pcre override once stock regex-pcre builds at the channel rev
- #5415 Laptop remote desktop rotted: KRdp logons all fail — krdpserver's KWallet user store never loads under the Hyprland session
- #5328 Backfill AA v5 and v4.3.2 into ainventory.measurement_events (#4401 step 2)
- #5272 Consolidate Dependabot PRs #3304 and #3307: conflicting vitest lines for pkgs/pkarr-worker (5.0.5 vs 4.1.12 + pool-workers 0.23.0) — land one coherent set
- #5121 meta-quota-export: every run fails when no dev.meta.ai tab is open in the laptop browser
- #5047 design: a name is claimed once — the hosted binding lifecycle contract (pkgs/id hosted_binding)
- #4889 append-usage-snapshots.ts fails on laptop: quota-view.md path ENOENT under ~/workspace
- #4837 id: generic event-body persistence, retrieval and range-sync — prerequisite for the didit tracker phases (#2031)
- #4818 default devShell unbuildable: cargo-llvm-cov 0.9.0 test suite fails under cargo 1.99.0
- #4604 Standing sweep: git-hygiene-sweep job — publish reconciliation + idle build-artifact cleanup (tracking)
- #4445 stale-pr: resolve code#3102 — Fix request-scoped direct tool aliases
- #4390 pool flag: re-verify terra-plan #medium seat premise — AA v5.0 reads Terra medium TTFT 4.90s, not 1.43s
- #4300 laptop meta provider: 403 'API key is billed to a Muse Code subscription' — sole-door Meta pool route down since 2026-10-30
- #4277 chore(deps): remove the cargo-llvm-cov llvm override once its pinned llvm matches rustc's profraw format
- #4167 chore(deps): remove the awscli botocore/s3transfer pins once nixpkgs' awscli pairing is consistent again
- #4000 Design: reconstruction is admission, continued — carried state, checked succession, and every restore's guards (pkgs/id follow-on to #2898)
- #3792 id: match attestation v1 — signed result objects, state machine, counting views in pkgs/id (from #2039)
- #3789 id(trust): writer-set + view projection in pkgs/id (Phase 2 of #2031)
- #3735 Switch pool-review consumers to conductor-log
- #3570 Preserved branch: fix/summarize-got-hash — decision needed (merge / cherry-pick / drop)
- #3418 design(queue): budget-aware claiming — parks carry a minimum required time budget, claim matches the claimer's remaining shift budget
- #3205 Standard task: per-repo hygiene census + cleanup documentation (low priority)
- #3150 investigate(routing): uncensored fallback lane (Sable) for content-policy rejections — long-term topic
- #2898 Design: reconstruction is admission — validate authority-bearing inputs where they are rebuilt (pkgs/id)
- #2875 openclaw: gateway home module applies to all hosts but sops secrets are amd-only — audit/scope the Intel host
- #2775 Survey: laptop services & proxies — declarative coverage and conversion backlog
- #2624 commandcode-proxy unit dies when its nix store start-wrapper is GC'd — point the unit at the checkout script
- #2581 guard: generated proxy units should not silently resurrect removed dependency directives on merge
- #2170 Add OpenClaw to #alignment — decide one shared channel vs point-to-point
- #2101 commandcode-proxy: MAX_TURNS / AGENCY_MAX_TURNS forwarded to the CLI verbatim — 'abc', '0', '-1' or '2.5' in the env corrupts only the spawn fallback, never startup
- #1815 Research: boats.dev
- #1799 sync-models: existing entries are never reconciled when limit.context already matches — a wrong limit.output and stale variants survive every sync
- #1687 power-guard: gate UPower's critical-power hybrid-sleep behind a sustained-reading confirmation (laptop slept 2h40m on a glitched battery reading, 2026-10-28)
- #1558 fix(openclaw): PR #1242's sops-nix secrets landed on a stranded feature branch — not on main, and the promised /run/secrets/openclaw wrapper repoint was never filed
- #1528 stewardship: Muse subscription panel has no owning read lane — flagged-stale 90% figure survived 24h+ and closed the pool by rule
- #1518 Define code/config embeddings: unit, metadata, usage — and whether they share the prose embedding dump
- #1319 Sandbox posture: sessions + ssh + browsing only — no servers; stay aligned with laptop state
- #1280 Home network: IPv6 uplink stalls on payload transfers (PMTUD/router/ISP)
- #861 Rotate the agent-auth Meta secret at the source (sops still encrypts the stale key)
- #531 lab-ops-hub: container validator blanket-refuses CONTEXTJ-valid ZWNJ/ZWJ hosts (virama and Arabic-script joining contexts) the Worker validator accepts
- #176 Nix Playwright E2E suite fails after vendoring dependencies

## Appendix B — opencode fork: unplaced (27)

- #587 design(core): one quota signal, three consumers — shared QuotaSnapshot and weight-layer contract for #566/#567/#568
- #800 design(codemode): Code Mode's missing standard library — one versioned std namespace for execute, import maps stay out of the interpreter
- #893 stale-pr: resolve opencode#233 — feat(core): pick pool members by pattern and control effort per level
- #1146 Design: generic OpenAI façades on the server — /v1/chat/completions + /v1/responses, one translation layer, proxy doors retire
- #1300 stale-pr: resolve opencode#777 — fix(server): close the serve handover teardown race
- #1634 main CI failure: .github/workflows/agents.yml (run 41845324868)
- #1654 chore(ci): close superseded open PRs #1497, #1566, #1504 — work landed via #1576 and #1594
- #1672 main CI failure: Daily Upstream Sync (OpenCode) (run 41903121391)
- #1677 Design: search providers behind the façade — one common search endpoint vs per-provider adapters per tool
- #1678 Design: voice input in OpenCode — speech-to-text providers behind the façade (/v1/audio/transcriptions shape)
- #1680 Design: custom + live audio through the façade — custom formats and realtime bidirectional (duplex) audio
- #1684 design(ai): translate the constraint, or say so — one fidelity contract for provider tool-schema projections
- #1823 design: provider naming & ownership convention — mark fleet-built providers (dt- proposal), rename map
- #1972 fix(ci): repo-sweeper daily stale-issue close always fails — close-issues.ts requires GITHUB_TOKEN, workflow provides only GH_TOKEN
- #2204 fix(script): raw-changelog still targets the upstream layout — the fork's packages never reach the changelog, and the default repo is anomalyco/opencode
- #2268 fix(server): reject weak validators for If-Range requests
- #2274 fix(core): materialize remote and managed URIs in completed tool-result history
- #2281 fix(server): compare Basic-auth passwords in constant time
- #2284 fix(server): drain active PTY WebSockets during graceful shutdown
- #2286 fix(core): symlinked workspace paths bypass external-directory approval
- #2289 fix(plugin-browser): authorize server-local file uploads
- #2302 feat(tui): highlight embedded JavaScript and CSS in HTML
- #2310 fix(codemode): block model fetches to private network targets
- #2313 fix(ai): in-process Secret Service lookup dropped the DBUS_SESSION_BUS_ADDRESS fallback — keyring reads silently fail in session-bus-less processes
- #2314 fix(server): bound JSON request bodies before buffering
- #2315 fix(core): reject non-positive quota window durations
- #2317 fix(core): a package-conflicted document's baseURL still strips the seated provider's per-model catalog baseURL pins (#2210 residual)

## Appendix C — openchamber fork: unplaced (41)

- #80 Anti-slop lint backlog: 9,498 findings in 1,086 files, tracked by phase
- #152 Tracking: threads from the 2026-10-20 config, goal editor and integrations session
- #224 Back to edit on a new chat can replace the session the user moved to with a new draft
- #322 Goal writer card says "The message was not sent." above reasons that say it may have been sent
- #937 stale-pr: resolve openchamber#216 — feat: goal writer card, model pool checks, Answer again on questions, project visibility and UI fixes
- #1437 Anti-slop phase 1: ~499 mechanical findings (separate statements, renames, named owner types)
- #1438 Anti-slop phase 2: ~1,799 owner-type findings (unknowns, dictionaries, chained assertions, widening)
- #1439 Anti-slop phase 3: ~7,091 design-pass findings (runtime typeof, safety comments, module mocking)
- #1450 main CI failure: .github/workflows/jules-daily-upstream-sync.yml (run 41633212191)
- #1570 chore(deps): automated upstream sync failing
- #1587 main CI failure: opencode-update (run 41850272006)
- #1642 main CI failure: .github/workflows/agents.yml (run 41997940196)
- #1768 fix(ui): Linear workflow-states truncation signal statesMore reaches the UI type but no consumer — the Source Board state picker silently shows a first-page-only list
- #2055 chore(ci): scripts/test-mobile-workspace-resize.mjs runs nowhere — the mobile-fold check rots silently
- #2056 chore(ci): flake checks.start-wrapper-guard runs nowhere in this fork — the code#2562 regression pin has no tripwire
- #2057 fix(scripts): updater-e2e-fixture stage silently swallows --port (and any --key typo)
- #2064 fix(scripts): release build test still targets the removed Tauri package
- #2069 pr-shepherd: close PR #1958 as superseded — its entity-v2 scratch-tree bound landed via PR #1968 (#1814 closed)
- #2073 pr-shepherd: close PR #1967 as superseded — its host-shutdown store-spy fix landed via PR #1974 (#1796 closed)
- #2077 pr-shepherd: close PR #1988 as superseded — its queued-message preview surrogate cuts landed via PR #2001 (#1959 closed)
- #2081 One-queue lane designations for OpenChamber scheduled tasks (Drew 2026-11-10, code#8097)
- #2097 pr-shepherd: close PR #2037 as superseded by merged PR #2039 (#2028 closed)
- #2098 pr-shepherd: close PR #2037 as superseded by merged PR #2039 (#2028 closed)
- #2103 fix(web): concurrent login attempts exceed the per-IP failure budget
- #2105 ci: include Knip classMembers in the dead-code check
- #2111 fix(web): do not advance durable replay state after an append failure
- #2113 fix(web): bound project-context todo IDs before writing context.json
- #2115 fix(web): scheduled-tasks run-history.js contains literal NUL bytes — git/GitHub treat the source file as binary, hiding its diffs
- #2120 fix(web): make scheduled run-history compaction failure-atomic
- #2123 fix(web): reject repeated project directory query parameters
- #2127 fix(web): keep the passkey store empty-file recheck bounded
- #2132 pr-shepherd: close PR #2091 as superseded — its followFile StringDecoder fix landed via PR #2100 (#2059 closed)
- #2133 pr-shepherd: close PR #2093 as superseded — its pair-safe browser history title clamp landed via PR #2099 (#2050 closed)
- #2141 fix(web): ignore multiline inline-code image references in grants
- #2142 fix(web): ignore HTML-comment image references in grants
- #2144 fix(ui): scope coalesced reads by Authorization header
- #2145 fix(ui): invalidate pending agent-memory loads on runtime switch
- #2146 pr_shepherd: PR #2117 is merge-conflicted (binary run-history.js) and its #2115 NUL fix is still needed — re-land it on current main
- #2150 fix(web): honor Markdown closing-fence rules in image grants
- #2151 fix(web): ignore images inside indented code blocks in grants
- #2152 fix(web): prevent split surrogate pairs in content-search previews

## Amendment (2026-11-12, later pass) — zero-carrier program epics created

Drew approved "do both"; the zero-carrier labels from the register above now
have epic issues in developing-today/code, members natively linked and
verified from both the epics' sub-issue lists (per_page=100) and sampled
children's parent pointers:

| Epic                                 | Label                         | Members | Natively linked                  |
| ------------------------------------ | ----------------------------- | ------- | -------------------------------- |
| #8983 Atlas program                  | `epic/atlas-program`          | 68      | 68                               |
| #8985 GitHub and dynamic compute     | `epic/github-dynamic-compute` | 28      | 24                               |
| #8986 design register                | `epic/design-register`        | 27      | 26                               |
| #8987 Turso + Cloudflare integration | `epic/turso-cloudflare`       | 16      | 11                               |
| #8988 storage program                | `epic/storage`                | 11      | 10                               |
| #8989 SaaS service                   | `epic/saas-service`           | 6       | 3                                |
| #8990 fleet tool                     | `epic/fleet-tool`             | 6       | 6                                |
| #8991 pool routing                   | `epic/pool-routing`           | 5       | 1 (+4 nested under member #1334) |
| #8992 fleet apps                     | `epic/fleet-apps`             | 2       | 2                                |
| #8993 mcpx                           | `epic/mcpx`                   | 2       | 0 (see below)                    |

- `epic/atlas` (8) is a strict subset of `epic/atlas-program` (68) — folded,
  no separate epic; both carrier labels sit on #8983 (label redundancy noted
  on the epic).
- Unlinked members are one-parent-rule exceptions, recorded on each epic:
  already parented elsewhere (#5942/#5992/#6188 under #6185, #7091 under
  #7089, #1111/#1115/#1116 under #1047, #1335/#1338/#1340/#1342 under fellow
  member #1334, #8833 under #8988 by cross-label assignment), or dual-carrier
  register tracking homes parented under #8986 (#8832/#8821/#8820/#8808/#8807
  of turso-cloudflare, #8811 of storage, #8822/#8809 of mcpx — hence #8993's
  0 direct sub-issues).
- All members of every cluster live in developing-today/code (other repos
  checked: zero carriers). The ambiguous multi-carrier labels remain
  untouched, awaiting Drew's pick. The Project-board blocker above is
  unchanged by this pass.
