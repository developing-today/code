# Human review template

The S6 orchestrator (`design-orchestrate`) writes `human-review.md` at the package root from the decision records, and regenerates it whenever a decision changes. It lists only the genuine toss-ups: preliminary decisions that work is already built on, ranked by effect, each with its default, alternatives and the cost of reversing. Obvious decisions are made, recorded in `decisions/`, and not listed. Nothing the transcript already states is asked again ([decisions](../decisions.md)).

```markdown
# Human review: <request title>

[Main reference](main.md) · [Meta design](design.md) · [Decision records](decisions/)

Work continues on every default below. To change one, comment on <issue URL> with the decision ID and your choice. A reversal starts rework scoped by `git grep -l <decision ID>`.

<!-- workflow:view human-review -->

- **D007** <question> — reversal cost <low, medium or high> · affects <IDs>
  - Default in effect: <choice>, because <one-line rationale>.
  - Alternatives: <option B>; <option C>.
  - If reversed: <what changes>; `git grep -l D007` lists <n> files today.
  - Record: [D007 <slug>](decisions/D007-<slug>.md)

<!-- /workflow:view human-review -->

## Questions

Batched and ranked by effect. Each has the default in use.

1. <question> — Default: <answer assumed>. Affects: <IDs>.
```

- A decision belongs here only when its options are genuinely balanced and choosing wrong would be costly to reverse. "None" is a valid list.
- Only a human moves a decision from `agent-preliminary` to `human-confirmed` or `reversed`.
