# C011: Disagreement Protocol, Reviewer Subagents, and Preliminary Decisions

[Main reference](../main.md#c011)

## Relevant transcript excerpts

<!-- workflow:excerpt C011 58 64 -->

[Transcript lines 58-64](../transcript.txt#L58-L64)

```text
At this stage we've done general research but the detailed design buildouts may find conflicts or have differing opinions about the best approach where it wouldn't be suitable to do both. The main agent shouldn't try to solve that.

If it doesn't immediately know the answer, it should launch reviewer and critiquer sub-agents, which aren't specific things. They are the kind of prompts that should be given to them to:
- analyze the data merged in from both
- make the obvious decisions
- document the choices and the rationale
- keep it to a minimum list as possible for human review
```

<!-- workflow:excerpt C011 67 67 -->

[Transcript lines 67-67](../transcript.txt#L67-L67)

```text
If in doubt just follow best practices and what other experts would think is best. If it's truly a toss-up then they can pull that up and have a human review it. Ideally the work can continue on without this decision as far as possible or make a preliminary decision and then at the end bring all of these preliminary decisions to a human reviewer (who may trigger further rounds of rework if they decide to go with a different approach). While rework isn't ideal, that is a better approach than halting until a human response in this scenario
```

## Normalized purpose

Arbitrate conflicting worker recommendations by dispatching reviewer and critic subagents to make reasoned preliminary decisions without stalling.

## Initial thoughts

- The pipeline must not halt for human input on routine architectural choices.
- Reviewers analyze merged data, adopt standard defaults as agent-preliminary, and log genuine trade-offs.

## Inferred suggestions

- Inference: Run a challenge pass before planning to verify decisions were not framed too narrowly.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
