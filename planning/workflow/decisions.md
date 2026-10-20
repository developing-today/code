# Decision Protocol & Human Review

This document defines how technical disagreements, trade-offs, and architecture decisions are evaluated, recorded, and escalated during research and design.

## 1. Decision Records (`decisions/D001-<slug>.md`)

Every architectural choice, technology selection, or contested trade-off is recorded as an explicit decision document:

```markdown
# D001: Structured Storage Engine Selection

- Status: preliminary | confirmed | reversed
- Source: stated | inferred | agent-preliminary | human-confirmed
- Date: 2026-10-19T06:09:02Z
- Affects: C001, C004
- Reversal cost: High (requires database migration)

## Question

What storage backend should index world state events?

## Options Considered

### Option A: SQLite via Rusqlite

- Steelman: Zero-dependency, embedded, robust file locking, mature ecosystem.
- Downsides: Concurrency limits on multi-writer setups.

### Option B: PostgreSQL / Neon

- Steelman: Full network concurrency, branching support, scalable analytics.
- Downsides: External network dependency, operational overhead.

## Choice & Rationale

Option A (SQLite). The platform requires self-contained offline execution; network DB introduces unnecessary latency for local nodes.

## Consequential Human Review Items

None; standard embedded default.
```

## 2. Source Attribution & Promotion Rules

- **`stated`**: Explicitly commanded by the user in the transcript.
- **`inferred`**: Deduced logically from stated requirements or existing codebase patterns.
- **`agent-preliminary`**: Decided autonomously by a subagent or reviewer to avoid stalling progress.
- **`human-confirmed`**: Explicitly approved by the human maintainer.

**Promotion Invariant**: Downstream automated agents may never promote an `agent-preliminary` decision to `human-confirmed`. Only explicit human interaction updates this field.

## 3. Disagreement Protocol

When two concept researchers or subagents arrive at conflicting recommendations or mutually exclusive designs:

1. **Do Not Stall**: The orchestrator does not halt work waiting for user arbitration.
2. **Launch Reviewer & Critic Subagents**: The orchestrator dispatches read-only reviewer and critic subagents (using `planning/workflow/templates/prompts/reviewer.md` and `critic.md`), providing the full merged context from both workers.
3. **Analyze and Decide**: The critic identifies obvious decisions where one option clearly aligns with repository invariants. The orchestrator adopts the most standard, reversible option as `agent-preliminary`.
4. **Log Consequential Items**: If the decision represents an irreversible architectural fork with meaningful trade-offs, it is recorded in `human-review.md`. Work proceeds on the preliminary assumption.

## 4. The Challenge Pass

Before concluding Stage 6 (Design), the orchestrator launches a fresh-context investigator subagent to inspect all decisions. The investigator specifically checks:

- Were the options framed too narrowly?
- Was an obvious third alternative missed?
- Were the downsides of the chosen option understated?

If a decision is found to have been artificially constrained, it is reopened with an investigator subagent before final plan handoff.

## 5. Traceable Reversal

Because every decision record lists the concepts and files it affects (`Affects: C001, C004`), if a human maintainer reverses a decision (e.g. changing D001 from SQLite to PostgreSQL):

```sh
git grep -l "D001"
```

immediately identifies the exact set of documents, plans, and code files that require rework, avoiding uncoordinated drift.
