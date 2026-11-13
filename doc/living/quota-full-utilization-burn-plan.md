# Quota full-utilization program + burn plan

> Living document — committed 2026-11-11 from ~/workspace/research_notes/quota-burn-plan-20261029/plan.md as the tracking home for a completed design (60-day design register entry F4.5; register committed as pm-stack/design-register-2026-11-11.md in developing-today/code, tracking epic developing-today/code#8805). Status: shipped. Update this doc as the design shifts; the register is the frozen audit snapshot.

---

# Full-quota utilization plan — first edition (code#2173)

Built: 2026-10-29 ~07:35Z (02:35 CDT). Read-only analysis — no jobs, weights, or queues changed.
Sources, each figure timestamped where used: laptop PM quota-gatherer series
(`~/.local/state/pm-aggregator/snapshots/usage-2026-10.jsonl`, 599 rows, 2026-10-25T12:00Z →
2026-10-29T07:15Z) and `quota-current.json` (generated 2026-10-29T07:30:13Z, source
openchamber-quota); Supabase `agent.quota_records` (assistant subscription, 1-min cadence);
Supabase `agent.mirror_token_usage_daily` (fleet token totals); provider-quota-registry.md
(pool map, reset calendar, route-pool structure); dated console reads recorded in the
registry / stewardship log where the feeds have no series (named as such).

Fleet context (mirror_token_usage_daily, dimension=agent, input tokens/day): Oct 20 0.40B →
Oct 24 1.42B → Oct 26 1.90B → Oct 27 2.84B → Oct 28 5.07B → Oct 29 partial (to ~07:30Z) 2.12B.
Demand is not the constraint — the fleet burns more every day. The constraint is _which pool_
each token lands on, and several pools will end their periods far from 100% in opposite
directions at once.

Conventions: "pts" = percentage points of the pool's largest window. Rates are measured
Δpts/day over the stated window. Projections assume the measured rate holds; where the rate
is unstable I give the scenario range instead of a fake single number.

---

## 1. Per-pool state, rates, projections (largest window each)

### OpenCode Go — Lab workspace (the active one) — MONTHLY per-model caps

- **Now: 87.17%** (gatherer series, 2026-10-29T05:30Z; the quota module's combined Go row read
  lower, 77.28% at 01:30Z — the two computations disagree; the continuous series is used as
  primary). Weekly sub-window 49.37% (resets Nov 2 00:00Z), 5h sub-window ~49%.
- **Reset: 2026-11-07T05:55:15Z** (Nov 6 23:55 CT). Plan is CANCELLED — this is the last period.
- **Burn rate:** +12.5 pts/day (72h), +18.6 pts/day (24h).
- **Projection:** 12.8 pts left ÷ ≥12.5/day → **monthly caps exhaust ~Oct 30–31, about 7 days
  before reset.** Projected % at reset: 100% (long before). This is the fleet's main
  DeepSeek-Flash engine right now, and it is nearly spent.
- Note: Go's windows are nested limits on the same per-model caps (5h = 20% of cap, weekly =
  50%, monthly = cap). The Nov 2 weekly reset does **not** reopen a monthly-exhausted model.

### OpenCode Go — Personal workspace — MONTHLY

- **Now: ~84%** (console read, 2026-10-28 19:33 CDT — single read, no series; registry series
  last showed 80% @ Oct 27 12:00Z). Plan CANCELLED, **plan ends 2026-11-02 21:02 CT**
  (Nov 3 03:02Z) — after that the quota does not reset, it _disappears_.
- **Burn rate: ~0/day** — Personal is not the active workspace on the laptop (Lab is), so it
  is not being burned at all.
- **Projection: ~16 pts (≈$9.60 of $60 list) stranded forever** unless the active workspace is
  rotated to Personal and the remainder burned before Nov 2 21:02 CT. **#1 under-use flag.**

### OpenAI plan (openai.main) — WEEKLY

- **Now: 27%** (gatherer, 2026-10-29T07:15Z). **Reset: 2026-11-04T06:35:38Z** (Nov 4 00:35 CT).
  73 pts left, 5.96 days → needs **12.2 pts/day** to land at 100%.
