# Decisions: triage, disagreement, challenge, human review

Read before step 7. [decisions.md](../../../../planning/workflow/decisions.md) owns the protocol. The [decision template](../../../../planning/workflow/templates/decision.md) and [human-review template](../../../../planning/workflow/templates/human-review.md) own the record shapes. This file is how the orchestrator applies them.

The rule that shapes everything here: **the orchestrator does not decide disagreements.** It keeps uncontested decisions that workers made with evidence. It sends every disagreement to read-only subagents and records what they conclude. Work then continues on that conclusion as a preliminary choice.

## 1. Triage after each merge

For every decision record the merged facet added, and every open question in its design record:

| Finding                                                                                                                                                 | Action                                                                                                                                                                 |
| ------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Uncontested: nothing merged or in flight contradicts it, and it cites evidence                                                                          | `kept`. Comment its path on every issue that builds on it.                                                                                                             |
| Source tag wrong: `stated` without a transcript segment, `inferred` without a code or doc citation, or an agent's recommendation recorded as the user's | `retagged`. Fix only the `Source` line, on a bookkeeping PR, with a dated line under "Choice". This is the most common way agent advice becomes a fake human decision. |
| Two facets choose differently on one question                                                                                                           | `routed` (section 2).                                                                                                                                                  |
| A `conflicts-with` edge, or one facet's choice breaks an assumption another facet states                                                                | `routed` (section 2).                                                                                                                                                  |
| An open question with no decision record                                                                                                                | `returned`. Back to its facet worker: restart it if its PR is unmerged, otherwise open a follow-up child issue. Never answer it yourself.                              |
| Fewer than three real options, no "keep what exists" option, or no option outside the author's own framing                                              | Note it for the challenge pass (section 4). Do not block on it now.                                                                                                    |

## 2. Routing a disagreement

1. **Build the merged material once.** Write the question in one sentence and list the facts both sides agree on. Then name the material:
   - side A and side B: branch, PR URL, record paths and author;
   - both together: `git diff origin/<trunk>...<branch A>` and the same for B, or a scratch worktree that merges both;
   - must satisfy: concept file, issue URLs, transcript segments and the end goal.

   Save it as `$LIVE/disagreements/<slug>.md`.

2. **Launch three subagents in parallel**, each given that file and its prompt verbatim: the [reviewer](../../../../planning/workflow/templates/prompts/reviewer.md) checks the facts, the [critic](../../../../planning/workflow/templates/prompts/critic.md) weighs the trade-offs, and the [investigator](../../../../planning/workflow/templates/prompts/investigator.md) looks for options nobody considered. All three are read-only leaves. Save each return verbatim as `$LIVE/disagreements/<slug>.<role>.md`.
3. **Combine** their returns, question by question:
   - all that answered agree: record that choice;
   - two agree and one differs: record the majority choice, put the dissent in the rationale, and list it as a toss-up if its reversal cost is medium or high;
   - all differ, or one calls it a toss-up: run one tie-break critic over the three returns, record its choice as the default, and list it in `human-review.md`;
   - "can both be done: yes": record "both", with how, if no return objects to the cost.

   Never run more than one tie-break round. Copy every "Not checked" line into the record's rationale as a hedge.

4. **Record.** Write one decision record per question from the template:
   - `Status: preliminary`;
   - `Source: agent-preliminary`;
   - `Decided by:` the three subagent names and the orchestration issue URL;
   - options steelmanned as they were returned.

   Commit the records on a bookkeeping branch, open the PR, merge it through the gate, and comment the record path on every affected issue. Workers that are running pick it up at their next 15-minute check. The losing side's facet is never rewritten in place. If its merged design record now contradicts the decision, open a follow-up issue `<branch>-2` for its worker to add a dated `Decision` section that points to the record.

## 3. D numbers

D numbers race across parallel branches. Allocate them under a lease:

1. `rw lease acquire` on `decision-ids-<package>`.
2. Take the highest D number from `ids --package <pkg>` on `origin/<trunk>`, and from every remote branch: `git for-each-ref --format='%(refname)' refs/remotes/origin | xargs -I{} git ls-tree -r --name-only {} -- <pkg>/decisions/`.
3. Use the next numbers, create the files, push at once, then `rw lease release`.

Workers that write decision records follow the same steps. Their briefs carry this section's path.

## 4. The challenge pass

Run it once, after every crucial and recommended child issue has merged and before consolidation:

1. Give every `decisions/D*.md` path, the main reference, the concept files, the transcript path and the end goal to one fresh [challenger](../../../../planning/workflow/templates/prompts/challenge-pass.md).
2. For each `reopen` line, launch an [investigator](../../../../planning/workflow/templates/prompts/investigator.md) with the decision record and the challenger's finding as "Why you are here". Run all the reopens in parallel.
3. Record each outcome on a bookkeeping PR as a dated `## <UTC> Reopened:` section in the decision record. It lists the new options with their steelmen, the result, and who returned it.
   - **Choice kept:** no other change.
   - **Choice changed:** set `Status: reversed` on the old record and write a new record for the new choice that names the old one. Run `git grep -l D<nnn>` to list the files to rework, and give each one a follow-up issue.
4. Re-challenge only the reopened decisions, once. Anything still contested becomes a toss-up with a default.

## 5. `human-review.md`

- **Include only** preliminary decisions that work already builds on, whose options are genuinely balanced, and whose reversal cost is medium or high. "None" is a valid list.
- **Never include** a decision whose source is `stated`, or any question the transcript already answers. Search the transcript for the question's key terms first.
- **Rank** by effect: reversal cost, then whether a `crucial` item depends on it, then the number of affected IDs.
- **Each entry** gives the question, the default in effect, the alternatives, what reversing it would change, and the `git grep -l D<nnn>` count today.
- **Questions:** batch them, give each a default, and never put two dependent questions in one batch.
- Regenerate the whole file from the records whenever a decision changes; never edit one entry by hand. A preliminary decision that is not listed carries `Human review: Not needed: <why>` in its own record.

## 6. Reversal by a human

Only a human moves a decision to `human-confirmed` or `reversed`. When one does, record it verbatim on a bookkeeping PR. `git grep -l D<nnn>` gives the rework set. Each file in it gets a follow-up issue in the stage that owns it: a facet record goes back to S6, a plan to S7, code to S8. Every other line of work continues.
