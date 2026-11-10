# Implement issues, the orchestration issue and recording

Read in steps 12 to 14. The bodies follow the [implement](../../../../planning/workflow/templates/issue-implement.md) and [orchestration](../../../../planning/workflow/templates/issue-orchestration.md) issue templates; the marker search is in the [templates README](../../../../planning/workflow/templates/README.md). This file says how to fill, check, create and record them without duplicates.

## 1. Titles, markers and branches

Step numbers restart in every plan, and one concept can have several sub-feature plans. So every implement and orchestration issue names the item, not only the concept.

| Issue         | Title                                                                                                        | Marker                                                                                                |
| ------------- | ------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| Implement     | `Implement: <step name> (<package>·<ITEM>·step <n>)`                                                         | `{"package":"<package>","concept":"C001","item":"C001.4","stage":"S7","kind":"implement","step":"3"}` |
| Orchestration | `Handoff: Orchestrate implementation of <sub-feature> (<package>·<ITEM>) as an orchestrator using subagents` | `{"package":"<package>","concept":"C001","item":"C001.4","stage":"S7","kind":"orchestration"}`        |

Each step's issue names its worker branch in "Owned outputs": `research/<slug>/s8-<item>-step-<n>`, for example `research/docs-pipeline/s8-c001.4-step-3`, with the worktree `<worktree_root>/research-<slug>-s8-<item>-step-<n>`. `<slug>` is the package slug, and the item is lowercase.

## 2. Links before the plan merges

The issues are created while the plan PR is open, so a link to `blob/<trunk>/planning/plans/<slug>.md` does not resolve yet. In "Read first", give the trunk link (the one that lasts) followed by `(until PR #<n> merges: <permalink>)`, where the permalink is `blob/<last pushed sha>/planning/plans/<slug>.md#wave-<w>`. A zero-context reader can then follow both.

## 3. Drafting (step 12)

Launch one read-only subagent per crucial or recommended step, all in parallel. Each gets paths and URLs only: the implement template, the plan at its permalink and the step number, the design record, the decisions the step cites, its `file:line` map from step 3, and its branch name. Each returns a title and a body; none creates an issue or writes a file.

- **Blockers.** A blocker's issue number does not exist yet when the drafts are written. Drafts write `{step 1}` for each one; you replace it with the issue number as you create issues in wave order.
- **Current state** is verified against the recorded trunk SHA, with `file:line`. "Blocking steps merged" is `none yet`.
- **Your role.** Keep the template's quote. Add: write the "Test first" test before the change, push it, and show it failing in the PR before the fix; do not fix failures that already exist on the trunk, file them.
- **Prerequisites.** `Blocked by` is exactly the step's `Requires:`, including file-collision edges. `Parallel-safe with` is the other `[P]` steps of its wave. `After:` edges go in "Current state" with their rework, so the worker knows what assumption it runs on.
- **Deferred decisions.** A step that settles one says so and gives the default. A step that depends on one names it and the default it builds on.
- **Not verified** says honestly what the drafter could not check.

Save each draft as `$LIVE/issues/step-<n>.md`.

## 4. The zero-context read

Give each draft body, and nothing else, to its own fresh [zero-context reader](../../../../planning/workflow/templates/prompts/zero-context-reader.md), all in parallel. Fix every blocking question and unresolvable link in the draft, not in a comment. Re-read a changed draft with a new reader, at most three rounds; a question still standing after that gets its answer written into the body as a default. Save each return as `$LIVE/issues/step-<n>.zcr-<round>.md`.

## 5. Creating (step 12)

Create in wave order, crucial first, so every blocker exists before the issue it blocks:

```sh
marker='{"package":"<package>","concept":"C001","item":"C001.4","stage":"S7","kind":"implement","step":"3"}'
# 1. The marker search from the templates README; reuse the URL if it prints one.
# 2. Otherwise:
gh issue create --repo <issue_repository> --title "<title>" --body-file "$LIVE/issues/step-3.md" \
  --blocked-by <issue of step 1>,<issue of step 2> --parent <plan issue number>
# 3. Record the URL in the log at once, then confirm:
gh issue view <url> --json url,state,body,blockedBy,parent
```

Confirm that the body ends with its marker, `blockedBy` holds exactly the issues of the step's `Requires:`, and `parent` is the plan issue. If `--blocked-by` or `--parent` is refused, create the issue without it, add the links with `gh issue edit <n> --add-blocked-by <n,...>` and `--parent <n>`, and keep the `Blocked by:` line in the body either way. Use existing labels only. Contingent steps get no issue; the orchestration issue lists them with their triggers.

## 6. The orchestration issue (step 13)

Keep the template's S8 lines and delete the S6 ones. "The graph" lists every step in wave order, crucial first, one line each:

```markdown
1. [#551 Implement: Lock layout](https://github.com/<issue_repository>/issues/551) — step 1, wave 1, crucial — blocked by: none. Parallel with: #552.
2. [#553 Implement: Lease list command](https://github.com/<issue_repository>/issues/553) — step 3, wave 2, crucial — blocked by: #551 (requires: the lock layout). After: #552 (rework if early: the owner line may gain a field).
3. Step 9, contingent — no issue until: the benchmark in step 7 exceeds 200 ms.
```

"Read first" links the plan (trunk link and permalink, as in section 2), the consolidated design record, `human-review.md` and the [implementation-orchestrate skill](../../implementation-orchestrate/SKILL.md). Run the marker search, give the body to a fresh zero-context reader, fix, then publish with `--parent <plan issue number>`. Comment its URL on every implement issue and on the plan issue (`gh issue comment <n> --body "Orchestrated by <url>"`).

## 7. Recording (step 14)

All on your plan branch, in the same PR:

- **The plan.** Fill `Issues:` in the header (plan, orchestration, then each step's issue by number), and add a Progress line: `- <UTC>: <n> implement issues and orchestration #<n> published`.
- **Plan catalog.** Take the `file-planning__plans__README.md` lease, add one line for the plan under a `## Plans` heading in `planning/plans/README.md` (create the heading above "Plans written before this folder existed" if it is missing), and release the lease after you push.
- **Roadmap.** If the plan's roadmap row exists, link the plan from it and set its status to `planned`, under that file's lease.
- **Package.** Only inside a research package:
  - `rw item add --package "$PKG" --parent "$ITEM" --stage S7 --text "S7 · <UTC> — plan at <path>; <n> steps in <w> waves; orchestration #<n>"`.
  - In the concept state block, add the implement and orchestration issues to `issues` with stage `S7` and `item`, and append a `history` entry. Set `disposition` to `planned` only when every crucial sub-feature of the concept has a merged or open plan PR; otherwise leave it.
  - In `design.md`, under the concept's "Initial research stages", add a dated block `S7 · <UTC> · <ITEM> plan`: the plan, the orchestration issue, and the implement issues by wave with tiers and edges. Never rewrite earlier blocks.
  - `rw render --package "$PKG"`, then one `changes.md` line.
- **Checks.** `prettier --write` on every markdown file you touched, `rw check --repo-root .`, and `rw check --package "$PKG"` inside a package.

Then merge `origin/main` into your branch, push, and mark the PR ready. The PR body says `Closes #<plan issue>`, lists the deferred decisions and any finding the validator stop rule left open, and links the orchestration issue. Comment the PR URL on the plan issue. Never merge your own PR.