- **Burn rate:** +5.9 pts/day (72h average) but +6 pts in the last 5.75h alone (21% @ 01:30Z →
  27% @ 07:15Z) — burn is bursty (sessions on this pool, Aster TASK work, and the OpenAI connector routes all draw
  this pool).
- **Projection (scenarios):** at the 72h average → lands **~62%** at reset (UNDER by ~38 pts);
  at the recent active-hours rate (~25/day) → **exhausts ~Nov 1** (OVER). Verdict: **base case
  UNDER**, with a live over-pace scenario — exactly what the 4–6h check exists to resolve.
  Next reset after Nov 4: Nov 11 06:35Z.

### Meta — Muse API High Usage (via `meta` provider only) — WEEKLY

- **Now: 23.2%** (file-export series, 2026-10-29T04:00Z; the 07:30Z gatherer tick errored on
  this provider — "file-export status: unparseable" — so the freshest read is ~3.5h old).
  **Reset: 2026-11-02T00:00:00Z** (Sun Nov 1 18:00 CT). 76.8 pts left, 3.70 days → needs
  **20.8 pts/day**.
- **Burn rate:** +6.6 pts/day measured post-reset (console 8.83% @ Oct 27 00:03 CDT →
  23.2% @ Oct 29 04:00Z). (The raw 72h series slope is meaningless — the Oct 26 window
  boundary sits inside it.)
- **Projection: lands ~48% at reset — ~52 pts stranded.** **Under-use flag.** Meta seats
  (spark-contributor workhorse/specialist, wonder-contributor summit) are the natural absorber
  for the Nov 2–5 DeepSeek trough (see cascade). Next reset: Nov 9 00:00Z.

### ClinePass — MONTHLY (largest window)

- **Now: 91%** (quota-current, 2026-10-29T07:30Z, corroborated by series 91% @ 00:15Z).
  **Reset: 2026-11-06T01:22Z** (Nov 5 19:22 CT). Weekly sub-window 69%, resets Oct 31 09:20Z.
- **Burn rate (monthly):** +6.1 pts/day (3.6d), +11.4 (72h), +25 (24h) — accelerating; the pool
  is in deliberate finishing-only mode.
- **Projection: monthly exhausts ~Oct 29–30, ~6–7 days before reset. OVER-pace** — deliberate,
  but its cascade consequence is real (§2). Weekly window: at +11.9 to +22.7/day it reaches
  ~100% right around its Oct 31 reset — on-track for the weekly, moot against the monthly cap.

### Command Code — Go $1 plan — MONTHLY

- **Now: 69%** (console read, 2026-10-27 10:05 CDT — **stale, 2 days**; gatherer auth is
  currently failing for this provider, and the console sign-in is a known pending item).
  Period ends **Nov 2** (series reset 2026-11-02T23:04Z). Weekly 14% (series, resets Oct 31
  15:16Z), recent burn ~+0.3–0.5 pts/day.
- **Projection at measured rate: lands ~71% — ~28 pts stranded.** Under-use flag, but small
  ($10 credit pool) and note the plan has **no API access** — burn happens only through the
  resident CLI-proxy route, so the work class is small CLI-shaped jobs. Data too stale for a
  firm projection; flagged in §4.

### OpenRouter — MONTHLY

- **100% — CLOSED until Oct 31 19:00 CDT** (known state, current). Reopens at the monthly
  reset ~Nov 1. Small free-tier pool; its reopening is a minor cascade event (scout seats).

### Open Design (opendesign) — 7-DAY (largest window)

- **Last read 2026-10-28 06:40 CDT: 7-day 97% LEFT (3% used)**, 5h 100% left. No live feed
  (not in quota-current). ~301 usage records/7d. **Under-used but tiny**; data ~1 day stale.

### Antigravity — WEEKLY buckets (per model family)

- **Last reads 2026-10-27T06:00Z: Gemini family 2% used (reset Oct 31 13:11Z), Claude/GPT
  family 50% used (reset Nov 1 09:23Z). The gatherer has emitted no antigravity rows since
  Oct 27.** These buckets matter — sonnet-plan and opus-plan route through antigravity-ls, so
  specialist/summit capacity depends on them. **Too stale to project (§4).**

### Claude (claude-code provider) — NOT CURRENTLY A BURNABLE POOL

