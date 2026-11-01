# Meta design template

Use at S0 (`request-intake`): `init` writes a package's first `design.md` in this shape, the intake agent writes the first-pass review order, and S5 (`issue-handoff` fanout) fills "Initial research stages". After intake it changes only through PRs. Waves and run status are generated views (`ready`, `render`, `status --live`); never type a run status by hand.

```markdown
# Meta design: <request title>

[Main reference](main.md) · [Transcript](transcript.txt) · [Change log](changes.md) · [Human review](human-review.md)

<One paragraph: what this design orders, and the end goal the concepts serve together.>

## Review order

First pass at the order in which to prepare, research and design the concepts. Default, when nothing blocks anything and nothing stands out: run one agent per concept, all at once, and track each in run status.

1. **C001** <title> — <why here: crucial, horizon now, required by C002>.
2. **C002** <title> — requires C001: <reason>.
3. **C003** <title> — recommended-after C001: can start now; if it does, <what gets reworked>.

## Waves

<!-- workflow:view waves -->

- Wave 1 can start now: everything it requires is done. Later waves assume the earlier ones finish. Concepts that share files never share a wave. Leases and open branches are live data, so run `ready` before dispatching.
- **Done:** none
- **Running:** none
- **Wave 1:** [C001](main.md#c001), [C003](main.md#c003)
- **Wave 2:** [C002](main.md#c002)
  - [C002](main.md#c002) is better after [C001](main.md#c001): <rework if C002 starts early>

<!-- /workflow:view waves -->

## Review status

<!-- workflow:view run-status -->

- S0 is `validated` when `render` runs, because render only runs on a package that passes `check`. Other stages show the `tasks` entries that record `concept`, `stage` and `status`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`, so they never appear here.
- **C001** [<title>](main.md#c001): S0 `validated` · S1–S8 `not-started`
- **C002** [<title>](main.md#c002): S0 `validated` · S1 `blocked` (requires C001) · S2–S8 `not-started`

<!-- /workflow:view run-status -->

## Initial research stages

Filled at S5. Every GitHub issue made for or related to this request, in recommended order. Issues on the same numbered line run in parallel.

### C001 <title>

- Parent: [#<n> Handoff: Investigate and plan <title>](https://github.com/<issue_repository>/issues/<n>)
- Orchestration: [#<n> Handoff: Orchestrate design of <title>](https://github.com/<issue_repository>/issues/<n>)

1. [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.3) — <one line>. Priority: crucial. Blocked by: none.
2. [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.4) and [#<n> Design: <facet>](https://github.com/<issue_repository>/issues/<n>) (C001.5) — <one line each>. Priority: recommended. Blocked by: #<n>.

## Links

- Workflow: [stages](/planning/workflow/stages.md), [ordering](/planning/workflow/ordering.md), [decisions](/planning/workflow/decisions.md)
- Order across requests: [DEPENDENCIES.md](/planning/DEPENDENCIES.md)
- Feature plans land in [planning/plans/](/planning/plans/README.md), from the [plan template](/planning/templates/plan.md)
```

- Run statuses: `not-started`, `claimed`, `running`, `pushed`, `pr-open`, `merged`, `validated`, `blocked`, `abandoned`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`.
- A soft edge (`recommended-after`) names the rework it costs; a hard edge (`requires`) names why it blocks ([ordering](../ordering.md)).
