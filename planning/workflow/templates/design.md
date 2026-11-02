# Meta design template

Use at S0 (`request-intake`): `init` writes a package's first `design.md` in this shape, the intake agent writes the first-pass review order, and S5 (`issue-handoff` fanout) fills "Initial research stages". After intake it changes only through PRs. Waves and the run table are generated views (`ready`, `render`, `status --live`); never type a run status by hand.

```markdown
# Meta design: <request title>

[Main reference](main.md) · [Transcript](transcript.txt) · [Change log](changes.md) · [Human review](human-review.md)

<One paragraph: what this design orders, and the end goal the concepts serve together.>

## Review order

First pass at the order in which to prepare, research and design the concepts. Default, when nothing blocks anything and nothing stands out: run one agent per concept, all at once, and track each in the run table.

1. **C001** <title> — <why here: crucial, horizon now, required by C002>.
2. **C002** <title> — requires C001: <reason>.
3. **C003** <title> — recommended-after C001: can start now; if it does, <what gets reworked>.

## Waves

<!-- workflow:view waves -->

- **Wave 1**: C001, C003
- **Wave 2**: C002 (requires C001)

<!-- /workflow:view waves -->

## Run table

<!-- workflow:view run-table -->

- **C001** <title> — S1 `not-started` · branch none · PR none · updated <UTC>
- **C002** <title> — S1 `not-started` · branch none · PR none · updated <UTC>

<!-- /workflow:view run-table -->

## Initial research stages

Filled at S5. Every GitHub issue made for or related to this request, in recommended order. Issues on the same numbered line run in parallel.

### C001 <title>

- Parent: [#<n> Handoff: Investigate and plan <title>](https://github.com/<issue_repository>/issues/<n>)
- Orchestration: [#<n> Handoff: Orchestrate design of <title>](https://github.com/<issue_repository>/issues/<n>)

1. [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.3) — <one line>. Priority: crucial. Blocked by: none.
2. [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.4) and [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.5) — <one line each>. Priority: recommended. Blocked by: #<n>.

## Links

- Workflow: [stages](../stages.md), [ordering](../ordering.md), [decisions](../decisions.md)
- Order across requests: [DEPENDENCIES.md](../../DEPENDENCIES.md)
- Feature plans land in [planning/plans/](../../plans/README.md), from the [plan template](../../templates/plan.md)
```

- Run statuses: `not-started`, `claimed`, `running`, `pushed`, `pr-open`, `merged`, `validated`, `blocked`, `abandoned`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`.
- A soft edge (`recommended-after`) names the rework it costs; a hard edge (`requires`) names why it blocks ([ordering](../ordering.md)).
