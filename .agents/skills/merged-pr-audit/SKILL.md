---
name: merged-pr-audit
description: "Audit merged PRs against the issues that spawned them: census every PR merged in a window, read each PR body and full diff, compare against its spawning issue (especially Closes/fixes closures), check later PRs for completions before filing gaps, file real gaps/bugs, apply the audit outcome labels, and land a dated record with all raw material as supplements. Use when asked to review merged PRs for completeness, verify that closed issues were fully implemented, or run a lane of a multi-lane audit programme over merged work."
---

# Merged-PR audit

You own the PR-vs-issue comparison: for every PR merged in the window, did it fully implement the issue that spawned it, and did anything get left on the floor?

## Rules of the road

- GitHub reads via `fleet gh api` only; issue/comment writes via `~/tools/gh-post`. Never plain `gh`, never the GitHub connector.
- File your tracking issue first, link it on the programme umbrella, and post progress at EVERY slice boundary.
- Check the umbrella and each item's comments before acting; another lane's claim is a hard stop. Reopen/correction decisions on closed issues belong to the closed-issues lane — send suspect closures there as pointer comments on the umbrella, never reopen them yourself.
- Never use closing keywords in comments on issues that must stay open.

## Outcome labels (create in each repo before applying)

| Label | Meaning |
|---|---|
| `audit:complete` | PR fully implemented its spawning issue / stated intent |
| `audit:partial` | PR partially implemented; remainder filed or already tracked (link it) |
| `audit:gap-filed` | A gap / unfiled next step from this PR was filed as a new issue |
| `audit:bug-filed` | A bug in the merged diff was filed as a new issue |
| `audit:completed-later` | A later merged PR completed/fixed what this PR left undone |
| `audit:no-issue` | No spawning issue; judged on stated intent only |

## Steps

1. **Census.** For each repo: `fleet gh api "repos/<repo>/pulls?state=closed&per_page=100&page=<k>&sort=updated&direction=desc"` until a full page predates the window; keep `merged_at` inside the window. Snapshot the lists to files before judging — the lists grow while you work, and every number in the snapshot must end with a verdict. (If the `fleet gh` cache daemon on 127.0.0.1:4802 is down, start `fleet gh cache-serve`; otherwise calls resolve in-process and take ~2 min each.)
2. **Read each PR in full.** `fleet gh api repos/<repo>/pulls/<n>` (body) and `.../pulls/<n>/files?per_page=100` (diff). No judging from titles.
3. **Read the spawning issue in full.** Extract references, especially `Closes/closes/fixes #n`; fetch the issue and all comments (`per_page=100`, count-confirmed against the issue's comment count).
4. **Judge.** Complete / completed-later / partial / gap / bug / no-issue. BEFORE filing a gap, search the repo's later merged PRs for the completion — things change over time, and a completed gap is a `completed-later` mark on both items, never a new issue.
5. **File what's real.** Gaps, bugs, and unfiled planned next steps go in as issues via `gh-post` in the owning repo, prioritised via the pilot queue, linked from your ledger. Check first whether the remainder is already tracked (standing issues, parent epics) — an already-tracked remainder is a `partial` with the link, not a duplicate.
6. **Mark.** Apply the outcome label to the PR (`fleet gh api repos/<repo>/issues/<pr>/labels -f labels[]=<label>`). Where a completed-later relationship isn't visible from the items themselves, comment the link on the PR/issue.
7. **Slice and enqueue the overflow.** A repo too big for one session gets packetised: batches of ~250 PRs, one `feature_slice` packet per batch to `repo-health-code-pilot`, full PR-number list in `payload.slice`, dedupe key `repo-health:v1:slice:<repo>:issue:<tracking>:slice-<k>`, acceptance = ledger comment on the tracking issue. Packet schema is enforced by the validator — run `fleet queue validate <packet>` on the laptop and fix what it names. If the sandbox→broker route refuses CONNECT, enqueue from the laptop.
8. **Land everything.** Dated record in the code repo's doc/ (`doc/<UTC>_<kind>_<name>/`, same-named .md, line in doc/README.md) containing: the brief verbatim, the label set, this method, the census, findings, the per-PR ledger, the enqueue record — plus ALL raw material as supplements (censuses, per-PR JSON, ledgers, scripts, logs). A summary is never the only record.
