# Close-out: consolidated record, sub-features, plan issues, bookkeeping PR

Read before step 10. The record shape is the [design record template](/planning/templates/design.md), the folder rule is [doc/DOCUMENTATION_PROTOCOL.md](/doc/DOCUMENTATION_PROTOCOL.md), and the plan issue body is the [plan issue template](/planning/workflow/templates/issue-plan.md). This file says how the orchestrator gets each of them written and merged without writing the prose itself.

## 1. The consolidated design record (step 10)

- **Path.** `doc/<UTC>_design_<concept_slug>/<UTC>_design_<concept_slug>.md`, in the same folder as the facet records. The wave-0 worker (step 3) may already have created it with only "Shared names"; the editor then owns the rest of it.
- **Editor brief.** The brief in [dispatch.md](dispatch.md), role `editor`. It owns only this file and one `changes.md` line. It reads every merged facet record, every decision record and `human-review.md` if one exists, and it never edits them; a facet record it thinks is wrong becomes a comment on the orchestration issue.
- **Outline.** The template's sections, filled like this:
  - **Intent**: the concept's goal in a paragraph, linking its `main.md` section, the concept file and the transcript segments.
  - **Shared names**: the terms, paths, config keys and interfaces pinned in wave 0.
  - **Design**: one sub-section per facet, a paragraph each, linking the facet record. Summarise; don't copy.
  - **Decisions**: one line per decision record: id, choice, status, source, reversal cost, link.
  - **Alternatives considered**: links to the options in the decision records, plus anything the challenge pass added.
  - **Sub-features**: the list in section 2, each with its tier, goal, what it enables, the decisions it builds on and its edges to the others.
  - **Consequences**: what gets easier or harder, and what must stay compatible.
  - **Open questions**: each with its default and who or what settles it. Questions that depend on implementation details are marked "deferred to the plan".
- **Review.** One fresh [reviewer](/planning/workflow/templates/prompts/reviewer.md) in single-PR mode, with "Must satisfy" set to the orchestration issue, the parent issue, every facet record and every decision record. Restart the editor on the same branch with the must-fix list. Stop after two rounds: open a follow-up issue for each must-fix still standing, link it from "Open questions", and merge.

## 2. Sub-features (step 12)

A sub-feature is a slice of the design that can be planned, built, tested and shipped on its own, for example "remote-note scanning" or "the lease command". A concept whose design does not split gets one sub-feature, the concept itself.

For each sub-feature, the consolidated record already states:

- **Tier.** `crucial` (the goal fails without it), `recommended` (worth doing now, the goal holds without it) or `contingent` (only if its trigger fires; name the trigger). Start from the concept's priority and lower it only with a reason.
- **Goal and what it enables.** One sentence each, so the planner can order the work by value.
- **Edges.** `requires`, `recommended-after` (with the rework cost of starting early) and `shares-files-with`, as in [ordering.md](/planning/workflow/ordering.md).
- **Decisions it builds on**, by id, including the preliminary ones listed in `human-review.md`.

Then, for each one:

1. Add an item under the concept: `rw item add --package "$PKG" --parent "$CID" --stage S6 --text "<tier> sub-feature: <name> — <goal>"`. Use the id the tool prints; never invent one.
2. Crucial and recommended: draft a plan issue from the template. Title `Plan: <sub-feature> (<package>·<item id>)`. Add `"item":"<item id>"` to the marker. "Prerequisites" carries the edges as plan issue links once they exist, so create them in edge order. "Current state" names the human-review defaults it builds on.
3. Contingent: the item only, with its trigger in the text. No issue until the trigger fires.

Run the marker search in the [templates README](/planning/workflow/templates/README.md) for every draft, so a retry never makes a duplicate. Give each draft body, and nothing else, to its own [zero-context reader](/planning/workflow/templates/prompts/zero-context-reader.md), all in parallel. Fix the gaps and re-run until none is blocking. Publish with `gh issue create --repo <issue_repository> --title <title> --body-file <file>`, record each URL in `root.md` at once, and confirm with `gh issue view <url> --json url,state,body` that the body ends with its marker.

## 3. The bookkeeping PR (step 13)

On `research/<slug>/s6-<concept>-books`, or `-2`, `-3` if an earlier bookkeeping PR already merged (collaboration rule 4):

- `rw item add --package "$PKG" --parent "$CID" --stage S6 --text "S6 · <UTC> — design at <path>; decisions D007–D012; <n> toss-ups in human-review.md; plan issues #<n>, #<n>"`.
- `rw item set-status --package "$PKG" --id <item> --stage S6 --status clarified|superseded --note "<what S6 found>" --see <item or D id>` for every earlier item the design clarified or replaced. Never remove an earlier finding.
- In `design.md`, under the concept's "Initial research stages" sub-section, add a dated block `S6 · <UTC> · result`: the consolidated record, the decision ids, `human-review.md`, and the plan issues by wave with their tiers and edges. Never rewrite earlier blocks.
- In the concept state block: add the plan issues to `issues` with stage `S6`, the merged PRs to `prs`, set `stage` to `S7`, and append a `history` entry.
- `rw render --package "$PKG"`, then one `changes.md` line.
- `prettier --write` on the markdown you touched, then `rw check --package "$PKG"`.

Merge it through the gate in [merge.md](merge.md), with your own last pushed SHA as the reported one. Then:

1. Comment on the orchestration issue: merged PRs, decision ids, the toss-ups with their defaults, the consolidated record and the plan issue URLs.
2. Comment the plan issue URLs on the parent issue.
3. Close the orchestration issue (`gh issue close <n> --reason completed`).
4. `rw lease release`, and append `end <last pushed sha>` to `root.md`.
