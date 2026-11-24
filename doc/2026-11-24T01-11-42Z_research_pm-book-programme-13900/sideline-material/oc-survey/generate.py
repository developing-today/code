import json, os, re, collections, datetime
O=os.path.expanduser("~/.oc-survey")
OUT=os.path.expanduser("~/workspace/pm-book/survey/oc-programme.md")
NOW=datetime.datetime(2026,11,24,0,55,tzinfo=datetime.timezone.utc)
def L(n):
    p=f"{O}/{n}"
    if os.path.exists(p) and os.path.getsize(p)>0:
        try: return json.load(open(p))
        except Exception: return []
    return []
def age(d):
    try:
        c=datetime.datetime.fromisoformat(d.replace("Z","+00:00"))
        return (NOW-c).days
    except Exception: return "?"
def esc(t):
    return t.replace("|","\\|").replace("\n"," ").strip()
SCOPE_RE=re.compile(r'^(?:feat|fix|chore|refactor|test|docs|ci|perf|build|style|revert)(?:\(([^)]+)\))?[:!]')
def theme(title):
    m=SCOPE_RE.match(title.strip())
    if m and m.group(1): return m.group(1)
    t=title.lower()
    for kw,th in [("route","routes/pools"),("quota","quota/plans"),("provider","providers"),("upstream","upstream sync"),("nix","nix/packaging"),("session","sessions"),("tui","tui"),("console","console"),("plugin","plugins/mcp"),("mcpx","plugins/mcp"),("workflow","ci"),("ci","ci"),("test","tests"),("doc","docs")]:
        if kw in t: return th
    return "other/no-scope"
def prio_key(x):
    labels=x.get('labels',[])
    p={'priority/high':0,'priority/normal':1,'priority/low':2}
    pr=3
    for k,v in p.items():
        if k in labels: pr=v
    esc_flag=0 if ('user/escalated' in labels) else 1
    blocked=0 if any(l in labels for l in ('status/blocked','queue/blocked')) else 1
    return (pr, esc_flag, blocked, x['created_at'])
def issue_table(items, repo_url):
    items=sorted([x for x in items if not x.get('is_pr')], key=prio_key)
    lines=[f"| # | Issue | Labels | Age (d) | Created |","|---|---|---|---|---|"]
    for x in items:
        lines.append(f"| [{x['number']}]({repo_url}/issues/{x['number']}) | {esc(x['title'])} | {', '.join(x.get('labels',[])) or '—'} | {age(x['created_at'])} | {x['created_at'][:10]} |")
    return lines, items
def pr_groups(pulls):
    m=[x for x in pulls if x.get('merged_at')]
    g=collections.defaultdict(list)
    for x in m: g[theme(x['title'])].append(x)
    for k in g: g[k].sort(key=lambda x:x['merged_at'])
    return g,m

oc_pc, oc_po = L("pulls-closed-oc.json"), L("pulls-open-oc.json")
och_pc, och_po = L("pulls-closed-och.json"), L("pulls-open-och.json")
oc_io, och_io = L("issues-open-oc.json"), L("issues-open-och.json")
oc_g, oc_m = pr_groups(oc_pc); och_g, och_m = pr_groups(och_pc)
code_sets={}
for n in ["code-title-opencode.json","code-title-openchamber.json","code-title-claimant.json"]:
    code_sets[n]=L(n)
code_open={}
for src in code_sets.values():
    for x in src:
        if x['state']=='open' and not x.get('is_pr'): code_open[x['number']]=x
code_ref=L("code-referenced.json")
ref={x['number']:x for x in code_ref}