- Plan is on the **free tier** since 2026-10-27 and the provider is **disabled in the fleet
  config**. Gatherer reads error ("Rate limited"). The old anthropic:claude-pro series (68% @
  Oct 27 12:00Z, window reset Oct 28 14:00 CT) predates the free-tier state. Drew's "eventually
  Claude again" is a future cascade position, not a current one. **Not projectable.**

### Assistant's own subscription (Maximum) — tracked, NOT a session pool

- Weekly **24%** @ 2026-10-29T07:33Z (quota_records; 20% @ 02:30Z the same night). Reset
  **Nov 3 00:59 CST**. Slope Oct 27 15:31 (8%) → Oct 29 07:33 (24%) ≈ +9.6 pts/day; 76 pts
  left over 5.97 days needs 12.7/day → **projects to land ~81–85%** — modestly under, and the
  heavy-night rate (+19/day) would exhaust it ~Nov 2. On-track-ish; watch only. Included
  because the recorder series exists; per the pool map this is never a routing input.

### Excluded by policy

- DeepSeek direct API and openai.alternate: PAYG/reserve and a stale-read second credential
  (weekly 0% @ Oct 27, resets Nov 3 06:15Z) respectively — not plan pools to schedule against.

---

## 2. The cascade — who absorbs what, in order, across two resets

Route pools fall through cheapest-sufficient-first; a known-100% pool is CLOSED and work
rotates laterally (registry ladder). Projected sequence at current rates:

1. **~Oct 29–30 — ClinePass monthly exhausts.** Its DeepSeek/GLM/Kimi seats stop. Load shifts
   to Go Lab Flash seats (already the main engine) and Meta spark seats. Minor: ClinePass
   weekly resets Oct 31 09:20Z but the monthly cap binds — no real capacity returns until the
   **Nov 5 19:22 CT monthly reset**.
2. **~Oct 30–31 — Go Lab monthly exhausts.** The fleet's DeepSeek capacity drops to opendesign's
   small plan only. **This is the moment for Audible 1** (rotate to Personal, below): Personal's
   ~16 pts become the only Go capacity and must burn before Nov 2 21:02 CT.
3. **Nov 1 ~01:00Z — OpenRouter monthly reopens** (small scout capacity returns).
   **Nov 2 00:00Z — Meta weekly resets** (~77 fresh pts at current projection) and Go weekly
   sub-windows reset (no effect while monthly caps bind).
4. **Nov 2 21:02 CT — Go Personal plan ENDS.** Any unburned Personal quota is gone permanently.
   From here to Nov 7 the only Go workspace is Lab, monthly-exhausted.
5. **Nov 2–5 — THE TROUGH.** DeepSeek-family plan capacity ≈ opendesign (tiny) only. Work must
   ride Meta (fresh Nov 2), OpenAI plan (until its Nov 4 reset), and Antigravity Claude seats
   (unmeasured — §4). Queue classes that only fit DeepSeek seats should be _held_, not forced
   onto PAYG (NEVER FUNDS).
6. **Nov 4 06:35Z — OpenAI plan weekly resets** (fresh ~100 pts; the Nov 4–11 week then needs
   ~14 pts/day sustained — queue Luna/Sol-fit work accordingly).
7. **Nov 5 19:22 CT — ClinePass monthly resets** → DeepSeek seats return via ClinePass.
   **Nov 7 05:55Z — Go Lab monthly resets** → full DeepSeek capacity returns. Second-cycle
   anchors after that: Meta weekly Nov 9 00:00Z, OpenAI plan weekly Nov 11 06:35Z, ClinePass weekly
   Nov 7 09:20Z / monthly Dec 5, Go Lab monthly Dec 7.
   Second-cycle warning: if the Nov 2–5 trough just _defers_ the DeepSeek backlog, Nov 5–7 will
   spike ClinePass/Go burn the same way this week did — the plan for the second cycle is to
   pre-spread that backlog across Nov 5–14 (~10 pts/day each) instead of another 3-day binge.

Over-pace scenario branch: if the OpenAI plan's recent ~25 pts/day persists, it exhausts ~Nov 1, _before_
the trough — the trough then leans entirely on Meta (post-Nov 2) and Antigravity. The 4–6h
check (§5) is what detects which branch we're on.

---

## 3. Standing theoretical plan per pool — target trajectory + dated audibles

