# Design decision record: boundary audit & appeals UX (id)

- Issue: developing-today/code#5883 · Parent: #5683 · Programme: epic #2056
- Status: DECIDED (2026-11-08, queue task 12845). Implementation timing is a
  separate decision, per the issue's framing; this record is the design.

## Question

When the residency & boundaries framework (#5683) makes or records a boundary
decision about a user — admit, deny, quarantine, exclude — what do the
decision log, the admin review surface, and the user-facing
explanation/appeal flow look like?

## Parent decisions this record relies on (not re-decided)

From #5683, Drew-ruled:

1. **Capabilities, never defaults.** Boundaries are opt-in capabilities an
   admin or user enables; no deployment inherits a boundary it did not
   choose. The audit/appeals machinery below therefore exists only inside a
   scope (server / world / account) that has enabled a boundary.
2. **Basic frameworks first.** Declaration, evidence, and enforcement
   skeleton in the simplest useful form before any policy language or
   automation.
3. **Evidence has provenance and assurance levels.** Declarations are
   evidence (self-declared, org-attested, provider-verified are different
   levels); public-IP evidence is weak evidence with known error modes.

Nothing in this record changes a parent decision. Where an implementation
choice would have to, it is flagged as a parent-level question, not forked
here.

## Decision

### 1. Decision log: append-only, signed, scoped — the boards pattern

Every boundary decision in an enabled scope appends one signed entry to
that scope's decision log, following the precedent `pkgs/id` already ships
for moderation (`src/boards.rs`: an append-only, signed moderation log
whose entries are statements, not enforcement).

An entry records: the scope; the boundary and the **version of the
rule/list data** applied (lists are data per #5683, so the version is what
makes a decision reproducible); the subject's identifier; the **evidence
classes** consulted with their assurance levels (e.g. "org-attested
declaration", "IP-derived country verdict") — references and derived
verdicts, never the underlying evidence dossier; the outcome; a reason
code from a small, legible vocabulary; the actor (rule evaluation or a
named admin); and the time.

- Entries are never edited or deleted. A correction, an overturn on
  review, or an appeal outcome is a **new entry referencing the original**.
- Enforcement with no log entry is a defect: the entry is written as part
  of the decision, not as a later audit sweep.
- Visibility is scoped: a subject can read every entry about themselves;
  admins of a scope can read that scope's log; there is no cross-scope or
  global log (see alternatives).

### 2. Admin review: a queue over the same log, decided by new entries

Review is a view over the scope's decision log — pending adverse
decisions, filterable by reason code and evidence class — not a separate
database. An admin affirms or overturns; either outcome is a signed log
entry referencing the decision, and an overturn restores access without
rewriting history. Automated (rule-evaluated) decisions are reviewable
the same way as manual ones, and an appeal (below) automatically places
its decision in the review queue. Version 1 has no sampling machinery,
dashboards, or SLA timers (basic frameworks first).

### 3. Explanation & appeal: every adverse decision explains itself

An adverse decision produces, at decision time, a plain-language
explanation addressed to the subject: which boundary decided (in the
scope's own user-facing vocabulary), which **evidence class** drove it
(never another user's evidence, and never raw evidence values beyond the
subject's own), what the outcome is, and **what would change it** (renew
an expired declaration, supply a higher-assurance declaration, correct
the record). The explanation is stored as part of — or alongside — the
log entry, so what the user was told is itself auditable.

Appeal is a first-class action on the decision, not an out-of-band email:
the subject submits a statement and/or new evidence against that decision;
the appeal and its outcome are log entries; the outcome gets the same
plain-language explanation treatment. One appeal per decision, reopening
when genuinely new evidence arrives. An appeal is never itself a negative
signal.

Weak-evidence rule, inherited from the parent: a decision driven **solely**
by IP-derived evidence may not be a final exclusion — at most a reversible
step-up (e.g. request a declaration) — and its explanation must say IP
evidence was the driver, because its error modes (VPN, CGNAT, corporate
egress) are the common case, not the exception.

## Alternatives rejected

- **Mutable audit table (edit/delete rows).** Rejected: an audit trail
  that can be rewritten cannot settle disputes, and it forks the
  append-only precedent the codebase already trusts for moderation.
- **One global decision log.** Rejected: it assembles exactly the
  cross-scope sensitive dossier the parent's evidence-privacy rules exist
  to avoid.
- **Enforcement without explanation ("access denied").** Rejected:
  illegible enforcement is indistinguishable from a bug; the parent's
  whole evidence model only works if a subject can see which evidence
  class decided and act on it.
- **Fully automated appeals (rule re-evaluation only).** Rejected:
  appeals exist for the cases the rules got wrong — weak-evidence error
  modes above all; re-running the same evaluation is not a review.
- **Out-of-band appeals (email / contact an admin).** Rejected: the
  appeal would be unlinked from the decision record it disputes, and
  unauditable.

## Invariants implementation must hold

1. Opt-in only: no boundary enabled, no decision log, no enforcement.
2. Append-only: history is extended, never rewritten; overturns and
   corrections are entries, not edits.
3. No silent enforcement: every enforcement action has its log entry and,
   if adverse, its user-facing explanation.
4. Evidence minimisation: logs and explanations carry evidence classes,
   assurance levels, and derived verdicts — not evidence dossiers.
5. Reproducibility: every entry pins the rule/list version it applied.
6. Subject visibility: a subject can always read the decisions about
   themselves and the explanations they were given.
7. IP-only evidence never produces a final exclusion.
8. Appeals are free of penalty and always reach a human admin of the
   scope.

## Splits

None. Implementation tickets are filed when implementation timing is
chosen (the issue's framing reserves that decision); they should cite
this record and the invariants above rather than reopening the design.
