---
created: 2026-11-23
last-modified: 2026-11-23
revision: 1
---

# Flag observations — the execution-risk flag dataset

Started 2026-11-23 on Drew's order (15:19 CST): stop treating the platform
execution-risk flag as folklore. From now on we record, per observation,
**what** flagged, **where**, and **how it was executed**, so the pattern can
be figured out from data instead of from accumulated impressions.

The log itself is [observations.md](observations.md). This file is the
schema and the standing practice — the durable part.

## Schema — one row per observation

| Field | What goes in it |
|---|---|
| Date/time | CST (America/Chicago). If the record gives only a window, say so in the cell — never invent a precise time. |
| Actor | The lane / session / seat that ran the operation (e.g. "Lane OA2 (Muse lane)", "laptop OpenCode session oc-code-03"). |
| Repo + content kind | The repo, and what the checkout or content actually was, in plain words (e.g. "opencode fork — legacy share-service fix slice"). "A checkout of code" is not a content kind; say what the work was. |
| Operation | One of: clone / worktree-add / fetch / read / edit / build / copy / install / exec — or the honest words when the record doesn't name it. |
| Route | commander / laptop-agent http / ssh / laptop-ssh wrapper / laptop-local session / sandbox-local / not recorded. |
| Invocation form | fleet subcommand / raw command / wrapper — per the route-form matrix in code#13717. |
| Result | clean / flagged / transport-failed. A transport failure (the route itself broke, e.g. Cloudflare 1010) is **not** a flag and never gets recorded as one. |
| Source | A link to the issue comment or lane record the row rests on. **No row without a source.** |

### Marking conventions

- **[issue record]** — the source is a comment on the item's own GitHub issue/PR.
- **[daily log]** — the source is the contemporaneous daily log (`~/memory/2026-11-23.md` and successors), which digests the lane's report. One step removed from the lane itself; labelled as such.
- **[reconstructed]** — the row was assembled from a lane's final report; part of it (usually the route) is attested only there, not on the issue. Reconstructed rows say which part.
- **†** after a route/form — inferred from the lane standard of the time (Muse lanes reached the laptop via the `laptop-ssh` wrapper); the source record attests the operation, content, and result but does not name the route.
- Note on the wrapper: `laptop-ssh.sh` has delegated to the laptop-agent since 2026-11-04, so wrapper rows from this era effectively ran over the laptop-agent HTTP transport while presenting the wrapper form to the agent. Form and transport are recorded as the agent experienced them; this note carries the equivalence.

## Standing practice (Drew, 2026-11-23)

1. **Every lane that hits a flag appends its row(s) the same day.** So does
   every lane that runs a bounded alternate-route probe. A flag with no row
   is a lost data point.
2. **When a flag fires, one alternate-mechanic attempt happens before
   routing, where the notice permits it** — a different route, a different
   invocation form, a different operation shape — **and both outcomes are
   recorded**: the original firing and the alternate attempt, whatever it
   does. Routing flagged work to laptop sessions remains the standing
   route; it is not a reason to stop gathering the comparison.
3. **Occasionally, when a new kind of checkout work starts and a probe is
   cheap, run one bounded probe via an alternate form and record the
   result either way** — clean results are data too. The dataset is
   useless if it only contains firings.
4. Flagged checkouts are never re-entered (standing rule), so the dataset
   has no within-checkout replications — comparisons are across checkouts,
   sessions, and routes. Keep that limit in mind when reading patterns.

## Reading the log

The top of `observations.md` carries a **preliminary pattern read**: what
the seeded data does and does not support, with the confounders named.
Update it only when new rows actually change what it says, and date the
revision. Do not let it harden into doctrine — the confounders
(session context shared across lanes, content kind, operation kind,
approval state) are exactly what more rows are meant to separate.

## Change log

- r1 (2026-11-23): Migrated from hatch-personal `workspace/flag-observations/` under the DOCS-MIGRATION programme (#5983); living-doc header added. Content unchanged except this header and change log.
