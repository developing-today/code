---
name: decision-request
description: "Ask the owner for a decision without stopping the work: one comment that states the question, the options, a recommended default, what proceeds on that default now, and the date the default becomes final. Use when a task needs a product or architecture choice only Drew can make, before applying user/escalated or parking for the owner, when an issue already carries user/escalated with no default recorded, or when a design leaves a question open that the next slice depends on."
---

# Decision request

An escalation that only says "needs Drew" stops the work until he reads it, and he is one person. The program rule already says otherwise: open questions go to Drew "with options + a recommendation; defaults recorded, never stalled on" (#9106 acceptance). This skill is the form that rule takes on an issue.

It does not replace the pending-questions ledger ([`pm-stack/decisions-ledger-system.md`](../../../pm-stack/decisions-ledger-system.md)). That ledger stays the one list of what waits on Drew. This skill fixes what each entry must contain so that most of them stop being blockers.

## First: which door is it?

| Door    | Meaning                                                                                                                                                                                             | What you do                                                                                              |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Two-way | A later PR can undo it at ordinary cost: a name, a default value, an internal interface, which of two libraries, the order of two steps.                                                            | Choose the default, proceed now, label `user/review`. Do not apply `user/escalated` or `status/blocked`. |
| One-way | Undoing it loses data, breaks other people or costs money: a journal or wire format others will hold, deleting user data, a public release, a paid plan, a credential, a licence, a legal position. | Stop that one step, label `user/escalated`, and keep working every slice that does not depend on it.     |

If you cannot say which, write down what undoing it would take. If the answer is "a PR", it is two-way.

## The comment

Post one comment on the issue the decision belongs to, in exactly this shape:

```markdown
<!-- decision-request:v1 door=two-way default=A final=2026-11-28 -->

**Decision needed:** <the question in one sentence, answerable with a letter>

**Default: A** — <what A is>. <"Work proceeds on A now." | "Step N waits; everything else proceeds.">
It becomes final on <UTC date> unless you reply.

**Options**

- **A.** <what it is>. Costs: <what>. Closes: <criterion or issue>.
- **B.** <what it is>. Costs: <what>. Closes: <criterion or issue>.

**Why A:** <one or two sentences of evidence, with a file, measurement or link>

**If you choose differently later:** <what has to be redone, in PRs or hours>

**To answer:** reply with the letter, or with a correction.
```

Rules for the content:

- One decision per comment. Two questions are two comments.
- At most four options. If there are more, you have not finished the analysis.
- The default is a recommendation you would defend, not the safest-sounding one.
- "Costs" and "what has to be redone" are concrete: files, PRs, a migration, a number.
- The final date is 7 days out for a two-way door. A one-way door has no final date: write `final=never` and say the step waits.

## After posting

1. Two-way: continue the work on the default in the same turn. Cite the comment in the PR body.
2. One-way: record the blocked step in the queue checkpoint with the comment URL, then take the next slice that does not need it.
3. Never post the request twice. If the issue already has a `decision-request:v1` comment for this question, link it and move on.
4. When the owner answers, or the final date passes, edit the issue body's criterion to record the outcome and remove `user/review` or `user/escalated`. An issue whose only blocker was an answered question is ready work.

## Repairing old escalations

An open issue carrying `user/escalated` with no `decision-request:v1` comment is an escalation without a default. When a task lands you on one:

1. Read the thread and state the actual question. If there is none (the issue is waiting on access or a physical step), say that in one line and leave the label.
2. Otherwise post the comment above. If the door is two-way, swap the label to `user/review` and remove `status/blocked`.
3. Do not start the work in the same turn unless the task asked for it. The repair is the deliverable.

## Should not load

Questions a search of the repository can answer are not decisions: look them up. Choosing between two designs with evidence on both sides is `design-orchestrate` and its decision protocol. Access requests and credentials are not decisions either: say what is missing and where it would come from.
