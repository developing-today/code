# Project-tracking tooling evaluation — GitHub Projects vs Linear vs self-hosted (2026-10-23)

> Living document — committed 2026-11-11 from ~/workspace/your_files/github-projects-vs-linear-2026-10-23.md as the tracking home for a completed design (60-day design register entry P1.1; register committed as pm-stack/design-register-2026-11-11.md in developing-today/code, tracking epic developing-today/code#8805). Status: shipped. Update this doc as the design shifts; the register is the frozen audit snapshot.

---

# GitHub Projects vs Linear — for running work across the fleet's repos

Written 2026-10-23 for Drew. Question: one place to see "what's next" across many
GitHub repos, at two levels — (1) work in progress / planned upcoming, granular
but curated, features and fixes; (2) a higher-level roadmap for planning and
prioritization. Neither level should list every GitHub issue.

Your terrain, so the comparison is honest: the work lives in GitHub issues and
PRs across developing-today/{code, opencode, openchamber} plus your personal
dezren39/nix; volume is high (hundreds of issues, ~740 PRs merged in the last
week alone); and a fleet of agents already creates, triages, and closes GitHub
issues around the clock. Any tool we pick has to survive that volume and be
maintainable by agents, not just by you clicking.

## Bottom line

**Start with GitHub Projects.** It's built out of the issues and PRs that
already exist, costs nothing extra, has both levels you asked for (board +
roadmap views over one item pool), and your agents can run it with the `gh`
tooling they already use. **Linear is the better _product_** — faster, more
opinionated, stronger cycle and roadmap mechanics — but it introduces a second
system of record next to GitHub and a per-seat bill, and its advantages matter
most to human teams, less to a solo operator whose workforce is agents.
Choose Linear instead if the deciding factors below flip for you.

## How each maps to your two levels

### Level 1 — work in progress / upcoming (the "what's next" board)

- **GitHub Projects:** a Board view on Status (Todo / In Progress / In Review /
  Done). Issues _and_ PRs are items, so "PR awaiting merge" sits next to the
  issue it closes. An Iteration field gives you sprint-style buckets
  (`sprint:@current` filtering), and board columns support soft WIP limits
  (a warning signal, not enforcement). Curation is the auto-add workflow: per
  repo, a filter (e.g. `is:issue is:open label:now`, `label:next`, assignee)
  decides what ever reaches the board; everything else stays in the repo
  backlogs. Built-in workflows move status on close/merge and auto-archive
  stale done items (caps: 50,000 active items, 10,000 archived — no longer a
  constraint at your scale).
- **Linear:** Cycles are first-class — recurring 1–2 week buckets with
  auto-rollover of unfinished work, scope history, and a triage inbox as the
  front door. PRs link automatically (branch name / issue ID), and statuses
  follow the code: branch created → In Progress, PR merged → Done. Filtering
  to "what's next" is saved custom views. This is genuinely smoother than
  GitHub's — Linear was designed around exactly this loop. The seam: work
  either lives in Linear (with GitHub linked) or syncs from GitHub issues;
  your fleet produces GitHub issues natively and constantly, so you'd run the
  GitHub sync and accept a mapping layer between the two.

### Level 2 — roadmap / planning & prioritization

- **GitHub Projects:** a Roadmap view (timeline bars from Start/Target date
  fields, zoom by month or quarter, milestone markers) over feature-sized
  items. Feature issues with sub-issues show rollup progress; Priority and
  Size fields drive ordering. Practical pattern: a second, coarser project
  ("Roadmap") whose items are only features/initiatives, kept separate from
  the work board so planning isn't drowned by fixes. Insights charts
  (status distribution, burn-up style) exist for a quick health read.
- **Linear:** this is Linear's home turf. Projects carry target dates,
  milestones, and written status updates; **Initiatives** group projects into
  a true top level with their own timeline and health — the closest thing to
  a planning surface designed for prioritization rather than a view over
  tickets. If you end up doing quarterly planning with other humans, this is
  the feature GitHub can't match today.

## The deciding differences, for your setup specifically

1. **Source of truth.** Projects adds fields _onto_ your GitHub issues; the
   repos stay canonical and nothing can drift. Linear duplicates the tracker:
   even with GitHub sync, statuses and comments live in two places and the
   sync rules become one more thing to debug. With an agent fleet writing to
   GitHub all day, no-sync is a structural win.
2. **The personal-repo seam.** A GitHub project belongs to one owner. An org
   project covers developing-today's three repos cleanly; dezren39/nix sits
   under your personal account and can't sit in the same org board (it would
   need its own small personal project, or a link). Linear's workspace
   connects multiple GitHub orgs/accounts at once, so this seam doesn't exist
   there. This is Linear's single strongest point for you.
3. **Agent maintainability.** Projects: full GraphQL API + `gh project`
   commands; the sweeps already authenticate as you and can promote items
   from their backlog files onto the board, set fields, and archive — no new
   credentials, no new tool to teach them. Linear: also a good GraphQL API,
   but it needs its own API key, workspace conventions, and prompt changes
   across the fleet before agents can curate it reliably.
4. **Cost.** Projects: $0 incremental — included with the GitHub plans you
   already hold. Linear: Free tier is a trial for you (250 non-archived
   issues — you'd blow past it in weeks at fleet volume); Basic is
   **$10/user/month billed yearly** (5 teams, unlimited issues), Business
   $16 (unlimited/private teams, Insights). Solo, that's $120/year for the
   tier you'd actually run — small, but it's a new recurring line, and it
   only pays off if _you_ live in the UI enough to feel the UX win.
5. **Feel.** GitHub Projects is a spreadsheet that learned boards: flexible,
   occasionally clunky, filters are powerful but fiddly. Linear is famously
   fast and keyboard-driven and decides your process for you (cycles,
   triage, standard states) — delightful if you accept its opinions,
   constraining if you don't.

## If you pick GitHub Projects (recommended setup)

- One org project, "Fleet — Now & Next": fields Status, Priority (P0–P3),
  Size (S/M/L), Iteration (2-week), Start/Target dates.
- Views: **Board** (Status columns, filtered to current iteration + In
  Progress), **Up Next** (table, next iteration, sorted by Priority),
  **Roadmap** (timeline grouped by repo, quarter zoom).
- Auto-add per org repo with label gates (`now`, `next`); sweeps manage the
  labels during triage, so the board stays curated without you touching it.
- A second org project, "Roadmap", feature-sized items only, for the
  planning level; nix gets a small personal project mirroring the same
  fields until/unless it moves under the org.

## If you pick Linear instead

- One workspace, one team per repo area (Basic caps at 5 teams — you'd fit:
  code, opencode, openchamber, nix, meta), GitHub integration on all four
  repos + issue sync from the org repos, cycles at 2 weeks, one Initiative
  per roadmap theme. Budget $120/year (Basic, solo seat, yearly billing).

## What would change the recommendation

- You start planning regularly _with other people_ (Linear's initiatives and
  updates are built for an audience).
- The personal/org repo seam starts hiding real work (Linear removes it).
- The Projects UI friction measurably slows your morning readout — the thing
  this is for. (Worth a two-week trial of Projects first, since it's free and
  the setup is an afternoon of agent work; Linear's trial can run after, with
  the label hygiene already in place.)

Sources: GitHub Docs (projects automations, auto-add/auto-archive filters,
50,000-item limit); Linear pricing page as recorded Sept–Oct 2026 by multiple
pricing trackers (Free 2 teams/250 issues; Basic $10/user/mo yearly; Business
$16). Prices verified 2026-10-23; re-check Linear's page before purchase.
