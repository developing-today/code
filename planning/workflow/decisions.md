# Decisions

How a choice is recorded, who may confirm it, how disagreements are settled without stopping work, and what a human is asked. The record format is in [records.md](records.md#decisions) and the [decision template](templates/decision.md).

## Source and status

Source says where a decision came from:

- `stated`: the transcript says it. Cite the segments: `stated (T051)`.
- `inferred`: it follows from what is stated or from the repository's existing rules. The rationale names what it follows from.
- `agent-preliminary`: an agent chose it so work could continue.
- `human-confirmed`: a human approved it. Cite the URL of their message.

Status says whether it is settled:

- `preliminary`: in force and open to change. `inferred` and `agent-preliminary` decisions start here.
- `confirmed`: settled. `stated` and `human-confirmed` decisions are confirmed.
- `reversed`: no longer in force. A dated line under Choice says what replaced it.

Only a human confirms an `agent-preliminary` decision. No agent, reviewer or later stage sets `human-confirmed`, however strong the evidence; an agent may record a human's confirmation or reversal by citing their message. Agents may change a preliminary choice (after the challenge pass, for example) by adding a dated line under Choice.

## Coverage

- Every decision has a stable ID, `D001`, `D002`, ..., taken from `ids` under the package's `ids` lease ([collaboration.md](collaboration.md#live-coordination)). An ID is never reused, even after a reversal.
- Every downstream artifact cites by ID the decisions it depends on: design sections, plan steps, issues and PR descriptions. The decision's `Affects` field lists the concepts and items it touches.
- A plan cites every `stated` or `human-confirmed` decision in at least one step ([S7](stages.md#s7-plan)).
- When a decision is reversed, `git grep -l D007` lists exactly what must be reworked. Rework is one issue per affected artifact, citing the reversal; nothing else is touched.

## Disagreement

Two workers with conflicting results, or a `conflicts-with` pair in scope at S6, go through these steps. No one waits for a human.

1. The orchestrator does not decide. It gives the merged material from both sides to three fresh, read-only subagents in parallel, using the prompts in [templates/prompts/](templates/prompts/): the [reviewer](templates/prompts/reviewer.md) checks the facts, the [critic](templates/prompts/critic.md) attacks each side's reasoning, and the [investigator](templates/prompts/investigator.md) looks for options nobody considered.
2. They decide everything the evidence settles, taking the option best practice would choose when in doubt, and list as toss-ups only choices that are genuinely balanced and costly to reverse.
3. The orchestrator files each decision as a record with `Source: agent-preliminary` and `Status: preliminary`. Work continues on the preliminary choice at once; toss-ups go to [human review](#human-review).
4. Challenge pass: before S6 closes, one fresh subagent with the [challenge-pass prompt](templates/prompts/challenge-pass.md) asks of every decision whether its options were too narrow. Each decision it flags is reopened with the investigator prompt, and the outcome is a dated line under Choice.

## Human review

`human-review.md` ([template](templates/human-review.md)) is regenerated from the decision records whenever one changes.

- Minimal: list only preliminary decisions whose options are genuinely balanced and whose wrong choice would be costly to reverse. Obvious decisions are made and recorded, not listed. "None" is a valid list.
- Batched: one list for the package, never questions one at a time.
- Ranked by effect, most consequential first.
- Defaults: each item names the choice already in effect, the alternatives and the cost of reversing.
- Never re-ask what the transcript states or a `stated` or `human-confirmed` decision already answers.
- Nothing waits on the answers. A reversal starts rework scoped as in [Coverage](#coverage).
