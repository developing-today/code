# FDR-NNNN — <short imperative title>

Copy this file to `FDR-<NNNN>-<kebab-slug>.md`, replace the bracketed fields,
delete the guidance comments, and add the index row in `README.md` — all in the
same commit. Required contents are defined in [README.md §3](README.md#3-required-contents-of-every-record);
this template reproduces that checklist in order.

- **Date:** YYYY-MM-DD (UTC, date written)
- **Status:** proposed | accepted | rejected | superseded | deprecated
- **Owner:** <named team or role that maintains this record>
- **Decision maker:** <who ruled, and the date of the ruling>
- **Scope:** <the semantics this record governs — one line>
- **Supersedes:** <FDR-NNNN / none>
- **Superseded by:** <FDR-NNNN / none>
- **Related records:** <FDR-NNNN, …>
- **Implementation:** <issue/PR links, file paths, queue verbs>

## Context and problem

What forced this decision: the incident, the measured behavior, the specification
requirement. Name the incident and its date, and link it. State the problem in
terms an implementer can act on — not "we discussed the queue", but "a task
cannot name the session that should resume it, so a stopped session's context is
unrecoverable".

## Source links

- Originating issue(s):
- PRs / commits:
- Queue task IDs:
- Dated ruling or other authority:
- Evidence artifacts (audits, runbooks, logs):

## Options considered

| Option     | What it means                                | Assessment |
| ---------- | -------------------------------------------- | ---------- |
| A          |                                              |            |
| B          |                                              |            |
| Do nothing | <only where doing nothing was a real option> |            |

## Decision

State it operationally. An implementer or automation author should be able to
follow it without reading the whole history: which verb changes, which column
exists, what happens on the edge case, what is recorded and by whom.

## Rationale and evidence

**Measured / sourced facts:** <what was observed, where, when — with links>

**Inference:** <what follows from the facts, and what is not proven yet>

## Consequences

- Behavior changes:
- Compatibility / migration:
- New failure modes:
- Monitoring and tests required:

## Revisit triggers

What future evidence or condition should reopen this decision. Be concrete: a
named investigation landing, a measurement crossing a threshold, an implementation
that cannot meet the decision.

## Open questions

Any open sub-choice, with the default currently in effect and what would settle
it. A record with open questions can still be `accepted`; the default is what
implementers follow.

- **OQ-1 — <question>.** Default: <what we do now>. Settled by: <event>.