Target trajectory = the constant daily rate from now that lands the pool at ~100% exactly at
reset (for windows already nearly spent, the trajectory is "finish cleanly, subsessions only
near the wall" per the registry caveat).

| Pool (largest window)               | Now → reset                     | Needed rate | Measured rate             | Verdict                                |
| ----------------------------------- | ------------------------------- | ----------- | ------------------------- | -------------------------------------- |
| Go Lab monthly                      | 87.2% → Nov 7 05:55Z            | 1.4/day     | +12.5 to +18.6/day        | Exhausts ~Oct 30–31 (over)             |
| Go Personal monthly                 | ~84% → plan ends Nov 2 21:02 CT | 3.3/day     | ~0/day                    | ~16 pts stranded (under)               |
| OpenAI plan weekly                  | 27% → Nov 4 06:35Z              | 12.2/day    | +5.9 (72h) / ~25 (active) | Base ~62% land (under); over scenario  |
| Meta weekly                         | 23.2% → Nov 2 00:00Z            | 20.8/day    | +6.6/day                  | Lands ~48% (under)                     |
| ClinePass monthly                   | 91% → Nov 6 01:22Z              | 1.2/day     | +6.1 to +25/day           | Exhausts ~Oct 29–30 (over, deliberate) |
| Command Code monthly                | 69% → Nov 2                     | ~9/day      | ~+0.4/day                 | Lands ~71% (under, stale data)         |
| Open Design 7-day                   | 3% used → ~Nov 3–4              | ~14/day     | ~0.4/day                  | Lands ~5% (under, tiny)                |
| Assistant subscription (not a pool) | 24% → Nov 3 06:59Z              | 12.7/day    | +9.6/day                  | Lands ~81–85% (watch)                  |

**Dated audibles (planned now, executed by the check in §5 or escalated per its rules):**

- **A1 — Oct 30 (or the first check where Go Lab monthly ≥ 95%): rotate the laptop's active Go
  workspace Lab → Personal** (existing runtime operation: activate the Personal _oauth_
  credential row) and pre-queue Flash-fit work against it, so Personal's remaining ~16 pts burn
  before the plan ends Nov 2 21:02 CT. If Lab monthly exhausts before Oct 30, rotate
  immediately at that check instead.
- **A2 — Oct 31 → Nov 1: Meta burn-up.** Shift workhorse overflow and subsession work onto
  `meta` spark-contributor seats and raise Meta's effective share (route-pool weight/priority)
  for Nov 1–2, targeting Meta weekly ≈ 95–100% at its Nov 2 00:00Z reset instead of the
  projected ~48%. Includes stepping subsessions up effort tiers on Meta in the final 24h
  (burn-by-expiry; plan quota only, never PAYG).
- **A3 — Nov 2 (at the resets): trough posture.** Hold DeepSeek-only queue classes; point the
  workhorse default at Meta (fresh weekly) and schedule the backlog's Luna/Sol-fit design and
  implementation work into Nov 2–4 to pull the OpenAI plan from its projected ~62% landing toward ~100%
  by Nov 4 06:35Z. Re-point scout seats to the reopened OpenRouter pool.
- **A4 — Nov 4–5: pre-spread the returning DeepSeek backlog** across Nov 5–14 (~10 pts/day per
  pool) ahead of the ClinePass (Nov 5) and Go Lab (Nov 7) monthly resets, so cycle 2 doesn't
  repeat cycle 1's binge-and-starve shape.
- **A5 — standing, from the first check: Command Code + Open Design micro-burn.** Route the
  small CLI-shaped job class (one-shot probes, small fixes) through the Command Code resident
  proxy and opendesign seats at a steady trickle (~9 pts/day Command Code) — worth ~28 pts
  before Nov 2 — _after_ a fresh Command Code read exists (§4); do not burn blind.

## 4. Flags

**Projected to finish UNDER-used (queue work against these):**

- **Go Personal (~16 pts, expires Nov 2 21:02 CT)** — work class: the Flash task queue
  (repo-queue worker shifts and issue-creator lanes are Flash-shaped). Rescued by A1.
- **Meta weekly (~52 pts stranded at Nov 2 reset)** — work class: workhorse/specialist sessions
  on spark-contributor, plus subsessions near the wall. Rescued by A2.
- **OpenAI plan weekly (base case ~38 pts stranded at Nov 4)** — work class: Luna Max
  sessions (design + implementation backlog), Sol for genuinely complicated items. A3.
- **Command Code (~28 pts, Nov 2)** — small CLI jobs via the resident proxy. A5.
- **Open Design 7-day (~95 pts)** — tiny; opendesign-seat scout/light work; low value, listed
  for completeness.

**Projected OVER-pace (the Drew flag: slow down or keep progressing?):**

- **Go Lab monthly** — exhausts ~Oct 30–31, ~7 days early. Not really slow-down-able (the work
  is useful and Personal rotation + the trough plan absorb the consequence), but the _trough
  itself_ is the cost to flag.
- **ClinePass monthly** — exhausts ~Oct 29–30, ~6–7 days early; deliberate finishing mode.
- **OpenAI plan weekly (scenario)** — if the ~25 pts/day active rate persists across two
  consecutive checks, it exhausts ~Nov 1. This is the one genuine slow-down-or-push question:
  pushing is Drew's stated bias, but it deepens the Nov 2–5 trough.
- **The Nov 2–5 trough is the composite flag**: three DeepSeek doors closed at once. The plan
  above treats it as survivable (Meta + OpenAI plan + Antigravity carry it); if Drew wants zero
  trough, the only lever is slowing Go/ClinePass burn _now_ — at the cost of stranding quota
  if the slowdown overshoots. Recommendation in-plan: keep progressing, execute A1–A3.

**Too stale / missing to project (named, not guessed):**

- **Antigravity weekly buckets** — no gatherer rows since 2026-10-27T06:00Z; last values
  (Gemini 2%, Claude/GPT 50%) are 2 days old and their resets (Oct 31 / Nov 1) are imminent.
  Blocks trough planning for sonnet/opus seats. Fix: restore the antigravity feed or a daily
  console read.
- **Go Personal burn rate** — % known from one console read (Oct 28 19:33 CDT); no series
  exists because the gatherer only reads the _active_ workspace (openchamber#736's
  per-workspace gap). A1 is sized on the % alone.
- **Command Code** — freshest good read Oct 27 10:05 CDT; gatherer auth failing Oct 29;
  console sign-in pending (standing item). Projection is provisional.
- **Meta freshness** — series stale since Oct 29 04:00Z and the 07:30Z tick errored
  (file-export unparseable). Projectable from the Oct 27 console + export points, but the
  exporter needs a look.
- **Claude** — free tier + provider disabled + reads erroring: no pool to project.
- **Open Design** — last read Oct 28 06:40 CDT, no feed; low stakes.
- **openai.alternate** — one read (Oct 27, 0%); cannot be projected separately from the main
  OpenAI plan pool.

## 5. Proposal: the 4–6h check (build, don't duplicate)

**Vehicle: evolve `go-plan-expiry-watch`** (already every 6h — inside Drew's 4–6h direction;
already reads the Go console and projects in its daily report) **into a full-quota check**,
rather than a sibling job that would double-read the same sources. `pool-quota-steward` keeps
its hourly staleness/flagging role; the watch owns projections + the standing plan. (Job-body
edits follow the standing cron-edit procedure; the 6h cadence itself is unchanged, so no rate
decision is needed beyond what #2173 already carries.)

**Inputs per run:** (1) fresh quota reads — aggregator `quota-current.json` + snapshot-series
tail (must be ≤60 min old per the measurement gate; console reads only for pools the feed
lacks: Go Personal, Command Code); (2) `agent.quota_records` for the subscription line;
(3) the previous standing plan (this document's successor, kept as a versioned file the job
rewrites); (4) `agent.mirror_token_usage_daily` for demand trend.

**Computation per run:** per pool (largest window): current %, reset instant, two measured
rates (24h and 72h, with a post-reset segment rate when a window boundary sits inside the
72h), required rate to land at 100%, projected % at reset, projected exhaustion date;
then the cascade pass — order pools by projected exhaustion, apply fall-through per the route
ladder, recompute the absorber pools' projections, across the next two resets per pool;
diff every figure against the previous plan and name what moved and why.

**Outputs:** the regenerated standing plan (per-pool trajectory + dated audibles, same shape
as this document); a one-line-per-pool status in the watch's report; and only three things
ever escalate to Drew: (a) an **over-pace flag** naming the pool, its projected exhaustion,
and the slow-down-or-keep-progressing question; (b) a pool that goes **unmeasurable** (stale >
24h) — like Antigravity today; (c) an audible that needs a route-config change beyond the
watch's authority.

**Audible execution split:** the check _executes directly_ the audibles that are pure
operations with standing authorization — queueing more work from the named backlogs against
an under-paced pool, Go workspace rotation at the A1 trigger (oauth row activation is a
documented runtime operation), holding a queue class at a pool wall. It _escalates_ anything
that changes route-pool weights/priorities in the deployed config (a code change via PR,
per the fleet's config rules) — A2/A3's weight components are therefore pre-filed as dated
PRs by the check's first runs, ready to merge on the audible date, rather than improvised at
the trigger. Every executed audible is recorded in the plan diff and the watch's state file.

**First two runs** (per #2173 acceptance): the next scheduled watch run produces plan v2 from
fresh reads and either confirms A1's Oct 30 trigger or fires it early; the run after that is
the second consecutive check. If A1 fires, it is also the acceptance criterion's "one real
under-pace pool caught by the check" (Go Personal), with the rotation + queued work as the
receipt.

---

## Addendum 2026-10-29 ~03:10 CDT — Command Code quota reads RESTORED

**Root cause:** every stored Command Code API key had been invalidated server-side. Three distinct key copies existed — the CLI login key in `~/.commandcode/auth.json` (minted 2026-10-23, == the Bitwarden `commandcode.api-key` copy), a second, different key in OpenCode's credential store (`commandcode` integration), and the 1Password copy (same value as the CLI key) — and ALL of them return HTTP 401 `Invalid 'Authorization' header or token` from `api.commandcode.ai/alpha/whoami`. The account itself is healthy: a fresh sign-in minted a working key immediately. The last steady reads were 2026-10-27, with intermittent successes into 2026-10-28 ~14:30Z before the failure became total (aggregator entry: `state: error`, "Command Code authentication failed", via the OpenChamber quota provider reading OpenCode's credential store). This is also the 401 that hit the OpenClaw `commandcode/...` route. The server-side trigger for the invalidation is not visible from here; no local config had changed.

**Fix (per Drew's OAuth rule — agents click through themselves, laptop desktop, no waiting on the pending sign-in card):**

1. `commandcode logout` + fresh `commandcode login` in a tmux TTY on the laptop (the login TUI needs a real terminal; it prints a `commandcode.ai/studio/auth/cli` link with a localhost callback listener).
2. Opened the link in the laptop's headed Helium (staged profile copy, CDP-driven) → Command Code sign-in via GitHub → GitHub demanded Vertex Incorporated org SSO (Microsoft SAML); used the authorize URL's `skip_sso=true` continuation instead (the OAuth app only requests `read:user user:email`, no org data) → signed in → clicked **Authorize** on Command Code's "Authorize CLI Access" page → callback landed, CLI wrote a fresh `auth.json` (2026-10-29T08:01:52Z).
3. New key verified directly: `whoami` 200, `billing/credits` 200. Replaced the OpenCode credential-store row for `commandcode` via the OpenCode server API (create new active row, delete the dead row; verified by hash match).
4. Synced the vaults to the new key, each verified by read-back hash: Bitwarden Secrets Manager `commandcode.api-key` ✅ · 1Password `commandcode.api-key` (Dev) ✅.
5. Verified end to end: `commandcode -p` probe answers; a manual `quota-gatherer` run put `command-code` back to `state: ok` in quota-current.json and appended fresh records to the snapshot series (2026-10-29T08:03:35Z).

**Fresh read (2026-10-29T08:03:35Z, source openchamber-quota):** weekly **13.996% used** (0.84 / 6 credits), resets **2026-10-31T15:16:57Z** (Sat Oct 31, 10:16 AM CT) · 5h **0%** (0 / 3) · monthly credits balance remaining **3.12** (purchased 0, free 0). Shape note for the plan: the API exposes no monthly-percent window — the Oct 27 "monthly 69%" figure does not come from this endpoint; the burn-relevant windows are weekly % + the credit balance. Plan A5 (Command Code micro-burn) is unblocked, sized by the weekly window: ~86% of the weekly cap remains, resetting Oct 31 10:16 AM CT — not Nov 2.

---

## Addendum — 2026-10-29 ~03:14 CDT: Antigravity quota feed REPAIRED (root cause + fix)

**Root cause (three layers).** (1) The OpenChamber quota module's Antigravity provider resolved auth ONLY from the `opencode-antigravity-auth` plugin's legacy accounts file (`~/.config/opencode/antigravity-accounts.json`, fallback the data-dir path). (2) That file was wiped in the 2026-10-27 ~13:45 CDT redeploy of the git-managed `~/.config/opencode` directory — the file is gitignored by design (the plugin adds it to `.gitignore` itself), and nothing recreates it, because the current plugin signs in through OpenCode's credential store instead: the live login sits in `opencode.db` as the `google` integration's active OAuth credential (method `antigravity`), which the interactive paths (plugin, agy) use — that's why interactive Antigravity worked all along. (3) With no file, `isConfigured()` returned false and `listConfiguredQuotaProviders` silently omitted the provider — no error row — so the gatherer produced no Antigravity rows after the 2026-10-27 13:30 CDT tick (snapshot series' last value change: 06:15Z). Dead ends ruled out by direct test: the `~/.gemini/oauth_creds.json` refresh token is bound to the Gemini CLI OAuth client (Antigravity-client exchange → 401 `unauthorized_client`) and its access token gets 403 PERMISSION_DENIED from the Cloud Code quota endpoints — Google gates those endpoints by issuing client, so only the credential-store Antigravity-method login can feed the quota read.

**Fix.** developing-today/openchamber PR #764 (`fix/quota-antigravity-store-auth`, commit 52c74cc19): the provider gains a second auth source mirroring the google provider's store resolution — `resolveAntigravityStoreAuth(auth)` reads the `google`/`google.oauth` OAuth entry from OpenCode's credential store (pipe-packed refresh parsing shared with the google provider); `fetchQuota` tries accounts-file then store candidates, deduplicated by refresh token; `isConfigured(auth)` accepts either source. Plus new `auth.test.js` (7 tests) and a DOCUMENTATION.md decision entry. Validation: antigravity + google vitest suites 63/63, `bun run check:changed` pass, live smoke test through the gatherer's call path. **Activation:** merge #764 (CI checks were still queued in the fleet-wide Actions runner stall at write time) → pull the laptop's `~/git/openchamber-fork` checkout → the gatherer's next 15-min tick runs it permanently. No NixOS switch or other deploy needed.

**Verified end-to-end (2026-10-29T08:13:38Z):** a manual gatherer tick against the patched module (via `OPENCHAMBER_LIB_DIR`) reported `antigravity ok`, providers back to 10, and appended 45 records; `quota-current.json`, the snapshot series, and gatherer state all carry fresh Antigravity rows.

**Fresh read (2026-10-29T08:13:38Z, source openchamber-quota) — replaces the stale Oct 27 values everywhere in this plan:** Gemini pool weekly **5% used** (resets **2026-10-31T13:11Z**, Sat Oct 31 08:11 CDT) · Claude/GPT pool weekly **57% used** (resets **2026-11-01T09:23Z**, Sun Nov 1 03:23 CST) · 5h windows ~0–1%. Plan impact: the trough-cover picture changes materially — the Claude/GPT pool is NOT sitting at half with most of its capacity free; it is 57% spent with 43% left until Nov 1, and the Gemini pool is nearly untouched (95% left) but resets Oct 31, before the Nov 2–5 trough. Recompute audible A3's cover math on these numbers.

**Correction to the addendum above (2026-10-29 ~03:20 CDT):** the first `check:changed` run on PR #764's original commit actually FAILED the anti-slop ratchet (`no-runtime-typeof` — a `typeof` guard I had used in the new store resolver); an early status read masked the failure and the addendum's "check:changed pass" was premature. Fixed in commit `d274ae591` (guard rewritten with the utils' `asObject`, the codebase idiom): gate re-run verified **exit 0, "No new findings / All checks passed"**, focused suites still 63/63, and a second live gatherer tick against the corrected code again landed fresh Antigravity rows (2026-10-29T08:19:23Z, +5 records). Everything else in the addendum stands.
