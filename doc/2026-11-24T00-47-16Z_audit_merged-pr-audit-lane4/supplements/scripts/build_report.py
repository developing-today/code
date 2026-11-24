import json, os
BASE="/home/hatch/workspace/lane4-audit7"
NAME="2026-11-24T00-47-16Z_audit_merged-pr-audit-lane4"
ledger=json.load(open(f"{BASE}/ledger-fleet.json"))
brief=open(f"{BASE}/brief-verbatim.md").read()
rows="\n".join(f"| [#{r['number']}]({r['url']}) | {r['title'].replace('|','/')} | {r['spawning_issue']} | {r['verdict']} | {r['label']} | {r['note']} |" for r in ledger)
report=f"""# AUDIT-7 Lane 4 — MERGED-PR-AUDIT: merged PRs since 2026-10-23 vs their spawning issues

- **Programme:** AUDIT-7 (Drew's directive, 2026-11-23 ~17:56 CST)
- **Lane:** 4 — MERGED-PR-AUDIT
- **Umbrella:** [developing-today/code#13871](https://github.com/developing-today/code/issues/13871)
- **Lane tracking issue:** [developing-today/code#13879](https://github.com/developing-today/code/issues/13879)
- **Record created:** 2026-11-24T00:47:16Z
- **Status at this record:** Slice 1 (dezren39/fleet) audited in full by this lane; the other three repos are packetised into the pilot queue as verified `feature_slice` tasks whose ledgers will append to #13879 and to this record's programme.

## The brief (verbatim)

{brief}

## Outcome labels (Lane 4 set)

Created in developing-today/code and in dezren39/fleet (same names/colours/meanings in both; slice workers create them in the forks repos before applying):

| Label | Meaning |
|---|---|
| `audit:complete` | PR fully implemented its spawning issue / stated intent; no gap found |
| `audit:partial` | PR partially implemented its spawning issue; the remainder is filed or already tracked (link in the ledger) |
| `audit:gap-filed` | A gap or unfiled planned next step found in this PR was filed as a new issue |
| `audit:bug-filed` | A bug found in the merged diff was filed as a new issue |
| `audit:completed-later` | Something this PR left undone (or got wrong) was completed/fixed by a later merged PR (linked in the ledger) |
| `audit:no-issue` | PR had no spawning issue to compare against; judged on its stated intent only |

## Method (repeatable procedure — also committed as a skill)

The full procedure is committed in this same PR as `.agents/skills/merged-pr-audit/SKILL.md`. In short:

1. Census: `fleet gh api repos/<repo>/pulls?state=closed&per_page=100&sort=updated&direction=desc`, keep `merged_at >= 2026-10-23T00:00:00Z`; snapshot the list before judging (the list grows while you work).
2. Per PR: fetch the PR (`/pulls/<n>`) and its files (`/pulls/<n>/files?per_page=100`) via `fleet gh api`; read body + diff in full. Extract the spawning issue, especially `Closes/closes/fixes` references; fetch that issue and ALL its comments (`per_page=100`, count-confirmed).
3. Judge PR vs issue. Before filing anything, search later merged PRs for a completion; if one completed it, verdict `completed-later` and mark both items (label + ledger note; comment where the record is unclear).
4. Real gaps/bugs/next-steps: file in the owning repo via `~/tools/gh-post`, prioritise via the pilot queue, link from the ledger and the tracking issue. Never use closing keywords in comments on issues that must stay open. Suspect closures go to Lane 2 as pointers on the umbrella — Lane 2 owns reopen/correction.
5. Apply the outcome label to the PR. Post progress on the lane tracking issue at every slice boundary.
6. Land EVERYTHING — report, censuses, ledgers, per-item JSON, scripts, logs — as a dated record in doc/ with supplements, per Drew's 2026-11-23 work-product-landing rule.

Tooling notes from this run: the `fleet gh` cache daemon (127.0.0.1:4802) may be down; start `fleet gh cache-serve` or calls resolve in-process and take ~2 min. The sandbox→queue-broker route may refuse CONNECT; enqueue from the laptop instead (`fleet queue enqueue <packet-file> --queue repo-health-code-pilot` on `amd`), where the packet validator prints the exact schema it wants (notably: `feature_slice` packets need `payload.priority_reason`, `payload.evidence[]`, `payload.tests[]`, `payload.dedupe{{checked_at,basis}}`, `payload.slice{{parent_issue,number,criterion,done_check,owned_paths}}`, and dedupe key `repo-health:v1:slice:<repo>:issue:<n>:slice-<k>`).

## Census (snapshot 2026-11-24T00:39Z, merged_at >= 2026-10-23)

| Repo | Merged PRs in window |
|---|---|
| developing-today/code | >= 3,335 (updated-desc listing still growing during the census; lower bound) |
| developing-today-forks/opencode | 1,122 |
| developing-today-forks/openchamber | 1,026 |
| dezren39/fleet | 92 |
| **Total** | **>= 5,575** |

Full census lists are supplements: `supplements/census-*.json`.

## Slice 1 — dezren39/fleet: COMPLETE (92/92, audited by this lane)

Verdicts: **81 complete · 8 completed-later · 3 partial · 0 gaps filed · 0 bugs filed** — every remainder found was already tracked in an existing open issue or programme record, so per the dedupe rule no new issues were filed; the marks are the labels + this ledger + the comments named below.

Findings worth naming:

- **A merged bug, fixed forward:** PR #82 added `workflows: write` to a job `permissions:` block — not a valid GitHub token permission, so the workflow file failed to parse and both forks' sync dispatches 422'd. PR #83 removed it the same day and documented why. #82 = `completed-later`, #83 = complete.
- **A fix chain that took four shapes:** PRs #50 → #51 → #52 iterated the `timeout_minutes` clamp (GitHub expressions have no arithmetic operators; the `* 1` coercion in #50 was itself invalid). #50/#51 = `completed-later`, #52 = complete.
- **Partial — verification still failing:** PR #100's mode=min cache-export fix for standing issue fleet#88 merged 2026-11-23T04:24Z and is verified on main, but the build leg failed again at 11:25Z and 23:08Z the same day (runs 45161352548, 45283270086). Lane 4's comparison record is posted on fleet#88; the standing issue owns the follow-up. #100 = `partial`.
- **Partial — remainder already tracked:** PR #101 scopes `sync-upstream` to the two App-mirror forks; its spawning issue fleet#87 stays open for the app-install track. PR #94's Drew-reserved proof step (PAT workflow scope) was completed later operationally (scope granted; code#12490 closed). Both = `partial` with the remainder named, no duplicates filed.
- **Suspect closure handed to Lane 2:** fleet#32's body asks for a visibility fix (warning annotations + per-leg outcome summary so optional-leg failures cannot hide inside green runs); it was closed on the strength of the R2 DNS-rename fix (PRs #44/#33). PR #44 is complete for the rename it stated; whether the closure stands is Lane 2's call. Pointer posted on the umbrella (#13871, comment 7118183345).
- **Completed-later chains:** #5's noted fallback architecture → built by #6; #11's serialisation stopgap → superseded by #12/#13 (ca-derivations was the real fix); #21's restore-path fix → superseded by #22's artifact handoff; #55's held fifth fleet member → added by #56; the fork-migration plan #59 (plan only, by design) → executed in #63/#64/#67/#76/#78; #105's bun support → bunx remainder completed by #110 via code#13189.
- **Closing-keyword PRs verified against their issues, all sound:** #108→fleet#107, #102→fleet#89 (acceptance-verified close), #61→fleet#28, #75/#77→fleet#70, #31→fleet#30, #90→code#2230 (done-check verified), #109→code#13537, #110→code#13189.

### Per-PR ledger — dezren39/fleet

| PR | Title | Spawning issue | Verdict | Label | Note |
|---|---|---|---|---|---|
{rows}

## Slices 2-4 — enqueued (pilot queue `repo-health-code-pilot`)

24 `feature_slice` packets, each carrying its full PR-number list, this lane's method, the label set, and a ledger-on-#13879 acceptance. Enqueued from the laptop (sandbox→broker CONNECT was refused) and verified in the queue 2026-11-24:

| Repo | Batches | PRs (snapshot) | Queue tasks | State at verification |
|---|---|---|---|---|
| developing-today/code | 14 | 3,335 | 21582–21599 | 11 claimed, 3 parked |
| developing-today-forks/opencode | 5 | 1,122 | 21606–21610 | queued |
| developing-today-forks/openchamber | 5 | 1,026 | 21601–21605 | queued |

Dedupe keys: `repo-health:v1:slice:<repo>:issue:13879:slice-<batch>`. Packet files are supplements: `supplements/packets/`. Queue verification output: `supplements/queue-verification.txt`.

## Supplements index (all raw material, per the work-product-landing rule)

- `supplements/brief-verbatim.md` — this lane's brief + the 2026-11-23 landing steer, verbatim.
- `supplements/census-fleet.json`, `supplements/census-developing-today_code.json`, `supplements/census-developing-today-forks_opencode.json`, `supplements/census-developing-today-forks_openchamber.json` — census snapshots.
- `supplements/full-dezren39_fleet.json` — every fleet PR's body + file list + patches as fetched (the raw material Slice 1 was judged from).
- `supplements/fleet-issues-full.json` — the verification issues read in full (fleet#107/#89/#88/#32/#30/#70/#28/#46/#87; code#13774/#13189/#13537/#2230/#6645) with all comments.
- `supplements/ledger-fleet.json` + `supplements/verdicts_fleet.py` — the Slice 1 ledger and the verdict source it was built from.
- `supplements/packets/` — the 24 queue packets as enqueued (+ manifest).
- `supplements/queue-verification.txt`, `supplements/enqueue-results.json`, `supplements/enqueue-laptop.log` — enqueue evidence.
- `supplements/progress-census.md` (see tracking issue), `supplements/progress-fleet-slice.md`, `supplements/progress-enqueued.md`, `supplements/pointer-32.md`, `supplements/comment-88.md` — the exact texts posted to #13879, the umbrella, and fleet#88. (progress-census text lives in the first progress comment on #13879; its file was lost in a sandbox restart and is recorded here by reference.)
- `supplements/scripts/` — every script used: census, per-PR fetch, issue fetch, ledger build, label application, packet build, enqueue.
- `supplements/logs/` — run logs (label application, censuses, fetches, enqueues).

One loss, recorded honestly: the first copy of the fleet per-PR dataset and the first census files were held in /tmp and destroyed by a sandbox service restart mid-lane (2026-11-24 ~00:28Z). The fleet dataset was re-gathered in full (that re-gather is the supplement above); the verdicts were reconstructed verbatim from the lane's working record and re-verified against the re-gathered census (92/92 numbers match).

## Counts (this lane, at this record)

- Examined in full by this lane: **92** (dezren39/fleet, complete)
- Complete: **81** · Completed-by-later-PR: **8** · Partial (remainder already tracked): **3**
- Gaps filed: **0** · Bugs filed: **0** (the one merged bug found, fleet#82's invalid permission, was already fixed by #83; the one suspect closure was handed to Lane 2 as a pointer, per the coordination rule)
- Enqueued for exhaustive completion: **5,483** PRs across 24 verified queue tasks (code >= 3,335 + opencode 1,122 + openchamber 1,026)
- Labels applied: **92/92** fleet PRs; label set created in code + fleet
"""
os.makedirs(f"{BASE}/record",exist_ok=True)
open(f"{BASE}/record/{NAME}.md","w").write(report)
print("report bytes", len(report))