W=[]
A=W.append
A("# OpenCode + OpenChamber forks programme — survey (OC-SURVEY)")
A("")
A("Survey date: **2026-11-23 (America/Chicago)**; API data read 2026-11-24T00:35–01:05Z via `fleet gh api`, fully paginated, counts computed from the responses. Laptop file reads via `~/tools/laptop-ssh.sh` against `~/git/code`, `~/git/opencode` and `~/git/openchamber-fork` (no fresh clones). Raw material for this survey is deposited in `~/workspace/pm-book/sideline-material/oc-survey/` (see §7). This section is written to be liftable into a standalone OpenCode/OpenChamber book later.")
A("")
A(f"**Counts at a glance** — opencode fork: **{len(oc_m)} merged PRs** (of {len(oc_pc)} closed, {len(oc_po)} open), **{sum(1 for x in oc_io if not x['is_pr'])} open issues**. openchamber fork: **{len(och_m)} merged PRs** (of {len(och_pc)} closed, {len(och_po)} open), **{sum(1 for x in och_io if not x['is_pr'])} open issues**. code repo programme set (open, title-matched, defined in §3): **{len(code_open)} open issues**. Search totals for context: `repo:developing-today/code opencode` = 4,147 items (all states, issues+PRs, body mentions included) — the programme's code footprint is far larger than the title-matched set; the raw title-search dumps (681 opencode / 137 openchamber / 148 claimant items) are in the deposit.")
A("")
A("---")
A("")
A("## 1. The two forks")
A("")
A("### 1.1 Identity (from the GitHub API repo objects)")
A("")
A("| | opencode fork | openchamber fork |")
A("|---|---|---|")
A("| Repo | [developing-today-forks/opencode](https://github.com/developing-today-forks/opencode) | [developing-today-forks/openchamber](https://github.com/developing-today-forks/openchamber) |")
A("| API `parent` | dezren39/opencode | dezren39/openchamber |")
A("| API `source` (root of fork network) | anomalyco/opencode | openchamber/openchamber |")
A("| Default branch | `main` | `main` |")
A("| Org fork created | 2026-10-09T04:04:23Z | 2026-10-09T04:04:25Z |")
A("| Last push at read time | 2026-11-23T23:54:52Z | 2026-11-23T23:16:56Z |")
A(f"| Branches (API) | {len(L('branches-oc.json'))} | {len(L('branches-och.json'))} |")
A(f"| Labels defined | {len(L('labels-oc.json'))} | {len(L('labels-och.json'))} |")
A("| Description | “The open source coding agent.” | “Agentic Development Environment based on OpenCode AI agent” |")
A("")
A("So the chain is two-step: upstream → Drew's personal fork (dezren39) → the org fork (developing-today-forks). Both org forks were created two seconds apart on 2026-10-09. On the laptop, `~/git/opencode` carries remotes `origin`/`devtoday` = the org fork, `upstream` = dezren39/opencode, `upstream-anomalyco` = anomalyco/opencode; `~/git/openchamber-fork` carries `origin` = the org fork and `upstream` = openchamber/openchamber directly.")
A("")
A("### 1.2 Why we fork (as stated in the repos)")
A("")
A("- The fork READMEs (fork-notice blocks at the top of each README, read on the laptop) state the tracking policy: opencode tracks upstream's **v2 line on `dev` exclusively** (upstream promoted v2 to `dev` on 2026-10-10; legacy v1 is on `v1`); openchamber tracks upstream `main`.")
A("- The programme's own epics state the purpose: the forks exist so the fleet can change the agent and its control surface itself — route pools, quota/plan handling, custom providers, session control — instead of waiting on upstream. See [opencode#2319 “epic: opencode fork — platform work”](https://github.com/developing-today-forks/opencode/issues/2319), [openchamber#2153 “Epic: OpenCode platform”](https://github.com/developing-today-forks/openchamber/issues/2153), and the migration epic [opencode#374 “Migrate the fleet to OpenCode 2”](https://github.com/developing-today-forks/opencode/issues/374).")
A("- A consolidation ruling (code#5060, cited in opencode#2490/#2491) goes further: merge OpenChamber's server-side control into OpenCode and fix the quota credential in the server credential store — i.e. the two forks are intended to converge at the server layer, not merely coexist.")
A("")
A("### 1.3 Claimed divergences (fork README fork-notices)")
A("")
A("**opencode fork claims:** (1) Model route pooling & scheduled weights (`experimental.model_routes`, recurring time-based weight schedules, ordered/round-robin/weighted selection, soft budgets, burst rate-limit continuity, silent-target hedging); (2) indefinite route telemetry & cost accounting (per-request `cost_usd` from catalog pricing); (3) autonomous route review & tuning (`route_stats`/`route_usage` tools, `route_adjust`); (4) plan & quota tracking (`plans.json`, `plan_info` tool, cost tiers cheap/standard/frontier); (5) built-in skills/commands (`/routing-review`, `/plan-update`, `/routing-note`, `/routing-config`); (6) MCPX integration & daemon handover (native MCPX tool projection into Code Mode, detached daemon, JIT secret prompts, socket handover on `serve` restarts).")
A("")
A("**openchamber fork claims:** (1) Model Pools UI & recurring schedules (Settings UI for `experimental.model_routes`); (2) Route Rules interpreter & timed overrides (“Route by sentence”); (3) Plans & Usage section (Settings → Routing → Plans and usage); (4) Review & Update action buttons for `/routing-review` and `/plan-update`; (5) control & workspace enhancements (worktree-by-default sessions, `projects.add`/`projects.remove`, fork-specific release update resolution); (6) MCPX secret management UI (JIT secrets on desktop/web/VS Code/mobile); (7) inter-session messaging (Messages surface) — explicitly “Needs an OpenCode build with messaging (developing-today-forks/opencode)” (a cross-fork dependency, §5).")
A("")
A("### 1.4 Standing divergences evidenced independently of the READMEs")
A("")
A(f"- **Merged-PR volume is the divergence.** {len(oc_m)} merged PRs on the opencode fork and {len(och_m)} on the openchamber fork, almost all authored by `dezren39` (opencode: {collections.Counter(x['user'] for x in oc_m).most_common(3)}; openchamber: {collections.Counter(x['user'] for x in och_m).most_common(4)}), the remainder by `developing-today[bot]`, `github-actions[bot]` and `dependabot[bot]`. Full enumeration in §1.5.")
A("- **Long-lived / standing branches.** opencode fork (61 branches): `spike/2492-tursodb-backend` (the TursoDB spike, opencode#2492), `pin/darwin-nix-opencode-6c43550`, `feat/2490-session-cookie-credential`, `feat/2494-route-overrides-api`, plus a large population of `queue/<task>-…`, `lane-*`, `probe-*` and `ci-*` working branches. openchamber fork (32 branches): six `sync/*-20261122` branches from the 2026-11-22 upstream-sync batch, seven `automation/opencode-update-20261117…20261122` daily branches, `216b-goal-spec-jobs`, `investigation/2325-dependabot-regression`. Full branch lists are in the deposit.")
A("- **Fleet config against the fork (code repo, laptop read).** `~/git/code/.opencode/opencode.jsonc` is written against fork-only surface: its header records the 2026-11-07 provider rename (opencode#1823) under which “providers the fleet custom-built in the fork” carry a leading underscore (`_claude`, `_antigravity`, `_vibe`, `_commandcode`, `_muse`), the fork-native antigravity provider (opencode#1457, replacing the :18751 proxy per opencode#1414), the fork-native claude-code provider (opencode#1749 / code#6117), and the provider-availability system (opencode#1828). None of that exists in upstream.")
A("- **Upstream sync is a standing mechanism, not a one-off:** Jules daily upstream-sync workflows, sync-failure tracking issues, and dated sync merges (latest in §6, item 1).")
A("")
A("### 1.5 Merged PRs, grouped by theme")
A("")
A("Grouping rule: theme = the conventional-commit scope in the PR title (`feat(scope):`, `fix(scope):`, …); titles without a scope are grouped by the first matching keyword (route/quota/provider/session/tui/console/plugin/nix/ci/test/doc/upstream), else “other/no-scope”. “Merged” = `merged_at` non-null in the API response. Shipped = merged into `main` (opencode: 1,254 of 1,258 merged PRs target `main`; the 4 exceptions target `sweep/560-provider-models`, `spike/2492-tursodb-backend` and `pin/darwin-nix-opencode-6c43550`). Everything in this section is **shipped**; intentions live in §2/§3, not here.")
A("")
for name,g,m,repo in [("developing-today-forks/opencode",oc_g,oc_m,"https://github.com/developing-today-forks/opencode"),("developing-today-forks/openchamber",och_g,och_m,"https://github.com/developing-today-forks/openchamber")]:
    A(f"#### {name} — {len(m)} merged PRs by theme")
    A("")
    A("| Theme | Merged PRs |")
    A("|---|---|")
    for th,items in sorted(g.items(), key=lambda kv:-len(kv[1])):
        A(f"| {esc(th)} | {len(items)} |")
    A("")
    for th,items in sorted(g.items(), key=lambda kv:-len(kv[1])):
        A(f"##### {name} — theme: {th} ({len(items)})")
        A("")
        A("| PR | Title | Author | Merged |")
        A("|---|---|---|---|")
        for x in items:
            A(f"| [#{x['number']}]({repo}/pull/{x['number']}) | {esc(x['title'])} | {x['user']} | {x['merged_at'][:10]} |")
        A("")
