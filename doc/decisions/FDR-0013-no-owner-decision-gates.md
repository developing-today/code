# FDR-0013 — No owner-decision gates

- **Date:** 2026-11-21
- **Status:** accepted
- **Owner:** Fleet queue maintainers
- **Decision maker:** Drew, 2026-11-21, to the session that wrote this record:
  agents must not think "anything needs me or is blocked on me because some
  other session assigned me or said it was my problem"; "it's always a wrong
  agents decisions getting regurgitated as my proclamations"; "my problem is
  agents not actually working on real issues", with agents "turning basic
  operations into security issues".
- **Scope:** which waits on a person may hold, park or block queue work or an
  issue, and what an agent does when work needs a choice.
- **Supersedes:** none. It narrows the `escalated-to-user`, `drew_decision` and
  `person-answer` block classes and the `person_answer` park condition, and it
  settles FDR-0012's OQ-1.
- **Superseded by:** none
- **Related records:** FDR-0008, FDR-0009, FDR-0012
- **Implementation:** the rule and the
  [`decide-and-proceed`](../../.agents/skills/decide-and-proceed/SKILL.md) skill
  in [#12216](https://github.com/developing-today/code/pull/12216); the
  `fleet queue block` and `fleet queue park` guard in the fleet-agent change
  that follows it.

## Context and problem

FDR-0012 removed one dead gate: a lane parking because an issue was assigned to
the shared account. A second gate does the same damage. A lane meets a choice,
decides the choice is the owner's, and blocks: `user/escalated` on the issue,
`--reason-class escalated-to-user` on the queue task, "awaiting owner decision"
in the checkpoint. On 2026-11-21, 108 open issues carried `user/escalated`.

Three things make it worse than a slow answer:

1. **The block is self-authored.** Drew did not assign himself these questions.
   An agent did, in his name, through the account they share.
2. **Agent decisions are restated as his rulings.** A later lane reads "owner
   decides" or "per the app-only rule" and treats it as something Drew said. It
   then parks on, or enforces, a rule no person made.
3. **Routine operations become security questions.** Dispatching a workflow
   with a token the fleet holds, reading a secret by name, enrolling an agent:
   each has been turned into a grant request or an approval wait.

Examples from the review that led here: #805 waited on four protocol choices;
#9112 on a "planning-window confirmation" requested on 2026-11-19; #557 was
blocked "for the owner to re-scope"; #3195 ended "owner decides". Four of the
seven placement surfaces in `pkgs/fleet/fleet-surfaces.json` are `pending` on a
grant or scope, one of them "per the app-only rule", which has no source.

## Source links

- Dated ruling: Drew, 2026-11-21, quoted above.
- Evidence: `scripts/fleet/flow-report` on 2026-11-21 (108 open
  `user/escalated`; 456 of 1,117 open issues blocked, parked or waiting); the
  agent notes on #557, #805, #9112 and #3195.
- Plan: [`planning/plans/2026-11-21_results-first.md`](../../planning/plans/2026-11-21_results-first.md),
  step F4.

## Options considered

| Option                        | What it means                                                                    | Assessment                                                                     |
| ----------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| A. Ask better                 | Escalations carry options, a default and a date after which the default is final | Rejected: still routes every choice through one person, and still reads as his |
| B. Decide, with a veto window | Agents decide but wait a set time for an objection                               | Rejected: a wait with a politer name; the reboot test (FDR-0009) still fails   |
| C. Agents decide; hands only  | The choice is the agent's. Only a step that needs a human body or login may wait | Chosen                                                                         |

## Decision

1. **A choice is the agent's.** When work needs a product, design, scope,
   ordering, naming or tooling choice, the agent makes it, records it on the
   issue (what, why, how to undo, which lane) and continues in the same turn.
2. **Nothing waits on Drew to decide, approve, confirm, re-scope or reassign.**
   No park, block, label, checkpoint or comment may have that as its release
   condition. `user/escalated` does not hold work.
3. **An owner statement needs a source.** A claim that Drew decided or ruled
   something counts only with a link to his own words: an issue or comment
   labelled `source/human`, or a decision record quoting him with a date. Any
   other such claim is an agent's decision and may be replaced on evidence.
4. **Routine operations are done, not escalated.** An operation the fleet
   already performs through the fleet binary (a secret by name, a merge that
   passed its checks, a dispatch to a registered surface, an enrolment, a
   deploy the fleet already does, a token the fleet already holds) is not a
   security review, a grant request or an approval wait.
5. **The rulebook's nevers stay nevers.** They are not questions: an agent does
   not do them and does not escalate them.
6. **Hands are the one real wait.** A step that needs a human body or a login
   only a person can complete (hardware, an interactive browser or MFA prompt,
   a payment, a signature) may block, as one imperative sentence a person can
   do in about five minutes. It blocks that step only.
7. **The queue enforces it.** `fleet queue block` with a person-wait reason
   class, and `fleet queue park` with a person-answer condition, are refused
   unless they name a hands kind and the action. The same guard refuses a
   reason that names an ownership handoff (FDR-0012 OQ-1).
8. **Existing owner parks are ready work.** Any lane that lands on one states
   the question, decides it or re-blocks it as a hands item, removes the
   labels, and returns the task to eligible.

## Rationale and evidence

**Measured / sourced facts:** 108 open issues carried `user/escalated` on
2026-11-21. Queue tasks 11885, 16920, 18051 and 19405 each re-read #1062 and
completed with no code. The four issues named above were each stopped by a
note an agent wrote.

**Inference:** a gate that only one person can open, on a fleet that opens
hundreds of issues a day, fails FDR-0008's release-path requirement in
practice. A wrong decision made today costs one corrective PR. A hundred
correct questions asked today cost a hundred stalled tasks.

## Consequences

- Behavior changes: lanes stop escalating choices. Wrong calls will be made
  and fixed forward; the decision comment on the issue is where to look.
- Compatibility / migration: lane prompts that call
  `fleet queue block --reason-class escalated-to-user` without `--hands` now
  get exit 2 and a message saying what to do instead. The 108 labelled issues
  are worked under rule 8.
- New failure modes: two lanes may decide the same question differently. The
  later decision comment wins and says why. A lane may label a real decision
  as a hands item to stop work; the five-minute, one-sentence test is the
  check, and the guard requires the sentence.
- Monitoring and tests required: guard tests in `fleet-queue`; the flow
  report's count of open `user/escalated` issues, expected to fall to the
  number of real hands items.

## Revisit triggers

A person other than Drew starts directing fleet work, or a class of decision
turns out to be costly enough to undo that it needs a named reviewer.

## Open questions

- **OQ-1 — Should the needs-drew alert throat accept only hands items?**
  Default: the queue guard already limits what reaches it, since only a
  hands block can carry the person-wait class. Settled by: an aggregator
  change, if anything else is found raising alerts.
