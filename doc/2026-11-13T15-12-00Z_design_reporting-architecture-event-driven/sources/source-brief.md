# Source — Drew's reporting-architecture directive, 2026-11-13 09:11 CST

> Provenance: Drew's long message in the **Reports** side chat (chat_id
> `f5915c63-26cd-4cd8-990e-c28388681a3c`), sent Fri 2026-11-13 09:11 CST
> (device 0324c94e475e03c7). The **verbatim message lives in that chat's
> transcript**; this file is the faithful working compression the intake ran
> from. Where the compression and the verbatim message could differ, the
> verbatim message governs.

## The directive, compressed

**Chat operating model.** Go one by one through all the side chats that are
just reporting anecdotes or status updates (lane numbers, quota details also
covered on other pages). Target model: each task is owned and updated by its
own owner/originator chat; that chat's agent makes the change in the repo and
pushes it to the backend without reading from the backend. Each task says to
send relevant notes to a category chat — e.g. a Lanes chat receives all
lane-effort notes; Issues & Designs covers all GitHub activity; one chat
(Lanes, Reports, or another) supports queues + PM-stack health + pending
questions. There were pinned channels before per-job side chats proliferated
and they stopped being used. A task ideally never reads its category chat: it
executes, inserts its update into the category chat, and ends. High-urgency
items may also post to the main chat, message other tasks or the
orchestrator, or create queue items / follow-ups.

**Event-driven direction.** Explore better use of Muse hooks; let the PM
stack directly trigger Muse to fill queue items. Consider removing much or
all of the heavy repeated cron jobs, leaving certain reports and checks, and
turning checks like "PM stack reachable" / "OpenCode working" into hook
scripts that fire on failure-to-connect and immediately troubleshoot and
report what occurred. Explore how OpenClaw, OpenChamber, OpenCode,
Cloudflare Workers, and the PM stack work together. Items of a kind arriving
from the GitHub org webhook or catchup scans are logged and inserted into
appropriate databases AND gathered into dedicated queues; hook scripts check
for new items and drain on thresholds (≥10 new items, or an item aged ≥15
min) → triage new issues, review open PRs, ensure a task minds each, report
current and projected quota usage.

**Quota forecasting & efficiency.** Recommendations to improve consistency
and reduce troughs; model changes; predictions for when pools run out and
when new models arrive; times we will be completely down; tracking
predictions and updating methodology until predictions are relatively
accurate, accounting for day-type differences (other days, this time of day,
other Fridays, other weekends) and spike-day modeling (down 10% across the
week → today may spike +20%). Document when quota usage went down but merged
PRs went down further (worse efficiency). Determine model-use changes: are
we hitting rate limits; should routing be round-robin or weighted instead of
ordered for certain providers (config). Perhaps everything is perfect
(doubted). Maybe all we need is another OpenCode plan, a CommandCode full
plan, a GLM plan, or benchmarking models that may perform better than
expected. Send push notifications with useful info.

**Spaces/pages freshness + actions.** Side-channel grouping carries the most
redundant detail other than the Space, which should be updated routinely.
Every update, lane change, and PR merge should update the appropriate other
spaces. Every space gets routine updates, and specific tasks update on their
own as they receive data — no space should be 6 days or 5 hours stale for
information that needs to be current. Push notifications might open specific
pages for an event or generic queue/quota monitoring pages. Pages may be
data-backed and automated: the same data gathered in a JSON blob from a
hook; the space job just replaces the data JSON object at the top (or points
at a different file) — everything else relies on the input data map.

**Mobile-first (hard requirement).** All spaces, reports, and PM-stack pages
fully support mobile viewing as the primary, first-class target: relatively
large font, relatively small width. Desktop may be a separate space/page, a
degraded/generic view, or basic adaptation (tiles in two columns instead of
one). If desktop can emphasize useful things other ways, great — but mobile
must work.

