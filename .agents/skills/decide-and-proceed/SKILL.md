---
name: decide-and-proceed
description: "Make the call yourself and keep working when a task needs a choice, an approval, an owner or a permission. Use before parking, blocking or escalating anything for Drew or the owner; before applying user/escalated, status/blocked or a person-answer park; when an issue, comment, checkpoint or doc says the owner decided, ruled, must decide or must approve; when a routine operation such as reading a secret by name, merging, dispatching a build or enrolling an agent starts to look like a security question; and when you find work already parked on any of these."
---

# Decide and proceed

Drew, 2026-11-21, on what goes wrong: agents think something "needs me or is blocked on me because some other session assigned me or said it was my problem", when "it's always a wrong agents decisions getting regurgitated as my proclamations". And on what the problem actually is: "my problem is agents not actually working on real issues", with agents "turning basic operations into security issues".

So the rule is short. **The choice is yours. Make it, write it down, keep working.** The decision record is [FDR-0013](../../../doc/decisions/FDR-0013-no-owner-decision-gates.md); [FDR-0012](../../../doc/decisions/FDR-0012-no-ownership-handoff-park-gates.md) is its sibling for assignment and handoff.

## The five rules

1. **Nothing waits on Drew.** No park, block, label, checkpoint or comment may have "the owner answers, approves, confirms, re-scopes or decides" as its release condition. If you were about to write one, you have found a decision. It is yours.
2. **Decide in the same turn.** Pick the option you would defend, record it (form below), and continue the work on it. A decision that turns out wrong is fixed by the next PR. Work that waited is not.
3. **An owner statement needs a source.** "Drew decided", "owner ruling", "per Drew" count only with a link to his own words: an issue or comment labelled `source/human`, or a decision record that quotes him with a date. Everything else is an agent's decision that someone repeated. Keep it if it is sound; replace it if the evidence says otherwise, and say that you did.
4. **Routine operations are routine.** Reading a secret by name through `fleet secrets`, merging a PR that passed its checks, dispatching a build to a surface in the registry, enrolling an agent, deploying something the fleet already deploys, using a token the fleet already holds: do them through the fleet binary. Do not open a security review, a grant request or an approval ticket for an operation the fleet already performs, and do not describe a stated requirement as a risk to mitigate.
5. **A never is not a question either.** The rulebook's nevers (never force-push `main`, never print a secret value, never touch `held` work) are not things to ask about. You do not do them and you do not escalate them.

## The one real stop: hands

Some steps need a human body or a login only a person can finish: plugging in hardware, answering an interactive browser or MFA prompt, paying for something, signing something. That is a **hands** item, and it is the only thing that may wait on a person.

```bash
fleet queue block <task> --reason-class escalated-to-user \
  --hands <physical|interactive-login|payment|signature> \
  --action "<one imperative sentence a person can do in five minutes>" \
  --blocker "<what this step is>"
```

- It blocks that step. It does not block the lane, the issue's other slices or anything else: take the next task.
- If you cannot write the action as one sentence a person could do in five minutes, it is not a hands item. It is a decision, and rule 1 applies.
- The queue refuses a person-wait block or park that has no `--hands` and `--action`.

On the GitHub issue, a hands item carries `user/escalated` and one marker line in its body, so every tool can tell it from a parked choice:

```markdown
<!-- hands:v1 kind=interactive-login action="<the same one sentence>" -->
```

`fleet agent next` never hands such an issue to an agent, and the dashboard lists it under "Needs a person's hands". An issue with `user/escalated` and no marker is shown as fleet work and is handed out to be decided.

## Recording a decision

One comment on the issue the decision belongs to:

```markdown
<!-- decision:v1 -->

**Decided:** <what, in one sentence>
**Why:** <the evidence: a file, a measurement, a link>
**Undo:** <what a later PR would have to change>
**By:** <lane or agent name>
```

Never write "Drew" in the **By** line unless rule 3's source is in the same comment. Do not ask for confirmation and do not set a date after which the decision "becomes final": it is final now and revisable always.

## Repairing work that is parked on the owner

When a task lands you on an issue with `user/escalated`, a `status/blocked` whose reason is the owner, or a checkpoint that says "awaiting owner":

1. Read the thread and state the actual question in one sentence.
2. If it is a hands item, re-block it in the form above. Otherwise decide it and post the record.
3. Remove `user/escalated` and `status/blocked`, and return the queue task to eligible (`fleet queue unblock` or `fleet queue wake`).
4. Do the work, or the next slice of it ([`feature-slice`](../feature-slice/SKILL.md)).

## What still holds work

Only a live queue claim, or one of the labels `held`, `user/owned`, `do-not-auto-edit` (and `do-not-merge` for merging). Those are switches Drew sets himself. Assignment, authorship, an old branch, a draft PR, `user/escalated` and "someone said it is his call" hold nothing.

## Should not load

Looking something up is not a decision: search the repository. Choosing between two designs on evidence is `design-orchestrate`. Being out of a credential the fleet has never held is a missing secret: name it and where it would come from, and take the next task.
