# Decision: sensitivity classification model for ID table queries (#5817)

Status: decided (this record). Parent design: #5675 (querying ID tables,
local and remote). Sibling designs: #5816 (query language/shape — the
foundational pair with this record), #5818 (filtered remote execution,
composes both), #5819 (owner override mechanics — depends on the levels
defined here). Programme: epic #2056.

This record settles only what #5817 owns: **sensitivity levels,
assignment, defaults, and field-level vs row-level behaviour**. It does
not re-decide the parent's fixed points — server-side filtering under
the requester's clearance, the owner override (own server, unencrypted
data only; encryption is a cryptographic boundary policy never
outranks), surface parity, and cursor pagination with truncation
honesty. Where an answer here touches one of those, it composes with
it and says so.

## 1. Levels: a fixed, ordered set of four

| Level | Name         | Meaning                                                          |
| ----- | ------------ | ---------------------------------------------------------------- |
| S0    | Public       | Intended to be world-readable.                                   |
| S1    | Internal     | For the owner's people/agents; not for strangers.                |
| S2    | Confidential | Named-principal material; the default (see §3).                  |
| S3    | Restricted   | Local-only in v1; never leaves the server via remote query (§5). |

The levels are a **total order**: a clearance is a single level, and a
datum is visible to a requester iff `classification <= clearance`.
One comparison, no label algebra — a remote server must be able to
evaluate the filter without knowing anything about the owner's local
policy beyond the level integers.

**Owners cannot define custom levels in v1.** Remote filtering is
executed by servers the owner does not operate; a label only filters
correctly if every server interprets it identically. The extension
path, when a real need is demonstrated, is namespaced local labels
that map onto the four levels at the server boundary — the mapping is
the owner's, the wire only ever carries S0–S3. (Rejected alternative:
owner-defined levels — they silently fork the filtering semantics per
server pair, the exact failure the parent files this ticket to avoid.)

## 2. Assignment: table default, raise-only refinement

- Every table/collection carries a classification, set by the data
  owner (the principal who owns the table). Only the owner changes
  it; changes are audit-logged on the serving server (parent §2).
- A **row** may carry a classification that **raises** it above the
  table level. A **field** may carry one that raises it above the
  row/table level. Refinement only ever raises.
- The effective classification of a value is
  `max(table, row, field)` over the levels that apply to it.
- A non-owner who holds a copy (e.g. a peer a table was shared to)
  may raise the classification of their copy, never lower it: a
  recipient's caution binds their copy; a recipient's laxness must
  not unclassify the owner's data. This mirrors the residency
  model's rule that scope is assigned locally and a peer cannot
  remove it (`id-core` `residency::Scope`).

## 3. Default: unclassified data is S2 Confidential

The S2 default applies at the root of the classification ancestry
only: a **table** with no explicit classification is **S2**. A row
or field with no explicit classification carries no level of its
own - it inherits the nearest classified ancestor's level, so in
section 2's formula only the levels that actually apply contribute
to the `max`. An owner who classifies a table S0 or S1 can publish
it by that act alone; labelling every row and field is never
required to lower an inherited level, because inheritance is not a
refinement. The parent requires the default to be safe, not
permissive; S2 is the lowest level that still requires a
named-principal clearance.

