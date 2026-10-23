# Validating the plan with fresh context

Read in step 11. You wrote the plan, so you cannot see what it leaves out. A validator that took no part in writing it reads the plan as a worker will. Run the mechanical checks first, so the validator spends its context on judgement, not counting.

## 1. Mechanical checks

```sh
python3 -I .agents/skills/feature-plan/scripts/plan_check.py "$PLAN" "$PKG"
```

It reads the steps under the `### Wave <n>` headings and reports:

- a step missing any field of the step format in [plan-outline.md](plan-outline.md#4-the-step-format-step-6);
- a done check with no command in backticks;
- a `Requires:` target that is not a lower-numbered step in an earlier wave;
- a file written by two steps of one wave, and a step without `[P]` that shares its wave;
- with the package path, a `stated` or `human-confirmed` decision that no step cites.

Fix everything it prints before launching the validator. Save the final `ok` line in the log. Outside a package, leave out `"$PKG"`.

## 2. The validator prompt

Give it to one fresh, read-only subagent. Pass paths, the branch SHA and URLs; never a summary of the plan ([collaboration.md](/planning/workflow/collaboration.md#the-rules), rule 9).

```text
You are the plan validator. You wrote none of this plan. Do not edit files; run only read-only commands.

Plan: <path> on branch <branch> at <sha>, PR <url>. Planned against: origin/<trunk> at <trunk sha>.
Must satisfy: <plan issue URL>; <consolidated design record>; <decision record paths>; <human-review.md>;
  the plan format in planning/templates/plan.md, planning/workflow/ordering.md (Plans) and
  .agents/skills/feature-plan/references/plan-outline.md.

Check each point. Cite step numbers, and file:line at <trunk sha> for claims about code:
1. Coverage: every section of the design, every stated or human-confirmed decision and every accepted item
   of <ITEM> is cited by a step, or listed under Scope: Out with where it is tracked.
2. Granularity: each step fits one PR and one fresh session. Name any that does not, and how to split it.
3. Edges: each Requires names a real input; each After states its rework; each file two steps write is
   either reassigned or a Requires; nothing that is only a preference is a hard edge.
4. Waves: no two steps in a wave share a file; [P] steps in a wave share no file and no edge; the critical
   path is the longest Requires chain.
5. Interfaces: every type, path, flag, format or name two steps share has one owner step before its users.
6. Tiers: no crucial step requires a lower tier; every contingent step names an observable trigger.
7. Tests: each step's "Test first" would fail at <trunk sha> (the behaviour it tests does not exist yet);
   each "Done when" is a command that runs.
8. Deferred decisions: each has a default, the step that settles it, the step it must precede and what
   changes if it differs; no step depends on a question that has none of these.
9. Could a worker holding only one step and its links start without asking a question? Name the step and
   the question if not.

Return:
Blocking: <step or section - problem - evidence - fix>
Non-blocking: <same>
Not checked: <what you did not verify>
```

Save the return verbatim as `$LIVE/validator-<round>.md`.

## 3. Acting on findings

- **Blocking.** Fix the plan, re-run `plan_check.py`, push, and give the new SHA to a new fresh validator. Never reuse a validator: it has seen the old plan.
- **Stop rule.** At most three rounds. After the third, each blocking finding still standing becomes a deferred decision with a default (plan-outline section 9), or a `Scope: Out` entry with a tracking issue. List each one in the PR body.
- **Non-blocking.** Fix it if it is cheap; otherwise list it in the PR body. Never let a non-blocking finding start another round.
- **Disagreeing.** If a blocking finding is wrong, write why in the log and the PR body, and give that reason to the next validator with the plan. Planning choices are yours to make; design choices are not (plan-outline section 9).
- **Design gaps.** A finding that the design is silent or contradictory is not fixed in the plan. Defer it with a default and comment it on the design orchestration issue.

Done for step 11 means the last return has no blocking finding, or the stop rule has run and every remaining finding is deferred or tracked.