**Dynamic actions / questions module.** Pages may have dynamic actual
actions. The questions module (used in multiple places) can sit next to
facts. A request with a confirmation bubble can use an OpenAI, Anthropic, or
Fable reset once the appropriate monthly/weekly quota reaches 100%; that can
be queued up from mobile. After all other queues are down and we reach 100%
on Claude, it can immediately trigger a cloud reset. Predict how long a
reset lasts as ranges: "between X and Y hours from now", "2.5 to 3.7 days",
"2 days 4 hours to up to 3 days 16 hours", "likely runs out between tomorrow
afternoon and Sunday 2 am" — depending on whether usage continues, based on:
normal rates of the past 30 days; rates used until it was reset last; since
config change 123; past day vs past 30 days. Question sets on errors: Should
I try to fix it? Hotpatch the binary used in the service? Trigger a NixOS
reset? Wait until downtime and then trigger a reset? Ask for more feedback /
other comments / multi-selects. Answers trigger a task in Muse, which
updates that specific page with more information (possibly with another push
notification or a message in the category or main chat).

**Custom report pages.** Build a concept for custom report pages with their
own graphs; a story (start, beginning, end); questions and dynamic updates
over time for specific issues — provider outages, or the Claude
authentication incident (broke in the morning, down 18 hours). The incident
report monitors: progress; trials and tribulations of attempting the fix;
findings; issues to prevent recurrence; related previous incidents and
incident reports; final root cause; remediation short/medium/long term. Not
every quota reset needs a page; useful pages include: activity that happened
today; predicted full wall on the workhorse pool; predicted workhorse pool
fully walled until 8 pm this evening; a special report on pool availability
and quota usage changes over recent time. Diagnostic framing example:
documents used fewer tokens in Muse and way more in OpenCode, but PRs merged
slightly less than before → efficiency is worse and distribution is wrong.
Providers that support many models: we should never use some provider for
Sonnet — only DeepSeek or GLM, etc. Free providers: wire into certain paths
because they often have limited context windows (100k vs 1M); wire many free
providers into small, or if they test well for speed into speed/scout; free
decision models or smart free-for-a-time models could sit in front of
workhorse even with limited context vs the normal 1–1.1M workhorse context.
Track context empirically: some models' real context differs from the
official record (usually smaller, sometimes larger). Many providers don't
provide enough tokens to matter, but if providers work reliably it's worth
wiring them in front whenever available — with reliable fall prevention when
quotas run out or rate limits hit, work moves to a new model and takes the
cache hit. Reports are listed somewhere (like the question-sets list) and
later searchable.

**Atlas ingestion.** All this data and pages are automatically ingested into
Atlas many times a day; agents search for issues/problems. Search biased
toward recency and certain datasets (reports on issues, major happenings) so
an agent learns about a recent issue from a day or two ago and gets resources
to learn more in GitHub or the database.

**Design pages & rollups.** Custom pages for many/most/all designs; designs
with subdesigns categorize and list all subdesigns: why they're needed for
the full epic/roadmap, part of a roadmap, or the originating design document.
A tracker for overall design status and each individual design: idea,
planned, initial issue created, initial issue built out; new designs added
as grandchildren of the initial request; an item can move to fully designed,
then fully designed with implementation issues — with counts (e.g. 50 real
implementation issues: 20 done, 10 in progress, 5 blocked, 15 queued). That
rolls up to the child design (its own items + child designs' statuses and
items + the total sum), building to pages for major roadmap items and epics,
combining details, summaries, reasons why, statuses, updates on issues, and
what was done each day or week. Very long-running items may get a new custom
page report per month or quarter. Link happenings of a date (e.g. Friday,
November 13, 2026) to quota reports and generic pages set to a specific
timestamp. Provide excerpts, graphs, pictures; an hourly or timestamp-based
event-log rollup; anything not easy to skim, provided pretty.