Rejected defaults: S1 (permissive — an unclassified table would be
readable by any internal-clearance identity, so a forgotten label
leaks); S3 (so strict it would make ordinary local use fail closed
everywhere and train owners to live in the override, eroding the
override's audit value).

## 4. Clearance, and composition with grants

A requester's clearance is the maximum level the data owner has
granted their identity, carried as an attribute of the
selective-sharing grant for the owner's domain. Grants and
classification compose **deny-wins** (answering the parent's open
question for this record's scope): the grant decides whether the
peer may reach the table at all; classification filtering then
decides what the query returns; satisfying one never widens the
other. An owner may set an assurance floor for issuing a clearance
at S2 or above, expressed in the `id-core` assurance vocabulary
(`residency::AssuranceLevel`: Self / OrgAttested / ProviderVerified);
v1 does not mandate a floor.

## 5. Field-level vs row-level behaviour

Filtering is server-side, before anything leaves the server
(parent §2). Per query, in order:

1. **Row filter.** A row whose effective row classification exceeds
   the requester's clearance is excluded whole.
2. **Field filter.** For a field whose classification exceeds the
   clearance, behaviour depends on how the query names it:
   - **Explicitly projected** → the query is **rejected** with a
     machine-readable error naming the field and its level. An
     explicit ask never silently returns a different shape than was
     requested (the parent's truncation-honesty rule, applied to
     projections).
   - **Reached via `*` / implicit projection** → the field is
     omitted from the result shape, and the result carries a
     `withheld_fields` notice: field names and levels, never values.
     Exception (section 6): an S3 field is never named in the notice -
     it is omitted entirely, because naming it would be an existence
     signal.
3. **Aggregates** are evaluated over the **authorized visible set**
   in v1 - the rows and fields the requester is cleared to see -
   never over withheld data, and never rejected because withheld
   data exists. A count or sum computed over withheld data would
   leak its magnitude, and a rejection conditioned on hidden data's
   presence would leak its existence (for S3, section 6 forbids any
   existence signal: the same aggregate query behaves identically
   whether or not an S3 row matches). The result describes the
   visible set; withholding is signalled only per section 6
   (S0/S1 count, S2 boolean, S3 nothing).
4. **S3 is local-only in v1.** A remote query never returns S3 data,
   even to an S3-cleared requester; S3 is reachable by local query
   and by the owner-override path (#5819), not by remote execution.

## 6. Withheld-result honesty, per level

The parent requires a per-level pick; silent filtering and existence
leaks are both failure modes.

| Withheld level | What the requester learns                                                                                                                                           |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| S0/S1          | The count of withheld rows (`withheld_count`).                                                                                                                      |
| S2             | That withholding happened (a boolean) and which fields were withheld (names + levels, §5) — **not** row counts; volume of confidential data is itself confidential. |
| S3             | Nothing: no count, no existence signal in the result. The withholding is recorded in the serving server's audit log, visible to its owner (parent §2 audit).        |

## 7. Encrypted data

Classification is a policy filter over data the serving server can
read. For encrypted tables the server cannot filter content, so:
classification of encrypted objects lives on the **envelope** at
table and row level only (there is no field level inside
ciphertext), and remote filtering applies to those envelope labels.
The parent's boundary is an invariant here, not a decision:
encryption defeats the owner override, and no classification rule
in this record changes that.

## 8. Relation to the id-core vocabulary

Sensitivity classification is an axis of its own — it classifies
**data** against a **requester's clearance**. It is not
`residency::PrivacyClass` (which governs how much of an evidence
claim may be revealed downstream) and not `AssuranceLevel` (how
strongly an identity claim was established); a field can be S2
while the evidence about it is `VerdictOnly`. Implementations must
not merge the axes; §4's assurance floor is the one sanctioned
touch point.

## Invariants implementation must hold

1. Effective classification is `max(table, row, field)`; refinement
   is raise-only, and a non-owner can never lower a classification.
2. Unclassified means S2, everywhere, with no per-surface exception.
3. The wire carries only S0–S3; custom labels, if ever added, map
   onto them at the server boundary.
4. Explicit projection of an above-clearance field rejects the
   query; implicit projection omits the field plus a
   `withheld_fields` notice.
5. Aggregates never span withheld data in v1.
6. S3 never crosses a remote query, and its withholding leaves no
   signal in the result — only in the owner's audit log.
7. Withheld signalling follows §6 exactly per level.
8. Classification never widens a grant, and a grant never widens a
   classification (deny-wins).

## Alternatives rejected (summary)

- **Boolean public/private** — cannot express the internal vs
  restricted distinction the parent's honesty rules need.
- **Per-object labels with no table default** — the assignment
  burden guarantees most data is unclassified, so the default does
  all the work with none of the review.
- **Client-side filtering** — forbidden by the parent; results must
  be filtered before anything leaves the server.
- **Counts at every level** — an S3 existence/volume leak.
- **Silent filtering at every level** — named a failure mode by the
  parent; makes filtered results indistinguishable from complete
  ones.
