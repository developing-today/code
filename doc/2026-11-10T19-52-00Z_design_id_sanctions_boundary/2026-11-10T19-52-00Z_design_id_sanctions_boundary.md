# Sanctions boundary data model — decision record for #5878

Status: decided (design only; no implementation in this record).
Parent: #5683 (residency & boundaries framework). Programme: epic #2056.
Companion records: the #5683 framework skeleton (PR #7781, §5) states the
framework rules this record builds on; #5883 supplies the audit/appeals UX
over the data defined here.

This record settles the five questions #5878 names — list sourcing,
updates, scope of interaction, grandfathering, and appeals — for the
sanctions boundary *data model*. It does not re-decide anything the
parent framework already decided. Where a question cannot be settled
without a new parent decision, §8 records it as an open question routed
to #5683 instead of silently forking a parent decision.

Parent decisions relied on (not re-decided here):

- Capabilities, never defaults: a sanctions boundary exists only where a
  server / world / account operator explicitly enables one; no
  deployment inherits a boundary it did not choose (#5683 framing rule 1).
- Basic frameworks first (#5683 framing rule 2).
- The list is data, not code; it is versioned; a decision log entry
  names the list version applied (framework skeleton §5.1).
- No interaction is covered by implication; omitted scope is not
  covered (§5.2). An omitted transition policy makes a new enforcement
  version invalid rather than silently applying a new restriction
  (§5.3). A deny on a sanctions boundary must name the appeal path
  (§5.4). No weak signal removes access a subject already has
  (framework evidence rule, §3 of the skeleton).

## D1. List sourcing

A sanctions boundary references operator-activated list data. Lists
enter the system in exactly two ways:

1. **Operator-supplied lists.** The operator (server admin, world
   owner, or account holder, matching who owns the boundary) supplies
   the list content or names the external list it is derived from.
   The reference set the parent names is the set of countries and
   organisations the United States prohibits interaction with; in
   practice an operator derives a list from a published source such
   as the US OFAC SDN and Consolidated Lists. The framework records
   the derivation; it does not assert the derivation is complete or
   legally sufficient.
2. **A shipped starting-point list.** The software may ship one list
   as a starting point, clearly labelled in the data itself as a
   starting point and not legal advice (parent §5.1). A shipped list
   is inert until an operator activates it for a boundary — shipping
   is not enabling (capabilities, never defaults).

There is no third source. The framework never fetches an external
feed on its own in this design (see §8, open question Q1), and no
list becomes active by being present on disk or in a release.

## D2. The data model

Two objects. The list is reusable across boundaries; everything
boundary-specific lives on the declaration, never on the list.

```
ListVersion {
  id,            // stable list id + monotonic version, e.g. (ofac-derived, 14)
  source,        // { kind: operator | shipped, derivation: text/URI,
                 //   derived_from: external list name + its publication date }
  effective_from,
  effective_to,  // null while current
  entries: [Entry],
  signature,     // over the canonical encoding of the fields above
}

Entry {
  kind: country | entity | person,
  // country: ISO 3166-1 alpha-2 code
  // entity / person: the source list's native entry id, plus the
  // identifiers the source publishes (names, programme, and for
  // persons date-of-birth where the source carries it)
  keys: [...],
  note,          // source-provided remark, carried verbatim
}

SanctionsBoundary (a BoundaryDeclaration, parent §2) {
  list: list id,
  version_policy: pinned(version) | follow_latest_activated,
  scope: [Interaction],          // D4 — explicit, never implied
  transition_policy: ...,        // D5 — mandatory
  appeal_path: ...,              // D6 — mandatory; a deny names it
}
```

Decisions inside the model:

- **Immutability.** A `ListVersion` is never edited in place. Any
  change — an entry appearing, disappearing, or being corrected — is
  a new version. Past enforcement decisions stay reproducible against
  the version that produced them (parent §5.1).
- **Match keys are list-native.** Entity and person entries match on
  the source list's native identifiers first. A name-only resemblance
  is not a match (D6).
- **Country entries match on evidence, not on the list's say-so about
  a person.** A country entry is evaluated against the subject's
  residency / citizenship evidence under the framework's evidence
  model (assurance levels, derived/verdict privacy classes); the list
  supplies the country set, the evidence model supplies the subject's
  relation to it.

## D3. Updates and propagation

- An update is a new `ListVersion` passing through three recorded
  states: **staged** (content present, signature unverified),
  **validated** (signature verified; schema checked; entry-count
  delta against the previous version recorded), and **activated**
  (operator action, or an operator-declared automatic-activation rule
  recorded on the boundary in advance — never an unrecorded default).
- Activation is what makes a version enforceable, at its
  `effective_from`. A boundary with `follow_latest_activated` moves
  to the new version at activation; a `pinned` boundary moves only
  when its operator re-pins. Both are recorded as new declaration
  versions, so the decision log can name the exact list version and
  declaration version behind any past decision.
- Removal of an entry is also just a new version. Nothing is deleted
  from history; `effective_to` closes the superseded version.
- Propagation to federation peers is out of this record's scope and
  is open question Q2 (§8): a peer asserts its own evidence and its
  own boundaries; this model does not push list content to peers.

## D4. Scope of interaction

Scope lives on the declaration (`SanctionsBoundary.scope`), because
the same list legitimately backs boundaries with different scopes on
different servers and worlds. The scope vocabulary is the parent's
§5.2 table plus one row the parent's enforcement section (§8) already
implies:

| Interaction | Notes |
| --- | --- |
| Account creation / signup | |
| Login to an existing account | transition behaviour is D5's, stated explicitly |
| World membership | |
| Blob storage | |
| Bridge / federation traffic | transit may differ from service; state both |
| Relay transit | transiting a jurisdiction may differ from serving it |
| Capability grants | whether a grant may be issued to, or exercised by, a matched subject |

Rules (from the parent, restated because implementations get them
wrong): the declaration must record covered *and* not-covered
interactions explicitly; an omitted interaction is not covered; an
empty overall enforcement scope is invalid.

## D5. Grandfathering and list changes

A list update alone never cuts off existing work (parent §5.3). What
it does is whatever the declaration's mandatory `transition_policy`
says. This record fixes the default an operator starts from, and the
menu; the declaration must still choose explicitly — an omitted
policy makes the new enforcement version invalid.

- **Default: `grandfather_and_flag`.** When an entry appears,
  existing accounts, world memberships, and federation links matched
  by it keep working and are flagged into the review queue (D6) with
  the list version that flagged them. New interactions in scope are
  denied from the version's `effective_from`.
- **Alternative: `reevaluate_next_protected_action`.** Existing
  relationships are re-evaluated at the subject's next in-scope
  protected action (e.g. next login, if login is in scope) instead of
  immediately; until then they are flagged the same way.
- **No third option severs existing relationships at activation.**
  Immediate severance is rejected (see Alternatives): it converts a
  data update into an unreviewed mass action against existing users,
  and the framework's evidence rule already forbids a weak signal
  from removing access a subject has.
- **Entry removed:** access denied solely because of that entry is
  restored prospectively from the new version's `effective_from`;
  exceptions recorded against the entry expire with it; the removal
  and every restoration are written to the decision log. Nothing is
  retroactively re-decided: past denies stand as correct under the
  version that produced them.

## D6. False positives, exceptions, and appeals

False positives are expected traffic, not edge cases (parent §5.4):
name collisions, dual citizens, residents vs citizens, travellers
temporarily present. The data side of the recourse path:

- **Match strength gates the outcome.** A match on list-native
  identifiers may deny (subject to D5). A name-only or otherwise
  weak match never denies and never removes existing access; it
  creates a review-queue item. This is the framework's no-weak-signal
  rule applied to sanctions data.
- **Exception records.** A subject cleared on review is recorded as
  an exception: a new declaration version carrying the exception, the
  evidence relied on, the reviewing actor, the scope it covers, and
  an expiry. Exceptions are data with a lifetime, not a silent flag
  on the subject; an expired exception returns the subject to the
  ordinary rule.
- **Review queue.** Flagged subjects (D5) and weak matches (above)
  land in one review queue whose items name: the subject, the list
  version and entry matched, the match basis (which keys hit), the
  boundary and declaration version, and the current effect (flagged /
  denied / excepted). The #5883 design supplies the audit and appeals
  UX over exactly these fields; this record fixes the fields, not
  the screens.
- **Every deny names the appeal path** in the user-facing message
  (parent §5.4): which boundary denied, under which list version, and
  how to request review. A deny that cannot name its appeal path is
  not a valid enforcement outcome.

## Invariants implementation must hold

1. No sanctions restriction exists without an explicit operator
   declaration; shipped or staged list data enforces nothing.
2. Enforcement decisions are reproducible: the decision log names
   list id + version, declaration version, scope row, and match basis
   for every deny, flag, and exception.
3. List versions are immutable and signed; activation is a recorded
   act (by an operator, or by an operator's pre-recorded rule).
4. Omitted scope is not covered; omitted transition policy or appeal
   path makes an enforcement version invalid, never silently defaulted
   at enforcement time (the D5 default is a drafting default the
   operator confirms, not a runtime fallback).
5. A weak (name-only) match never denies and never removes existing
   access; it only enqueues review.
6. A list update never, by itself, severs an existing account, world
   membership, or federation link.
7. Every deny names its appeal path; every exception carries
   evidence, actor, scope, and expiry.

## Alternatives rejected

- **Hard-coded country / entity lists in code.** Rejected by the
  parent outright: the list is data (§5.1); code-embedded lists
  cannot carry effective dates, provenance, or per-operator choice.
- **A runtime third-party sanctions API as the source of truth.**
  Enforcement would depend on a vendor's availability, match logic,
  and versioning the decision log cannot reproduce. Operators may
  derive `ListVersion`s from such a service at staging time (D1/D3);
  the runtime never calls it.
- **Fuzzy name matching as a deny basis.** Name collisions are the
  known false-positive factory (§5.4); fuzzy hits are review input,
  never a deny.
- **Immediate severance of existing relationships on list addition.**
  See D5: an unreviewed mass action triggered by a data update, and
  indefensible for the dual-citizen / traveller cases the parent
  names.
- **Scope carried on the list instead of the declaration.** The same
  list backs different boundaries with different scopes; putting
  scope on the list forces list duplication per scope and makes the
  reusable-data property (D2) false.

## 8. Open questions routed to #5683

These need a parent-level decision; this record does not fork one.

- **Q1 — Automated feed refresh.** The parent skeleton defers
  external feed fetching to a later step (its §11). Whether
  activated lists may be refreshed from a feed on a cadence — and
  who is responsible for a stale list's legal exposure — is a
  framework decision for #5683. Until then D1/D3 stand: staging is
  an operator act.
- **Q2 — List distribution over federation.** Whether a server may
  distribute its activated `ListVersion` to peers (so a peer can
  reproduce a transit decision) touches the parent's bridge /
  federation evidence-assertion model and is #5683's to settle.
- **Q3 — Shipped starter list contents.** Which external list the
  shipped starting point derives from, and its legal review, is a
  product decision above this data model; D1 fixes only that it is
  labelled, inert until activated, and versioned like any other list.

## Next actions

- Implementation of the `ListVersion` store, declaration evaluation
  over the D4 scope rows, and the D6 review-queue data is filed as
  issue #8201 alongside this record; the parent framework's
  implementation issues (#7782–#7787) remain the home for the shared
  declaration / evidence machinery this model plugs into.
- #5883 (audit & appeals) consumes the D6 field set; if its UX work
  needs a field this record does not name, that is a change to this
  record, made as a new section appended while the work is active.