**Daily/weekly/monthly pages.** Daily, weekly, monthly activity pages. PM
Stack page data adapted into spaces for a current view; at daily, weekly,
monthly, quarterly intervals the current view is cloned into a new library
space (unpinned, never updated again) named original + date / week number /
year+quarter. Major roadmap items, major epics, primary binaries, and
projects (Fleet, PM stack, Atlas, ID) each get their own spaces describing:
what exists; what we'd like to exist; what we're actually working on; what
we've designed; inspiration and end state; open research items; links to
reports and design pages in PM stack. Link to other spaces where possible.
PM stack must answer "What did we do today?" — e.g. "We mostly worked on ID
and queue mechanics. For queue mechanics we did A, B, C. For ID we did D and
E, started F, but due to issues 1, 2, 3 pivoted to fixing outages, resulting
in permanent remediations X, Y, Z." Weekly view (Monday morning CT to next
Monday morning CT) as a running tracker: biggest items/issues/designs made,
modified, or built out; what we plan next week; queue-item prioritization
summary and custom report; spaces and links for next items, issues, PRs
required. Summarized daily entries listed backwards in time; done this week;
plans for next week; handled the week before; biggest things (good, bad, or
designed) today, tomorrow, the next days, and going back (yesterday, the day
before…). Context back through Friday of the week before, how it was used,
issues that came up from other days' activity. A daily report is not updated
after its day ends — or updated in the hours immediately after (midnight to
midnight CT for convenience; update until ~2, 5, or 8 am next day as final
details come in). At midday, notes go on today's report: "yesterday we also
achieved this (documented or not); this outcome confirmed/disproved the
hypothesis, or invalidated the benchmark, set up yesterday or 14 days ago."
Weekly reports: small write-back tweaks for a day or two. Monthly: 3–4 days,
maybe a week. Quarterly: a full week. Soft numbers for casual edits. Edits
allowed any time, but after the window, restricted to: quote blocks;
appended items at the end; banners at the top; injecting items into the
event log; a special feedback-after-finalization section. All feed back into
Atlas embeddings and search.

**Linking & metadata.** All pages linked together. Pages linked on any Atlas
page become metadata links to related items, if possible linked somehow.
Embeddings may link classifications to the file that caused them (nearly
identical classifications), embedding links between an item and its linked
items, giving 1–2-step skips so linked items surface together in search.
Metadata may include: URLs exploded into base name, path, file name, last
path part, port, MIME/media type; is it HTML; is it a video; page kind (a
report; a page with comments and a video stream like YouTube; streamed live
like Twitch); a report page with these graphs and likely these libraries
used for dynamic graphs/filters; a specific Python library in this format; a
specific script and JS file used to generate dynamic content or static
pictures; links to all GitHub issues.

**Query-planner boosts / error-driven investigation.** Some embed items get
boosted scores: a query planner seeing a search for reports related to a
recent new error message about a config property that is new or was never
added to the config file pulls files/embeds sorted by recency, prioritizing
reports, specific issues/PRs with certain labels, direct error messages
anywhere. Find clusters of issues/embeds; take one specific line in a
found-bugs-and-errors page for ID and associate it with other pages. Use the
timing when the item was added to the ID troubleshooting event log and the
first appearance of the error string in any embedded logs to get a time
range; pull issues touching the file that now has that string. Gather: any
PRs related to any keywords found; any mentions; any bug-report issues.
Track all issues that might touch that error message in the code (also
uploaded) or places where it might be used; once commits touching those
places are found, review full commits, files, messages and associate commits
to specific PRs and issues. Track in the queue: how many attempts occurred;
what models were used; what worktrees were used. Check for a final commit
never pushed in the worktree. Check session transcripts (we have the session
ID) for any acknowledgement or expectation that the error might occur;
perhaps future encounters weren't aware it was an expected outcome; there may
be a set of instructions for what the session expected to do when this
occurred, which could fix it. Find related items that are fixes for this
class of issue, or related by a similar-family label: "a different fix that
doesn't apply here, but the 'fixed there' rhymes" — usable to permanently
fix this class in this area. With both implemented, we have building blocks
to design a wider architectural fix (impossible to occur anymore); if it
does occur, troubleshooting becomes routine, handled with imperative scripts
or as subtasks of a triage reminder job.

