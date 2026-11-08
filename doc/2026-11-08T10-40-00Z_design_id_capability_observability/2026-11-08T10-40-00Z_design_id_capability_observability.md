# Design: capability observability — who can see which capability facts, where

- Date: 2026-11-08
- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Issue: [#6017](https://github.com/developing-today/code/issues/6017), the capability-observability child of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955) (wave-2 follow-up, prior-art Lane B item 50).
- Specified against: the capability-model registry schema of [#5958](https://github.com/developing-today/code/issues/5958) (`doc/2026-11-08T09-15-00Z_design_id_capability_model/`, delivered in [PR #7599](https://github.com/developing-today/code/pull/7599)). Field names from that record are **consumed verbatim, never re-defined** (§2).
- Presentation sibling: the permission-UX record of [#5960](https://github.com/developing-today/code/issues/5960) (`doc/2026-11-08T09-06-08Z_design_id_permission_ux/`, merged in PR #7594), which owns the pre-connect disclosure surface several views below render into.

## Decision in brief

Capability facts already exist as registry data under #5958's schema: a
**release record** (what the code asks for — `requested`; what analysis proved
it uses — `discovered_used`, with a certification state) joined to a
**world-instance record** (what this world may use — `effective_grant`; what it
has been seen using — `observed`). Observability is the discipline of showing
each audience exactly the join it is entitled to, in the vocabulary #5960
settled, with one rule above all others:

**Unknown is rendered as unknown, never as empty.** `discovered_used.caps` is
absent unless `discovered_used.state == "certified"`; every view in this
record inherits that rule. A surface that shows "uses nothing" for an
uncertified world is lying, and this design treats it as a defect in the
surface, not a data gap.

Three audiences, three different joins of the same two records:

- **Users** deciding whether to connect see the *instance join for one world*:
  the granted set, the derived used set, and what changed since their last
  connection — rendered into #5960's pre-connect disclosure and world page.
- **World authors** see the *release join for their own releases*: requested
  vs used, the footprint they inherit from their libraries, and honest
  markings where a derivation is uncertified.
- **Server admins** see the *instance join across their server*: grants in
  force, a per-world capability inventory, and moderation-pack provenance.

History is a registry fact, not a view trick: §5 adds the append-only
capability event log that #5958 §4.3 explicitly deferred to this record, so
"what changed since …" is a query over recorded diffs, not a reconstruction
from two current-state snapshots that may both have moved.

## 1. Terms (consumed, not re-defined)

- **Release record / instance record**, `requested`, `discovered_used`
  (with `state` ∈ `certified | not_certified | stale | unknown`, `reason`,
  `partial`, `provenance`), `effective_grant` (`tiers`, `caps`, `ceiling`,
  `computed_at`), `observed` (`window`, `caps`, `runs`), `custom_platform`:
  exactly as defined in #5958 §4. This record adds no field to either record.
- **Requested set / used set**, **client-side vs server-side capability**,
  **load-bearing capability**, **grant mode**: as defined in #5960. The
  actor phrase ("using your client" / "through the server side") is part of
  every user-facing line, per #5960 §1.1.
- **View**: a named, audience-scoped projection of the two records (plus the
  §5 log). A view is a *reading* of registry facts; it stores nothing of its
  own except per-user reading state (e.g. a user's last-connection marker,
  §3.1), which is client state, not registry state.
- **Uncertified derivation**: a release whose `discovered_used.state` is not
  `certified`, or whose only usage data is `partial` (labelled
  `non_authoritative: true` in the schema). The exception *policy* for such
  worlds is #6000's (PR #7025); this record specifies only how the state is
  **marked** on surfaces, never what is done about it.

## 2. The audience × surface matrix

| Audience | Surface | View (§) | Records read |
|---|---|---|---|
| User (connecting) | Pre-connect disclosure (#5960 §1) | Granted & used, two groups (§3.1) | instance `effective_grant` + release `requested` / `discovered_used` |
| User (connecting) | Pre-connect disclosure, delta form (#5960 §1.2) | Changed since last connection (§3.1) | §5 log, from the user's last-connection marker |
| User (connected) | World page | Grant state & observed use (§3.1) | instance `effective_grant` + `observed` |
| World author | Registry listing for own releases | Requested vs used (§3.2) | release `requested` / `discovered_used` |
| World author | Release detail | Inherited library footprint (§3.2) | release identity closure (#5958 §4.1) + the same two sets per dependency release |
| World author | Registry listing + release detail | Uncertified-derivation marking (§3.2) | `discovered_used.state` / `partial` |
| Server admin | Admin console, per world | Grants in force (§3.3) | instance `effective_grant` (+ `custom_platform` from the release) |
| Server admin | Admin console, per server | Capability inventory (§3.3) | all instance records on the server, joined to releases |
| Server admin | Admin console, per world | Moderation-pack provenance (§3.3) | moderation provenance tuple (§3.3) |
| Any of the above | Registry history query | Diffs over time (§5) | §5 log |

Surfaces not in this matrix do not show capability facts. In particular,
chat, presence, and world-discovery listings show no capability data beyond
the markings §3.2 requires on a listing an author or admin already entitled
to see — observability is not advertising.

## 3. The views

### 3.1 Users

**Granted & used (pre-connect).** The disclosure's two groups (#5960 §1.1)
are populated from the instance join, not from the release alone:

- The **granted set** a user is asked about is `effective_grant.caps` for
  this world instance — what the world may actually exercise here — shown
  split by the client-side / server-side grouping of #5960. `requested.caps`
  is the annotation source: a granted capability the release never requested
  cannot exist by construction of the grant computation, and a requested
  capability the server did not grant is shown struck-through as "asks for
  this, not granted on this server" when it is load-bearing (its denial
  semantics are #5960 §4's), and omitted when optional.
- The **used set** line is `discovered_used`, rendered state-first: the
  certification `state` and `provenance.at` are shown with the set
  ("analysis proved this subset on 2026-11-08"), and when
  `state != "certified"` the line reads "usage not certified — what this
  world actually uses is not known", with `reason` in plain language. It
  never renders as an empty list.
- `observed.caps` appears only on the **world page after connection**, as a
  clearly separate, weaker line: "seen using in the last 24h" (with
  `observed.runs`). It is journaled observation (#5958 §4.3), never merged
  into the derived used set and never shown pre-connect, where it would
  masquerade as certification.

**Changed since last connection.** The delta disclosure of #5960 §1.2 is
populated from the §5 log: the diff of `requested.caps`,
`discovered_used.caps` (when certified at both ends), and
`effective_grant.caps` between the user's last-connection marker and now.
A capability that became uncertified in the interval (`state` moved to
`stale` because `release.analyzer` changed, per #5958 §4.2) is a delta line
of its own — "usage certification lapsed" — because the honest content of
the set changed even though no capability did.

### 3.2 World authors

**Requested vs used.** For each of the author's releases: `requested.caps`
beside `discovered_used.caps`, with the two diffs named the way #5960 §1.1
names them — requested-but-unused ("asks for this, has not used it":
over-request, visible, worth trimming before the next release) and
used-but-unrequested (a defect: shown highlighted, and reported through the
capability-change channel owned by #5959, exactly as the user surface
treats it). Tier membership (`requested.tiers` vs the vocabulary of #5958
§3) is shown alongside, since a tier the server has not enabled explains an
empty intersection better than the raw diff does.

**Inherited library footprint.** A release identity covers the transitive
dependency closure (#5958 §4.1: platform tree hash + source closure hash +
DLL closure hash). The release detail view decomposes the requested and
used sets **by closure member**: capabilities attributable to the world's
own code vs capabilities pulled in by each library release in the closure,
each library entry itself a release record with its own two sets. An author
can therefore see "my world asks for `chat.say` because library X's release
asks for it" without any new schema: the closure is already the identity;
this view is the join rendered per member.

**Uncertified-derivation markings.** Wherever an author's (or admin's)
listing shows a release, `discovered_used.state != "certified"` renders as
a visible marking naming the state and `reason` — "not certified",
"certification stale (analyzer changed)", "unknown" — and a `partial` set,
when present, is rendered only under its schema label
(`non_authoritative: true`), visually distinct from a certified set. The
marking is presentation only: whether an uncertified world may run, and
under what exceptions, is #6000's policy (PR #7025), consumed here as a
boundary, not re-decided.

### 3.3 Server admins

**Grants in force.** Per world instance: `effective_grant` in full —
`tiers`, `caps`, `ceiling`, and `computed_at`, so an admin can see both
what a world may do and the ceiling it was computed against, and when the
computation last ran. A release on a synthesised platform shows
`custom_platform` (`tree_hash`, `requested_key`, `compiled_at`) beside the
grant, because the grant's meaning depends on it (#5958 §5).

**Capability inventory.** Per server: every instance record joined to its
release, grouped by capability — for each cap name (vocabulary: #5958 §3),
the worlds whose `effective_grant.caps` include it, and separately the
worlds whose certified `discovered_used.caps` include it. The two lists
differing is the server-level form of over-request, and the inventory is
where an admin notices a capability nobody uses but everybody holds before
tightening a tier.

**Moderation-pack provenance.** Per world: which moderation pack applies,
in the tuple **(pack id, pack version, feed(s) the pack was taken from,
applied_at)**. The moderation follow-ups under #5955 own the pack mechanism
and therefore own these fields; this record specifies only that the admin
surface must render the full tuple or, when no provenance has been
recorded, the explicit line "no pack provenance recorded" — never a blank,
which would be indistinguishable from "no pack applies". (Dependency
named, per §6: until the moderation child lands its record, this view has
a specified shape and no data source.)

## 4. Rendering rules (all audiences)

1. **State first.** Any surface showing a used set shows
   `discovered_used.state` before, and more prominently than, the set
   itself (#5958 §4.2's rule, applied to presentation).
2. **Unknown ≠ empty.** `null` with a state and reason is "we do not
   know"; `[]` is a certified proof of no platform calls. The two never
   share a rendering.
3. **Two groups, never merged.** User surfaces keep client-side and
   server-side lines in separate headed groups with the actor phrase in
   the sentence (#5960 §1.1). Author and admin surfaces are exempt from
   the grouping (they read registry vocabulary directly) but not from
   rules 1–2.
4. **No record, no invention.** A view renders only fields that exist in
   the two records or the §5 log. A missing field is rendered as its
   absence ("not recorded"), never defaulted into a plausible value.
5. **Provenance travels.** Every used-set rendering carries
   `discovered_used.provenance` (method, analyzer version, timestamp) one
   interaction away at most; a set without visible provenance reads as
   assertion, and assertion is what certification exists to replace.

## 5. History: capability diffs are registry facts

#5958 §4.3 deferred this explicitly: the registry entry is the *current*
fact for a release identity; history is this record's addition.

**The capability event log** is an append-only sequence of events, each
keyed by `(world_id, release_identity)` — either key may be the query
axis — carrying:

- `at`: event timestamp;
- `kind`: one of `requested.changed`, `discovered_used.certified`,
  `discovered_used.staled`, `discovered_used.invalidated`,
  `effective_grant.changed`, `custom_platform.compiled`;
- the **field-level diff**: for set-valued fields (`requested.caps`,
  `discovered_used.caps`, `effective_grant.caps`, `effective_grant.tiers`,
  `effective_grant.ceiling`), the added and removed cap/tier names; for
  `discovered_used`, the `state` transition and `provenance` of the new
  analysis.

Events are written by whoever writes the corresponding record field, in
the same action — a record change without its event is a defect of the
writer, exactly as a label change without its state change is in the
fleet's own protocol. The current-state records remain the authority for
"now"; the log is the authority for "what changed, when".

**Queries the log must answer** (these are the acceptance shape, not an
API sketch):

- *Diff between two points* for one world or one release: the folded
  added/removed sets per field — this is what §3.1's changed-since-last-
  connection view consumes, with the user's last-connection marker as the
  earlier point.
- *Diff since a release*: everything after a given `release_identity`
  first appeared — the author's "what has moved since I published".
- *Server-wide capability timeline*: every event naming a given cap —
  the admin's "when did worlds here start holding `fs.*`".

`observed` is deliberately **not** in the log: it is a rolling window
(#5958 §4.2.1), and journaling it would drown the certification and grant
events — the facts a reader can act on — in runtime noise. Retention of
the log is a server-local storage decision; pruning, when a server
chooses it, is by whole events older than a stated horizon, never by
rewriting diffs, and a pruned range renders as "history before <date>
not retained" under rule 4 of §4.

## 6. Sibling coordination and dependencies

| Sibling | State at writing | What this record takes from it | What it gives back |
|---|---|---|---|
| #5958 capability model | Design record delivered in PR #7599, **open at writing** | The registry schema (§4), vocabulary, certification states — consumed verbatim | The history layer its §4.3 deferred here (§5) |
| #5960 permission UX | Merged (PR #7594) | Disclosure shape, two-group rule, requested/used vocabulary, denial semantics | The concrete record population for its disclosure and delta surfaces (§3.1) |
| #5959 change notices | Open | — (boundary: the change-notice *shape* is its own surface) | The §5 diffs its notices announce; used-but-unrequested reports route through its channel (§3.2) |
| #5961 precedent research | Open | No dependency: no view here waits on it | — |
| #6000 / PR #7025 exception policy | PR open | The uncertified-derivation boundary: markings here, policy there | The marking surfaces its policy outcomes will display on (§3.2) |
| #5840 knowledge-layer sharing | Open, different layer | Shared vocabulary only (#5960 §0) | — |
| Moderation follow-ups (#5955) | Not yet filed as records at writing | **Dependency**: the provenance fields of §3.3 are theirs to define | The specified admin view those fields will populate |

The one hard dependency is #5958: if PR #7599's schema changes in review,
this record's field references must change with it — they are references,
not copies, so a schema rename is a rename here, not a divergence. No
other sibling's unfinished work is silently assumed anywhere above; each
is either consumed as landed or named in this table.

## 7. What this record does not do

- No field is added to, removed from, or renamed in #5958's two records;
  the §5 log is an addition beside them, keyed to them.
- No implementation: no view, surface, or log writer is built here.
- No budget observability (the server-invariants child of #5955 owns it),
  no exception policy (#6000), no change-notice design (#5959).
- No split is required: the views and the history layer are one record
  because every view in §3 is defined by how it reads the same two
  records and the one log. If implementation later shows the log needs
  its own record, that split is filed from #6017 at that time.

## Revision history

- 2026-11-08: Initial design record for #6017 (queue task 13008, lane
  code-m): audience×surface matrix, per-audience views specified against
  the #5958 registry schema, rendering rules, the §5 history layer
  deferred here by #5958 §4.3, sibling coordination with the #5958 /
  PR #7599 dependency named.
