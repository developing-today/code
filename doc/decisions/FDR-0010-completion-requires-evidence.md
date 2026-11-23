# FDR-0010 — Issue completion requires deliverable evidence from the bound work

- **Date:** 2026-11-17
- **Status:** proposed
- **Owner:** Fleet queue maintainers / fleet supervision owners
- **Decision maker:** Policy direction from the #9155 closure, 2026-11-12; the specific closing automation remains under investigation
- **Scope:** when an automation may label an issue done or close it, relative to active bound work
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-0005 (attribution is what makes the closer identifiable), FDR-0006 (stop/park remediation), FDR-0009 (reboot test)
- **Implementation:** [#9221](https://github.com/developing-today/code/issues/9221) (closure records + reason-required closes); [#9177](https://github.com/developing-today/code/issues/9177) (completion guard + detective control); prior defect [#7610](https://github.com/developing-today/code/issues/7610)

## Context and problem

On 2026-11-12, epic [#9155](https://github.com/developing-today/code/issues/9155)
was labeled `status:done` and **closed by an automation 8 minutes into an actively
working bound Summit session**, before any deliverable existed. Two separate
failures compounded:

1. **The completion claim was false.** No deliverable existed; the session was
   still working.
2. **The claim was unattributable.** The closing automation has not been
   identified from the GitHub surface, because every fleet writer shares one
   account and the close carried no run identity (see FDR-0005).

The prior pattern is recorded: [#7610](https://github.com/developing-today/code/issues/7610)
covers the bot closing Refs-only issues on PR merge even when the packet required
the issue to stay open. The #9155 close is the same class of defect with a larger
blast radius: a live, bound, in-progress epic closed as if complete.

Sources: the #9155 closure, 2026-11-12 (seed in
[#9180](https://github.com/developing-today/code/issues/9180));
[#9221](https://github.com/developing-today/code/issues/9221);
[#7610](https://github.com/developing-today/code/issues/7610);
[#9177](https://github.com/developing-today/code/issues/9177).

## Source links

- Policy direction: the [#9155](https://github.com/developing-today/code/issues/9155) closure, 2026-11-12
- Investigation: [#9177](https://github.com/developing-today/code/issues/9177) — the unidentified App-identity writer
- Companion: [#9221](https://github.com/developing-today/code/issues/9221) — closure records, reason-required closes, the implemented/superseded/cancelled/started-incorrectly taxonomy
- Prior defect: [#7610](https://github.com/developing-today/code/issues/7610)
- Attribution dependency: FDR-0005

## Options considered

| Option                                              | What it means                                                                                                     | Assessment                                                                                                                     |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| A. Completion requires evidence from the bound work | No automation marks done or closes while an active bound task exists and no completion record from that work does | Chosen. Directly prevents the #9155 shape                                                                                      |
| B. Require a reason on every close                  | #9221's reason-required closes                                                                                    | Necessary but not sufficient alone: a wrong reason passes validation; the guard is the bound-work check, not the reason string |
| C. Detect and reopen after the fact                 | Detective control only                                                                                            | #9177 is building this; it repairs but does not prevent, and the reopen itself needs evidence                                  |
| D. Do nothing                                       | Automations keep closing on their own signals                                                                     | The measured failure stands, and #7610 shows it recurs                                                                         |

## Decision

- An automation **must not** label an issue `status:done` or close it while:
  - an **active bound session or task** exists for that issue, **and**
  - there is **no completion record from that bound work** (a deliverable, a PR
    the task produced, or an explicit completion note written by the bound work).
- The only accepted evidence is **deliverable evidence tied to the bound work** —
  not a merge that merely references the issue, not a label someone applied, and
  not inactivity.
- A close that is not completion uses a different reason from [#9221](https://github.com/developing-today/code/issues/9221)'s
  taxonomy (`superseded`, `cancelled`, `started-incorrectly`), each with its
  required reference. "Implemented" is reserved for evidence-bearing completion.
- Where an automation cannot determine bound-work state, it **must not close** —
  unknown is not permission.
- Every close records actor/run identity and reason into the transaction record
  (FDR-0005), so the #9155 question becomes a lookup.

## Rationale and evidence

**Measured / sourced facts:** the #9155 close happened 8 minutes into an active
bound session with no deliverable, and the closing automation is unidentified from
the GitHub surface. [#7610](https://github.com/developing-today/code/issues/7610)
records the same defect class (automated close of work that should stay open).
[#9221](https://github.com/developing-today/code/issues/9221) records that every
such closure currently has to be reconstructed by hand.

**Inference:** reason-required closes (option B) cannot bear this alone because a
reason is a claim, not evidence; validating the claim's **form** does not validate
its **truth**. The bound-work check is the cheapest test that would have blocked
the #9155 close: the task was live and no completion record existed.

**Why this is `proposed`, not `accepted`:** #9180 directs that it be recorded as
proposed because the specific closing automation remains under investigation
([#9177](https://github.com/developing-today/code/issues/9177)). The policy
direction is settled; the enforcement point and its owner are not.

## Consequences (if accepted in this form)

- **Behavior changes:** closing automations gain a bound-work gate; some closes
  become blocks that require a human or the bound work itself to complete.
- **Compatibility / migration:** closes now depend on task state being readable;
  the #9221 `--reason` requirement changes every existing close call site.
- **New failure modes:** a stale bound task (dead session, never released) blocks
  a legitimate close — the escape is [#9221](https://github.com/developing-today/code/issues/9221)'s
  non-completion reasons plus the manual release path (FDR-0002), not a bypass of
  the gate.
- **Monitoring and tests required:** a test that a done-label/close attempt during
  an active bound task with no completion record is refused; a test that a close
  with recorded evidence succeeds; a test that unknown bound-work state refuses to
  close; the detective control from #9177 counts violations.

## Revisit triggers

- [#9177](https://github.com/developing-today/code/issues/9177) identifying the
  closing automation — **this is the settling event**; the record is then accepted
  in place or amended with the enforcement point.
- [#9221](https://github.com/developing-today/code/issues/9221) landing
  reason-required closes — supplies the taxonomy this record refers to.
- A legitimate close being blocked repeatedly by stale bound state, indicating the
  bound-work read is unreliable rather than the gate being wrong.

## Open questions

- **OQ-1 — what counts as a "completion record" from bound work?**
  **Default in force:** a PR authored by the bound work (merged, linked to the
  issue), an artifact the task produced and linked, or an explicit completion note
  written under the bound session. Settled by: [#9221](https://github.com/developing-today/code/issues/9221)'s
  record shape and [#9177](https://github.com/developing-today/code/issues/9177)'s findings.
- **OQ-2 — what is "active"?** **Default in force:** a claimed/bound task that has
  not been released, completed, or parked with a term (FDR-0008). A parked task
  with a term is not active for this gate. Settled by: [#9177](https://github.com/developing-today/code/issues/9177).