**Reports discovery.** Report pages need not all be on the top nav of PM
stack. There should be places to find all reports — a tree or sitemap — and
a search function (type any page name or text fields on most pages). Pages
use stored data in PM stack (or data uploaded into the PM stack database for
specific reports) so they are permanent records. A custom-pages page or
reports page; a page grouping all troubleshooting reports; a reports
timeline. The timeline shows each quarter; the current quarter's report list
unfolded to monthly, weekly, and daily for the current week and day — quick
access to current day/week/month/quarter overall activity reports. Extra
columns/fields possible: a weekly ID report; a weekly fleet/queues/PM-stack
report; a weekly Atlas/Turso/Cloudflare report with LLM-provider and
tertiary cloud quotas. On the timeline page, overall activity reports are
front and center as the first column; the name is a nice link, possibly with
1–3 big positives and/or negatives for that day/week; folded detail unfolds
to top-level items, with a further unfold for a details view without
clicking into the item (a summary vs the real report, more detailed than the
subject line). Weekly ID/fleet reports appear as bullets in another column
or a dedicated foldable next to each weekly overall report. Additional
reports from a different day (e.g. Tuesday's queue-system overhaul) attach as
a column to the right of, or a subitem inside (most likely a column of) that
specific day — created that day or where the majority of the effort was. A
long-lived design is mentioned as an additional link with phase milestones:
created on the 1st; phase 1 began on the 6th; phases 1–4 completed and 5
begun on the 8th; paused after phase 4 because another design is higher
priority. Monthly and quarterly roll-ups include: overall roadmap; specific
new features; struggles; process improvements; changes in costs and
capabilities; traffic patterns; surprises; learnings. A monthly report goes
into all new ideas and designs, extensions of existing work, and entirely
new work: at a high level, the largest designs started, fully designed, or
fully closed out (implemented or canceled). Inside: breakdown of automation-
suggested ideas-of-the-day and ledger items; brainstorm kernels and rejected
ideas; concepts vaguely mentioned where we chose a different path (could
return to them); details of all design-mill items and smaller designs that
occurred or changed materially that day, with prose explanations; ideally
graphs/charts/flow diagrams for more important designs or designs suited to
visual storytelling (what a feature does; what happens now; what would
change; why now is a blocker; what the change fixes short/medium/permanent;
why this design over others; a picture of how the design could be adapted
and extended). Eventually, building more web pages, apps, games, libraries
with UIs → wireframes, prototypes, screenshots of work in progress or
front-ends built with dummy data; SVGs describing keyframes for apps/games;
examples of levels, enemies, story dialogue; specific inspirations for a
project or phase. Use-case sketches (probably from the ID/federation
material): infinite currencies and exchange rates; graphs representing data
combined with signatures spreading data redundantly; filtering/selecting
appropriate data per server when coordinating (copy data vs provide
references: return resolved items plus items not found, an open async task
to find them checkable until TTL unless renewed, provenance of each returned
item, caching state, first returner, how many remotes provided identical
hash/results, disagreements or invalid signatures, server counts vs
likely-unique users/clusters); a browser user opening a game/world/chat URL
with an automatically generated ID, password or Google SSO linking the
anonymous ID to the primary user — see the identity amendment filed
separately (this intake routes that material to the federation/namespaces
program, issues #9132 / #9121 / #9127, per Drew's instruction that it is an
amendment input, not a new program).

## Intake instruction (from the commissioning brief)

Two deliverables: (A) side-chat audit + proposed target map — report only, no
live rerouting; Drew reviews the map before anything moves. (B) program
intake: starter epic(s) + statement inventory + proto-design scaffold PR,
design-mill pattern, do not narrow (current tooling is collision-check
only). Thread 5 of the original program (identity material) is NOT a new
program: it is an amendment input to the federation/namespaces program —
structured comments on #9132, pointers on #9121 and #9127.
