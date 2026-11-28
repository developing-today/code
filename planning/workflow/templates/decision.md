# Decision record template

One file per decision, `decisions/D001-<slug>.md` (ID from `ids`), written at S6 by the worker or decision subagent that makes it. The orchestrator does not decide disagreements; it records what the reviewer, critic and investigator conclude ([decisions](../decisions.md)). Nothing downstream promotes `agent-preliminary` to `human-confirmed`; only a human does.

```markdown
# D001: <question, short>

- Status: <preliminary, confirmed or reversed>
- Source: <stated, inferred, agent-preliminary or human-confirmed>
- Date: <UTC>
- Affects: <IDs: C001, C001.3, D002>
- Reversal cost: <low, medium or high> — <what reversing would redo>
- Decided by: <agent names, with the PR or issue URL>

## Question

<The question, with the facts both sides agree on.>

## Options

### A: <option>

- Steelman: <the strongest case for it>
- Against: <the strongest case against it>

### B: <option>

- Steelman: <...>
- Against: <...>

## Choice

<A or B>.

## Rationale

<Why, with evidence links. Say which best practice or expert view it follows when the evidence alone does not settle it.>

## Human review

<"Not needed: <why it is obvious>", or "Listed in human-review.md: <why it is a genuine toss-up>".>
```

Reversing a decision: set `Status: reversed`, add a dated line under "Choice", and find the rework with `git grep -l D001`.