A("---")
A("")
A("## 2. Goals & intentions (code repo + design records)")
A("")
A("What the programme is trying to make true, each with its issue link. Landmark/closed items are marked (closed); open items appear again in the §3 map.")
A("")
goals=[
 ("Attach, don't SSH: research which is more efficient for OpenCode work — SSH-on-laptop vs attaching to the laptop's running server", "https://github.com/developing-today/code/issues/1344", "closed"),
 ("Attach-first OpenCode client helper — forward lifecycle, service discovery, stdin-safe run wrapper (the #1344 follow-through)", "https://github.com/developing-today/code/issues/1357", "closed"),
 ("Evaluate a tailnet-bound `opencode serve` option on the laptop for attach clients (the #1344/#1357 family)", "https://github.com/developing-today/code/issues/1363", "closed"),
]
for t,u,s in goals: A(f"- [{t}]({u}) ({s}).")
A("- **A claimant owns dispatch.** The OpenCode claimant/executor (code#3026, slice 1) should discover unbound opencode-lane queue tasks, create/verify/bind sessions on the fork server, deliver prompts, and recover stuck sessions — under the session contract of code#4912 (phases #4757/#4870/#4758, closing audit #4916) and the seam contract of code#7770 (phases #7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732; audit #7772). Live state of those numbers is in the deposit (`code-referenced.json`).")
A("- **OpenCode 2 migration of the whole fleet** — [opencode#374](https://github.com/developing-today-forks/opencode/issues/374) (priority/high), its core-migration tracking home [opencode#2295](https://github.com/developing-today-forks/opencode/issues/2295), plugin follow-through [opencode#2290](https://github.com/developing-today-forks/opencode/issues/2290), the embedded-OpenCode upgrade on the OpenChamber side [openchamber#384](https://github.com/developing-today-forks/openchamber/issues/384), and the deployed-plugin gap list at `doc/2026-11-23T03-45-00Z_opencode-v2-deployed-plugin-gap-list/` in the code repo (three v1 constructs with no v2 equivalent, each with a chosen workaround).")
A("- **Routes, pools and quota as fork-native machinery** — model-route follow-ups [opencode#2681](https://github.com/developing-today-forks/opencode/issues/2681), the config-projection gap [opencode#2782](https://github.com/developing-today-forks/opencode/issues/2782) (“needed by code#13675 Part 2”), OpenChamber's pool-settings follow-ups [openchamber#2498](https://github.com/developing-today-forks/openchamber/issues/2498), retry-recovery controls [openchamber#2126](https://github.com/developing-today-forks/openchamber/issues/2126), and the provider doctrine [opencode#1717](https://github.com/developing-today-forks/opencode/issues/1717) (“*-direct providers EMULATE the first-party wire”).")
A("- **Server façades and consolidation** — generic OpenAI façades on the fork server [opencode#1146](https://github.com/developing-today-forks/opencode/issues/1146) (priority/high; “proxy doors retire”), OpenChamber server-control consolidation + quota credential [opencode#2491](https://github.com/developing-today-forks/opencode/issues/2491) / [opencode#2490](https://github.com/developing-today-forks/opencode/issues/2490) (both citing the code#5060 ruling).")
A("- **Console work** — the PM console (code repo `pm-stack/console`, §4 context) consumes the forks' server/session surface; on the fork itself the console scope is one of the largest PR themes in §1.5, and OpenChamber is the operator console for sessions (its epic: [openchamber#2153](https://github.com/developing-today-forks/openchamber/issues/2153); session dispatch + babysitting epic in the code title set, `code-title-openchamber.json` in the deposit).")
A("- **Observability without instrumentation** — usage export by pull over the fork's own DB (code#2072 slice 1, §4.3); slices 2–7 are stated follow-ups, i.e. intentions, not shipped work.")
A("")
A("---")
A("")
A("## 3. Prioritised issue maps")
A("")
A("**Ordering rule (stated, as required).** Both fork repos *do* carry explicit priority labels (`priority/high`, `priority/normal`, `priority/low`) on most open issues, and the code repo uses the same family on part of its set. Order is therefore: (1) `priority/high` → `priority/normal` → `priority/low` → no priority label; (2) within a tier, `user/escalated` first; (3) then `status/blocked`/`queue/blocked`; (4) then oldest first. Milestones: **none** of the open issues on either fork, and none in the code set below, carries a milestone (the API `milestone` field is null throughout) — milestones play no part in the ordering. Cross-references from code issues are noted where the title/body itself states them (e.g. opencode#2782). Age = whole days from creation to 2026-11-24.")
A("")
A(f"### 3.1 developing-today-forks/opencode — every open issue ({sum(1 for x in oc_io if not x['is_pr'])})")
A("")
lines,_=issue_table(oc_io,"https://github.com/developing-today-forks/opencode"); W.extend(lines); A("")
A(f"### 3.2 developing-today-forks/openchamber — every open issue ({sum(1 for x in och_io if not x['is_pr'])})")
A("")
lines,_=issue_table(och_io,"https://github.com/developing-today-forks/openchamber"); W.extend(lines); A("")
A(f"### 3.3 developing-today/code — programme issues, open ({len(code_open)})")
A("")
A("Scope rule for this table: open code issues whose **title** names opencode, openchamber or the (opencode) claimant — the union of the three title searches in the deposit. This is deliberately narrower than “mentions opencode anywhere” (480 open code issues do that); those 480, and all closed title-matched items, are in the deposit for the book's full-record landing.")
A("")
lines,_=issue_table(list(code_open.values()),"https://github.com/developing-today/code"); W.extend(lines); A("")
A("---")
A("")
A("## 4. PM-stack integration surface (code repo, `pm-stack/`, read on the laptop)")
A("")
A("### 4.1 `pm-stack/opencode-claimant` — the dispatcher")
A("")
A("A Python claimant/executor (main script `claimant.py`, ~2,250 lines, plus `poll.py`, `launch_one.py`, `dispatch_batch.py`, `send_to_opencode.py`, `babysitter.py`, `nudges.py`; README 546 lines; specified in code#3026). Each fire: (1) supervises bound sessions and recovers zero-message/gone sessions (code#4758 rules: verify via the fork server's per-id endpoints, DELETE the verified-empty session, atomically reclaim via `repo-queue bind`); (2) discovers unbound `lane: \"opencode\"` tasks across the queue broker; (3) enforces a 36-root-session cap counted from the fork server's `GET /api/session/active` (claimant roots only, ruling session-cap-5016/code#5016); (4) creates sessions with the v2 create shape (`POST /api/session` with `location.directory`, verified against the repo primary before bind — a top-level `directory` is stripped by v2, code#4870), binds via `repo-queue bind`, and delivers the prompt with verified delivery; (5) logs the fire to `~/.local/state/opencode-claimant/fires.jsonl`. Mode gate: `shadow` (default, mutates nothing) vs `live`, from a local mode file. On cap saturation it hands one candidate back per fire, parked with reason `executor-saturated` and a capacity ready-condition (code#1187). Every fire also runs a babysitter round (code#1338): a verdict (progressing/stalled/done-unreported/blocked/walled/idle) for every root session, stall = ≥20 min silent with no live CI on the bound PR, confirmed on a second round. It drives the **fork** server specifically: the README's session contract (code#4912) and seam contract (code#7770) are written against the fork's v2 API behaviour (singular `/message`, `/session/active`, location-verified create) that upstream v1 did not have.")
A("")
A("### 4.2 `pm-stack/opencode-scheduled-jobs` — fire-and-forget fork jobs")
A("")
A("Source for small scheduled OpenCode jobs on the laptop: each job is a systemd user service/timer pair with its route visible in the unit, a read-only prompt, and a stable per-job state dir. The registry (`jobs.json`) currently holds exactly **one** job: `quota-freshness-summary` — hourly, route `opencode-route/workhorse` (a fork route pool, §1.3), reading a sanitised projection of `~/.local/state/pm-aggregator/quota-current.json` (only provider names, status, timestamps, window names/states reach the model; credentials, raw usage, limits and balances are stripped by `summarize.py` before the fork session sees them). Enabled 2026-11-21 after an explicit pilot decision; first hourly firing observed exit 0. Logs at `~/.local/state/opencode-scheduled/quota-freshness-summary/{latest.log,history.jsonl}`. The README is explicit that the timer provides **no** stall supervision — that remains the claimant babysitter's job (§4.1), and that a monitor must correlate via directory-scoped server API reads because `opencode session list` “is not a reliable discovery surface” (against the fork server).")
A("")
A("### 4.3 `pm-stack/opencode-usage-export` — the usage producer")
A("")
A("A read-only **pull** exporter (Node, `node:sqlite`, store opened read-only with `query_only`; 15-minute systemd timer on the laptop) over the fork's own history database — code#2072 slice 1, “the same pull pattern as the Muse runtime-db export. No push instrumentation is added to OpenCode/OpenChamber.” It probes candidate stores in order (`~/.local/share/opencode/opencode.db`, `~/.local/share/openchamber/opencode.db`, `~/.local/state/opencode/opencode.db`), picks the first that is *live* (has the v2 tables `session_v2`/`session_message` and a newest `time_updated` within 24 h), surfaces a store move in the tick document, and otherwise fails loudly (`store-missing`/`store-stale`/`schema-drift`/`read-failed`, exits 3–6, failure documents carry no sessions). Output: `~/atlas/opencode-usage-export/{latest.json,history.jsonl,state.json}` — per-session cost/token totals with parentage, feeding the PM aggregator's usage views. The README warns that the legacy `session`/`message`/`part` tables in the same file froze on 2026-10-06; the exporter reads only the v2 tables the fork writes.")
A("")
A("---")
A("")
A("## 5. Dependency notes (only edges seen in titles/bodies/READMEs actually read)")
A("")
deps=[
 "code#1357 and code#1363 both follow the code#1344 research (attach-first family, titles/bodies of the three issues).",
 "opencode#2782 (config projection strips `model_routes` purpose / `model_designations`) states in its title that it is “needed by code#13675 Part 2” — the fork fix blocks that code work.",
 "opencode#2681 is explicitly “follow-ups after #2674, #2676 and #2679” (same-repo opencode numbers).",
 "opencode#2505 (no CI coverage for the TursoDB backend) is about PR opencode#2499's zero tests; the TursoDB evaluation itself is the open spike opencode#2492, which also owns the standing branch `spike/2492-tursodb-backend`.",
 "opencode#2490 and opencode#2491 both cite the code#5060 ruling; #2491 (consolidation of OpenChamber's server control into OpenCode) is the parent intention, #2490 (quota credential in the server credential store) is named “(code#5060 fix)”.",
 "openchamber#384 (upgrade the embedded OpenCode to a v2 build) depends on the opencode fork's v2 line — the same migration as opencode#374/#2295/#2290; the openchamber README likewise states its messaging surface “needs an OpenCode build with messaging (developing-today-forks/opencode)”.",
 "Claimant chain (claimant README): babysitter rounds (code#1338) produce the confirmed stalls that the nudges slice (code#1187's next slice) consumes; the claimant's session contract is code#4912 phase 4 (audit code#4916) over phases code#4757/#4870/#4758, and its seam contract is code#7770 over code#7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732.",
 "opencode-usage-export is code#2072 slice 1; the README states slices 2–7 (webhooks, views, rollups, projections) are separate follow-ups — they depend on this exporter existing, not vice versa.",
 "opencode-scheduled-jobs depends on the claimant babysitter for stall supervision (its README: “that remains the babysitter's responsibility”) and on the aggregator's quota snapshot file as its only input.",
 "openchamber#1573 is a split of openchamber PR #216 (“Split 216b of PR #216”); openchamber#937 is the stale-PR resolution for the same PR #216 — the two open issues share that parent PR.",
 "No other blocking edges are asserted: no `blocked-by` label or explicit dependency field exists on the fork repos beyond the `status/blocked`/`queue/blocked` labels shown in §3.",
]
for i,d in enumerate(deps,1): A(f"{i}. {d}")
A("")
A("---")
A("")
A("## 6. Disagreements — where the docs/READMEs and the repos' actual state differ")
A("")
dis=[
 "**Upstream-sync date (both forks).** Both fork READMEs state the latest upstream commit merged is from **Fri Oct 16 2026** (opencode: `00e7472f…`; openchamber: `2b94db84…` via `8f9956331`). The repos show later upstream syncs merged: opencode PR #2683 “sync anomalyco/opencode dev (2026-11-22)” and #2690 “sync anomalyco/opencode dev (2026-11-23)”; openchamber PR #2502 “Merge upstream openchamber main (v2.8.0)” (2026-11-22) and #2510 “Merge upstream openchamber main (3a0dbeafb4)…” (2026-11-23). opencode#2677 “Follow-ups from the 2026-11-22 upstream sync” is itself open. The README notices are ~5 weeks stale.",
 "**Fork parentage (both forks).** Both READMEs describe the repo as a fork of the upstream project (anomalyco/opencode; openchamber/openchamber). The API says each org fork's direct `parent` is Drew's personal fork (dezren39/opencode; dezren39/openchamber); upstream is the `source`, one step further up.",
 "**openchamber#2564 vs its merged fix.** Issue #2564 (preview-grant symlink escape) is still **open**, labelled `status/in-progress` / `queue/claimed`, yet its fix PR #2579 (“preview-grant serve route re-checks canonical path vs grant root (openchamber#2564)”) and the underlying commit both merged on 2026-11-23 (also visible at the head of the laptop's openchamber clone). The issue record does not reflect the landed fix.",
 "**“Done” labels on open issues.** opencode#2505 carries `status/done` and openchamber#2081 carries `status/done` + `queue/done`, yet both issues are open. Either the labels or the issue states are wrong.",
 "**“Shipped” titles on open migration issues.** opencode#2295 is titled “…core migration (**shipped** — tracking home)” and openchamber#2126 “…(**shi**pped…)”, while the parent migration opencode#374 “Migrate the fleet to OpenCode 2” (priority/high), opencode#2290 (port the v1 plugins) and openchamber#384 (embedded v2 upgrade) all remain open — the migration is simultaneously recorded as shipped and as outstanding, on different surfaces.",
 "**openchamber Actions: disabled vs running.** openchamber#2274 (2026-11-13) states “GitHub Actions is disabled for this repo again — no workflow has run since 2026-11-11”. The repo's own later record contradicts the standing state: openchamber#2586 (2026-11-23) reports a stale-workflow Actions run (run 45147203731), and PRs continued merging through 2026-11-23 — Actions did run after 2026-11-11, whatever was true on the 13th.",
 "**Claimant README, internally and vs current operation.** (a) The README's queue section says the claimant “monitors four queues” (the four `repo-health-*-pilot` queues), while its own `launch_one.py` section describes the current one-queue form (“For the one-queue `code` stream, use `code:<task_id>`…”). (b) Its Zero-Message Recovery step 1 writes the endpoint as `GET /api/session/{id}/messages` (plural) in the same sentence that warns the plural endpoint 404s and the singular is correct; the Session Contract table later gives the singular `/message` form.",
 "**Fork default branch vs tracking claim.** The opencode README says the fork tracks upstream `dev` exclusively, but the fork's own default branch in the API is `main` (all 1,254 home-grown merged PRs target `main`); `dev` is upstream's branch name, not the fork's — a reader following the README literally would look for the fork's work on a branch the fork does not default to.",
 "**opencode CI state, two ways at once.** opencode#2785 (2026-11-23) reports “CI run silence — no completed Actions run within threshold”, while opencode#2638 and #2518 are open CI-*failure* issues about specific completed runs (44247391571, 43849921062). Both can be true for different workflows, but as recorded they describe the same repo's CI as simultaneously silent and failing, with no surface reconciling the two.",
]
for i,d in enumerate(dis,1): A(f"{i}. {d}")
A("")
A("---")
A("")
A("## 7. Raw material deposit (per the 2026-11-23 standing rule)")
A("")
A("Deposited in `~/workspace/pm-book/sideline-material/oc-survey/`: the full API dumps behind every count in this survey — merged/closed/open PR enumerations and open/closed issue lists for both forks, branch and label lists, the code-repo title-search sets (opencode/openchamber/claimant) and open-mention set, the individually verified code referenced-issue set, the fetch scripts, and laptop README excerpts (fork notices + claimant README tail). File list is in the deposit's `README.md`.")
A("")
with open(OUT,"w") as f: f.write("\n".join(W)+"\n")
print("wrote",OUT,"lines",len(W)+sum(1 for _ in open(OUT)) if False else "")
print("oc merged",len(oc_m),"och merged",len(och_m),"code open set",len(code_open))
