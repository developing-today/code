# Capabilities: what we can do now

The capabilities timeline from Census A: each capability, the week it
landed, and its citation. "Landed" means the cited source records it
working, not merely designed. Thirty entries, W40 to W48 — the timeline
starts in W40 because that is where the evidence starts; earlier design
records unlocked plans, not capabilities.

| Capability | Week | Evidence |
| --- | --- | --- |
| Nix-packaged mcpx under a continuous upstream watch and rebuild loop | W40 | Daily notes, 2026-09-30 ("nix/mcpx program phase 2 COMPLETE"); rebuilds on upstream #308 and #323 |
| ID/worlds platform phased design (phases 0–8) | W41–W42 | `doc/` phase records, 2026-10-11 to 2026-10-13 |
| Automated issue sweeps that file, fix and merge across repos | W42 | Daily notes, 2026-10-17 (code PR #245 merged) and 2026-10-18 (openchamber PR #50 merged) |
| CI concurrency groups cancelling superseded runs | W42 | Code PR #289 |
| PM aggregator: live needs-you and quota panels; orchestration section; dashboard slice 3 | W43 | Code PRs #892, #900, #920 |
| Google Drive cold store (pack, resumable verified upload, range fetch) | W43 | Code PR #923 |
| Claude Code resident bridge via claude-code-proxy | W43 | Code PR #875 |
| Hourly stewardship commission | W43 | Cycle log archive; inaugural cycle 2026-10-24T07:37:51Z |
| Full queue cutover (one fleet queue) | W44 | Daily notes, 2026-10-26 |
| Atlas in production on Turso; canonical merge; Atlas migrated to the laptop | W44 | Daily notes, 2026-10-26 and 2026-10-28 |
| Live quota readback on `/quota` | W44 | Code PR #1329 |
| pm-dispatch service deployed | W44 | Daily notes, 2026-10-30 |
| Meta Muse direct door (sole Meta route) | W44 | Daily notes, 2026-10-30 |
| Supabase cut over to muse-dev2 | W44 | Daily notes, 2026-10-30, confirmed 2026-10-31 |
| laptop-agent (laptop exec without SSH) | W45 | Daily notes, 2026-11-04 |
| Fleet queue and secrets brokers; repo-queue to fleet migration | W45 | Issue #5362; daily notes, 2026-11-04 |
| mcpx consolidation; the openchamber enable switch cascades to opencode and mcpx | W45 | Long-term memory record (consolidation complete 2026-11-04) |
| First lab Cloudflare Tunnel (`lab.security.cab`) | W45 | Live 2026-11-06; parent issue #6066 |
| Projects Overview and AI Plans and Usage Spaces | W45 | Daily notes, 2026-11-07 |
| Yard reaper for the fleet repo yard | W45 | Issue #7093; code PRs #7820, #7829 |
| Free-tier provider doors in the route pools (Groq, NVIDIA NIM, OpenRouter, GLM free) | W46 | Code PRs #10558, #10561 |
| ID native shell: live WorldClient session plus Screen renderer | W46 | Code PRs #10526, #10536 |
| Fleet access without SSH, verified (`fleet agent next` from the sandbox) | W47 | Verified 2026-11-21, recorded 2026-11-22 |
| Any-agent, any-name queue claiming | W47 | Issue #12544, PR #12558, deployed 2026-11-22 |
| Fork sync: fleet main to all 102 forks | W47 | dezren39/fleet PRs #93 and #94; issue #12490 |
| Broker stack self-healing on init (cold-tested, five legs) | W47 | 2026-11-22 record |
| Queue durability: SQLite coordination replica; Turso-to-D1 backup plus restore drill | W47 | Code PRs #12692, #12685 and #12686 |
| Atlas book published (57 chapters, 108,391 words); ID book published (67 chapters, about 126,700 words); mcpx book aligned and published (84 chapters, about 288,000 words) | W48 | Code PRs #13288, #13358, #13413 |
| Desktop Commander self-hosted endpoint plus OAuth 2.1 front door; fleet-static rollout with queue-broker generation 6; queue packet repair verb applied to production (37 rows); in-house interim primary CA; fenced jurisdiction migration control plane in ID | W48 | Code PRs #13427 and following, #13488, #13347; issues #13404, #13191, #9155 |

Two source disagreements the census recorded, carried here because they
touch this timeline: the books are secondary sources — where a book's
roadmap and the daily notes differ on what shipped (the Nov 8–12 Atlas
records are designs, several marked design-only in the book itself), the
notes and GitHub win, and those records are counted above as design work,
not capability. And fork merged-PR counts in W43 and W44 came back from
GitHub search near-identical between the two forks (181 and 181; 366 and
365); they are reported as returned, not adjusted.

## What we can do now

The closing summary, as of 2026-11-23. A new agent can enrol, claim a queue
job under any name, resolve secrets by name from a broker, search a
production corpus with provenance on every result, dispatch a build to
where capacity is, reach the laptop without SSH through several doors, and
read complete books about every one of those systems. The project can run
dozens of agent lanes against one queue, keep its quota burn visible live,
sync its fleet repository to 102 forks on every push, issue certificates
from its own CA, and restore its queue from a drilled backup. What it
cannot yet do is the merged list: ingest its corpus automatically, search
across Atlas and ID as one system, boot the basement cluster, or present a
finished record for the 25 weeks of 2026 that no source covers. Those are
items 8, 12, 6 and the honest gap — in that order of candour, if not of
rank.
