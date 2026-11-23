# PRI-03 — Required before/after peer ordering within equal effective urgency

Issue: [#9511](https://github.com/developing-today/code/issues/9511)
(slice `PRI-03` of [#9199](https://github.com/developing-today/code/issues/9199),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the peer-ordering slice of the merged
design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the rank, rebalance, and comparator-parity
contract a separately scoped implementation issue can be filed
against. The package's component #9199 states the candidate
order and sketches the rank semantics; this record *decides*
the points that sketch left open — representation, stride,
bounds, epoch, refusal vocabulary, and the parity rule — and it
authorizes no code, schema, migration, deployment, or live
acceptance; those remain separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Component #9199, "Candidate order and rank semantics"** (the
  PRI-03 slice row and its V2 contract): eligibility is a gate,
  not a score; the comparator descends effective urgency, then
  explicit peer rank, then creation time, then canonical
  identity; rank is an exact signed integer, lossless across
  Rust/JSON/TypeScript, never a JS float; legacy `NULL` rank
  normalizes to the common rank `0` on read; a before/after
  command carries the group, the authority-qualified neighbor
  identities, neighbor work versions, and `rank_epoch`; moves
  across unequal effective urgency require explicit
  confirmation. This record fixes the constants and algorithms
  that text defers ("a fixed stride", "a deterministic bounded
  local range") and changes none of its semantics.
- **FDR-PMO-002** (versioned priority profiles, ordering, and
  provenance; accepted as a local design contract 2026-11-13):
  option 1 — canonical intrinsic values, derived effective
  pressure, explicit tie rank — is the shape this slice fills
  in. The protected-owner policy in the package's
  `architecture.md` is normative over every rank mutation
  (§4.3); #7030's ratified `expedite=15` / `next-up=25` bands
  are consumed, never re-ratified or renumbered.
- **FDR-PMO-008** (bounded exploration, fairness, calibration):
  selection fairness consumes the total order this slice
  defines; it does not amend it. A deterministic order is the
  substrate the fairness state perturbs *by policy*, never by
  comparator drift between surfaces.
- **The package `architecture.md` digest contract**: the
  selection-order fingerprint is
  `digest("fleet.selection-order/v1", order_object)` with the
  framing `SHA256(ASCII(domain) || 0x00 || UTF8(JCS(dto)))`,
  rendered `sha256:` + lower hex, over the **entire** eligible
  population in canonical comparator order — never a display
  page. §3.4 states what this slice contributes to that object.
  The digest is a fingerprint, not authority, not proof of
  complete inputs, and not a claim grant.
- **CLI-01** ([#9451](https://github.com/developing-today/code/issues/9451),
  CLOSED) and **CLI-02** ([#9438](https://github.com/developing-today/code/issues/9438),
  design record PR #13255): rank mutations ride the shared
  `fleet.command/v1` envelope and its `Command`/`Result` types
  (`contracts.rs`); the allocated command keys this slice's
  semantics serve are `flag.order`, `flag.priority-before`,
  `flag.priority-after`, and `priority.move` (census
  command-slice map). Spelling and grammar are CLI-02's;
  registration is CLI-13's. This record fixes what an accepted
  command *does*, not how it is spelled.
- **CEN-04** ([#9434](https://github.com/developing-today/code/issues/9434),
  design record PR #13341) **§5, the field-admission gate**: a
  persisted field with no matrix row may not be written. This
  slice persists exactly two new facts — the peer rank and the
  per-group rank epoch — so §7 supplies their admission entries
  in §5's shape, and §9 makes the matrix amendment a hard
  predecessor of the first rank write. CEN-04 §3.3 (urgency and
  profile) is the adjacent row: `priority` is urgency, and §7
  records the standing refusal that it is not rank.
- **PRI-01 / PRI-02** ([#9479](https://github.com/developing-today/code/issues/9479)
  is PRI-02): the profile contract, the dimension types
  (intrinsic urgency, nullable importance, effective pressure),
  and the explanation shape are theirs. This record consumes
  *effective urgency* as an input computed under a pinned
  policy snapshot; it defines no urgency semantics, no aging,
  and no pressure derivation (PRI-04/PRI-05 territory).

Verified state at writing (2026-11-23, origin/main `1310027fd`):

- No dated PRI-03 record exists under `doc/`; the only PRI-03
  text on main is the #9225 package (component slice row,
  census, delivery plan, preserved issue bodies). This record
  is the first.
- `pkgs/fleet/crates/fleet-queue/src/work/priority.rs` **does
  not exist**; the `work/` module has no priority submodule
  yet. PRI-01/PRI-02 create that file's foundations first —
  the shared-file sequencing edge in §9.
- The broker's claim and list orders **still diverge** on
  current main, exactly as the component's V1 baseline
  recorded at the census pin: the claim path
  (`turso_queue/lifecycle.rs`) orders
  `(priority IS NULL), priority ASC, created_at ASC` with **no
  final identity tie-break**, while the admin/list path
  (`turso_queue/admin.rs`) orders the same keys **plus
  `id ASC`**. §3.3 makes repairing this divergence — not
  merely tolerating it — part of this slice's parity contract.
- The `tasks` schema of record (the `SCHEMA` const in
  `turso_queue/core.rs`) carries `priority INTEGER` and an
  index on `(queue, status, priority, created_at)`; **no rank
  column and no epoch store exists**. The two facts of §7 are
  genuinely absent, not hidden under another name (CEN-04 §4's
  standard applied).
- Gate state for this slice: PR #9225 MERGED 2026-11-13;
  #9074 CLOSED (the slice row's "persistent rank writes gated
  on QTI/#9074 schema disposition" is discharged — #9074's
  store exists and its DDL is of record, per CEN-04);
  QTI disposition #9241 CLOSED with the bounded verdict CEN-04
  §6 states (QTI contributes no schema; if material surfaces
  later it re-opens only rows it overlaps — for this slice,
  the §7 rows). #9199 remains OPEN as the design parent and
  tracker, per the sibling pattern; it gates the
  implementation issue (§9), not this record.

## 1. Intent

Within one effective-urgency group, the broker today has no
way to say "this peer goes before that one" except by editing
urgency — which lies about urgency, breaks the ratified band
meanings, and cannot express a relative order anyway when two
items must stay equals. The source requirements are explicit
that before/after peer operations are a first-class input
(component #9199: "do not omit them"), and that the resulting
order must be *the same order on every surface that claims
it*: the broker's claim path, its list path, and the
cross-target selection-order fingerprint.

PRI-03 supplies that missing layer: an explicit, exact,
auditable peer rank inside the group; a move operation whose
preconditions make stale intent fail loudly instead of
silently landing in the wrong place; a rebalance that is
local, bounded, deterministic, and atomic, so rank arithmetic
never becomes a global renumber or a partial write; and a
single comparator definition that the SQL orderings must match
byte-for-byte in outcome, proven by one golden fixture run
against both.

## 2. Rank representation

**Decision — type.** A peer rank is a signed 64-bit integer,
`PeerRank(i64)`, defined in `work/priority.rs`. Signed, not
unsigned: the common rank is `0` and a before-first insertion
must be able to go below the current minimum without a
renumber. Sixty-four bits, not thirty-two: with the stride of
§2.1, the space admits ~2^53 stride steps per group before
endpoint pressure is even theoretically reachable, and the
rebalance of §5 — not the integer width — is the operative
bound.

**Decision — scope.** A rank is meaningful **only inside its
group**: the pair `(queue, effective_urgency)` where
`effective_urgency` is the value computed for the item under
the pinned policy snapshot the ordering was taken from. The
comparator (§3) reaches rank only after effective urgency has
compared equal, so a rank is never compared across groups and
no global rank space exists. The untriaged value (null
effective urgency) forms its own group per queue, with the
group key spelled `untriaged` wherever a string key is needed
(§5.3); it is a real group — before/after works there too —
not an error state.

**Decision — wire and storage encoding.** On the wire a rank
is a **canonical signed decimal string** (`-?[0-9]+`, no
leading zeros except `0` itself, no plus sign), per the
package architecture's rule that potentially unsafe
integer-like values travel as decimal strings; it is never a
JSON number and is never coerced through a JavaScript number
on any surface. In the broker store it is a nullable SQLite
`INTEGER` column (§7): `NULL` means *never explicitly ranked*
and **normalizes to `0` on read**; every write stores an
explicit value. No reader ever observes SQL `NULL` ordering —
normalization happens in the comparator's input, in Rust and
in the SQL expression alike (§3.3).

### 2.1 Stride and gap allocation

**Decision — stride.** Fresh ranks are spaced by the fixed
stride `S = 1024`. A stride is a spacing convention, not a
meaning: the distance between two ranks carries no semantics
beyond order, and no consumer may interpret gaps as counts,
scores, or headroom promises.

**Decision — allocation.** Given the group's current ordered
membership and a target insertion point, the allocated rank
is exactly one of:

- *Between neighbors* `a < b` (the ranks bounding the gap):
  if `b - a >= 2`, the midpoint `a + (b - a) / 2` (integer
  division, rounding toward `a`). If `b - a < 2`, no integer
  exists in the gap: allocation does not squeeze — it
  escalates to the rebalance of §5.
- *Before the first member* with rank `f`: `f - S`, unless
  that would underflow `i64`, in which case §5.
- *After the last member* with rank `l`: `l + S`, unless that
  would overflow `i64`, in which case §5.
- *Into an empty group*: `0`.

Repeated midpoint insertion halves the gap each time, so a
group sustains ~10 same-gap insertions per stride interval
before a rebalance — the intended trade: rebalances are rare,
small, and local (§5), and no insertion ever fails merely
because a neighbor was placed by midpoint before it.

## 3. The comparator and parity

### 3.1 The reference comparator

**Decision.** There is exactly one comparator definition: a
pure function in `work/priority.rs`,
`compare_peer(a, b) -> Ordering`, over two already-eligible
candidates carrying their pinned ordering inputs. Eligibility
(capabilities, both matching predicates, hard blockers,
`manual_only`, lease/concurrency) is a **gate applied before
comparison**, never a comparator key. The tuple, in order:

1. **Effective urgency, ascending** (lower sooner). Where the
   pinned profile has no derivation, intrinsic urgency stands
   in, per component #9199. A null (untriaged) value sorts
   **after** every present value — the current SQL's
   `(priority IS NULL)` leg, preserved as decided behavior,
   not an accident of one query.
2. **Peer rank, ascending**, after the §2 normalization
   (`NULL` → `0`). Reached only inside a group, by
   construction of step 1.
3. **Creation time, ascending** — the authority's canonical
   UTC string, compared as issued, never parsed and
   re-serialized into a different spelling.
4. **Authority identity**: `authority_id`, bytewise
   ascending.
5. **Work identity, numeric**: `work_id` compares by
   arbitrary-precision numeric value — digit count first,
   then digits — **not** lexical string order and never
   floating point. The golden tie is `9` before `10`. Wire
   work IDs are canonical unsigned decimal strings; an ID
   that is not canonical decimal is not a work ID for this
   step (opaque provider/session IDs are different facts and
   never enter this comparator).

The comparator is total: two distinct eligible candidates in
one snapshot never compare equal, because step 5 is a
tie-break over unique identities. Same snapshot, any input
permutation → same output order; this is the slice's first
acceptance property (§8).

### 3.2 Tuple version

**Decision.** This tuple is **peer-order tuple version 1**.
The version is carried in the policy snapshot that ordering
consumers pin (alongside the profile and policy versions
PRI-01/PRI-02 own), and the `comparator_ref` of the
selection-order object (§3.4) names the PRI-CMP-01 contract
key under which this tuple is registered in the program
index. Any future change to the tuple — a key added, a
direction flipped, the work-ID rule altered — is a new tuple
version, never an in-place edit: snapshots taken under
version 1 remain interpretable, and two surfaces may claim
parity only when their tuple versions match.

### 3.3 SQL parity in `turso_queue.rs`

**Decision.** The claim path and the list path in
`turso_queue.rs` must both implement §3.1's tuple, and the
claim path gains the final identity tie-break it lacks
today. Concretely, for the broker store — whose local `id`
is the integer identity of the row — both orderings become:

```sql
ORDER BY (priority IS NULL), priority ASC,
         COALESCE(peer_rank, 0) ASC,
         created_at ASC, id ASC
```

- `id ASC` *is* §3.1 step 5 inside this store: the store's
  identities are integers, so integer order and the
  digit-count-then-digits rule coincide. The string form of
  the rule governs cross-target surfaces (§3.4), where work
  IDs travel as decimal strings.
- The effective-urgency leg is written with today's
  `priority` column because, until PRI-02's derivation lands,
  the broker's stored value *is* the effective value under
  the pinned profile; when derivation lands, this leg's
  expression changes under PRI-02's sequencing, and the
  parity fixture (§3.5) is re-pinned — the tuple does not
  change.
- **One ordering, both paths.** After this slice, claim and
  list may differ in *which rows* they return (claim takes
  the head; list pages), never in the order of the rows they
  share. The V1 divergence recorded in the header is a
  defect this slice repairs, and the §8 oracle asserts the
  repair against both paths.

### 3.4 The selection-order digest

This slice's contribution to the `fleet.selection-order/v1`
object (architecture.md; DST-06 is its host-side consumer)
is fixed as follows: each `ordered_items` entry carries the
item's **normalized** peer rank as a canonical signed
decimal string in `peer_rank` — never null, never a number —
alongside the `effective_urgency` and `created_at` the
architecture already names, all in §3.1 comparator order,
over the complete eligible population. A partial or
truncated population must not emit a conformant `order_hash`
(the architecture's rule, restated because rank makes
top-page shortcuts more tempting, not less). A matching
hash across two surfaces is evidence they ran the same
tuple version over the same pinned inputs; it is not a
claim grant and not proof the inputs were complete.

### 3.5 Parity proof shape

**Decision.** Parity is proven by a golden fixture, not by
inspection: one pinned snapshot (identities, urgencies,
ranks including `NULL`s, creation times) with its expected
total order, executed against (a) the pure Rust comparator
over every permutation of the input, (b) the claim path's
ordering, and (c) the list path's ordering. All three must
produce the identical sequence, and the fixture's
`order_hash` must match the architecture's golden framing.
The fixture lives with the implementation (§8); this record
fixes that there is one fixture and three executions, so a
future "the SQL is obviously equivalent" cannot substitute
for running it.

## 4. Before/after commands

### 4.1 Shape

A rank mutation is a `fleet.command/v1` command (CLI-01
envelope; allocated keys `flag.priority-before`,
`flag.priority-after`, `flag.order`, `priority.move`)
carrying:

- the **target**: `(authority_id, work_id)` and its expected
  work version;
- the **anchor**: the `(authority_id, work_id)` of the peer
  the target is placed immediately before or after, and the
  anchor's expected work version;
- the **group**: the queue and the effective-urgency value
  both items are asserted to sit in, under the pinned policy
  snapshot the caller ordered by;
- the **observed `rank_epoch`** of that group (§5.3);
- `owner_policy_ref` and the shared reason object (code,
  evidence refs, confidence), per the `Command` contract.

There is no "move to position N" form: positions are
unstable under concurrent inserts, and a numeric position
cannot name its intent after the fact. Before/after against
a named, versioned anchor is the only relative form; an
absolute form, if ever wanted, is a new command key under a
future slice, not a flag on this one.

### 4.2 Preconditions, checked atomically

The command is accepted only if, **in the committing
transaction**, all of the following hold:

1. Target and anchor are distinct, both exist, and both
   currently sit in the named group (their current effective
   urgencies equal the named value under the current pinned
   inputs — see §4.4 for the mismatch path).
2. The anchor is **adjacent to the insertion point**: in the
   group's current order, the anchor is the immediate
   predecessor (for *after*) or immediate successor (for
   *before*) of the position the target will occupy, once
   the target's own current position is discounted. A third
   item sitting where the caller believed a gap was makes
   the neighborhood stale, however fresh the versions look.
3. Both expected work versions match the current rows.
4. The observed `rank_epoch` equals the group's current
   epoch.
5. The protected-owner gate (§4.3) admits the mutation.

On acceptance the broker allocates per §2.1 (escalating to
§5 when no gap exists), writes the target's explicit rank,
increments the group epoch by one in the same transaction,
and returns a `Result` recording the target's before/after
ranks and the epoch transition. On any precondition failure
**nothing is written** — no rank, no epoch increment, no
audit row claiming an effect.

### 4.3 The protected-owner gate

Every rank mutation first passes the protected-owner policy
of the package architecture (consumed, not restated): a
routine `priority.write` authority cannot move a protected
member's rank, move another item across a protected member
in a way that changes the protected member's peer placement,
or amend protected membership. Only the specifically
authenticated owner amendment admits those. Denials ride the
shared gate's typed vocabulary and change no rank, epoch, or
membership. The §8 oracle carries the protected cases as
negative fixtures alongside PRI-01/PRI-05's, per the
component's shared-oracle note.

### 4.4 Cross-group moves require confirmation

Rank never overrides urgency (§3.1 step 1 dominates step 2
by construction), so a "move" whose target and anchor sit in
different effective-urgency groups is **not a rank
operation**, and this slice refuses to let it masquerade as
one:

- Without explicit confirmation, the command is refused
  with the typed error
  **`cross_priority_move_requires_confirmation`** — including
  the drift case, where the caller named the group correctly
  but the target's (or anchor's) effective urgency has since
  changed: from the command's side, asserted group ≠ current
  group is the same refusal, and the `Result` names which
  side drifted.
- With the confirmation carried **as a field of the
  command** (never as a UI gesture one surface can forget),
  the operation is a compound: first the urgency change —
  owned by the profile semantics of PRI-01/PRI-02 and the
  §4.3 gate, recorded as its own effect with its own
  before/after — then placement in the destination group at
  the anchor's gap under §§2.1/4.2, preconditioned on the
  destination group's epoch. The `Result` records the two
  effects separately; a failure of the second after the
  first commits is reported as a partial compound outcome,
  never silently retried as a fresh rank command, and never
  presented as a single verified effect (the family's
  no-false-verified rule).

### 4.5 Refusal vocabulary

This slice's typed refusals, all with zero writes:

| Refusal | Cause |
| --- | --- |
| `stale_neighbor` | §4.2(2) adjacency fails, or §4.2(3) a work version mismatches |
| `rank_epoch_conflict` | §4.2(4) the group epoch moved since the caller's snapshot |
| `cross_priority_move_requires_confirmation` | §4.4 group mismatch, unconfirmed |
| `rank_rebalance_required` | §5.2 the local range cannot be re-spread within bounds |

Unknown command versions, ambiguous identity (an
`authority_id`/`work_id` pair that does not resolve to
exactly one row), and unauthorized actors refuse through
the shared CLI-01/lifecycle vocabulary, per the slice
body's negative-case list; this record adds no parallel
spellings for them.

## 5. Local deterministic rebalance

### 5.1 Trigger

A rebalance is triggered only by allocation (§2.1): the
bounding ranks differ by less than 2, or an endpoint
extension would leave the `i64` range. Nothing else
renumbers ranks — not reads, not claims, not triage sweeps
(PRI-05 may *request* moves; it does not rebalance), and
never a background process. Rebalance is a subroutine of a
move command that a caller asked for and can see refused;
it is not maintenance that happens to a group unobserved.

### 5.2 Scope: the bounded local range

**Decision — the run.** The rebalance re-spreads the
smallest contiguous run of the group's current order that
(a) contains the insertion point and both its bounding
members, and (b) can be re-spread at stride `S` inside the
ranks bounding the run (the rank of the member before the
run, or the `i64` floor if none; the rank of the member
after the run, or the ceiling if none). The run grows
outward one member at a time — alternating sides,
predecessor first — until (b) holds. Deterministic means
exactly this growth rule: two executions over the same
snapshot choose the same run.

**Decision — the bound.** A rebalance rewrites at most
**64 ranks** (`REBALANCE_MAX_ITEMS`). If the minimal run
exceeds the bound, or cannot fit even at the bound (only
reachable with a group pressed against an `i64` extreme),
the command fails with **`rank_rebalance_required`** and
writes nothing. A whole-group renumber is thereby never a
side effect of a move: it is a separate, explicitly scoped
operator action under its own issue, with its own evidence
— this slice neither performs nor pre-authorizes it.

**Decision — re-spread.** With the run fixed as
`m_1..m_n` in current comparator order, and `lo` the rank
of the member before the run (or the value derived below)
and `hi` the rank of the member after it, new ranks are
`base + S*i` for `i = 1..=n`, where `base` is:

- `lo`, when a predecessor exists and `lo + S*n < hi` (or
  no successor exists and `lo + S*n` is in range);
- `hi - S*(n+1)`, when no predecessor exists and that value
  is in range (the run is anchored to its successor);
- `-(S * (n/2))` (integer division) when the run is the
  whole group — the spread straddles the common rank `0`,
  so a from-scratch group is centered, not pinned to an
  extreme.

If none of the applicable cases fits, §5.2's refusal
applies. The target's insertion then proceeds inside the
re-spread run by §2.1 allocation, in the same transaction.

### 5.3 Epoch

**Decision.** Each group carries a **`rank_epoch`**: a
non-negative integer counter, starting at `0` when the
group's first rank write creates it, incremented by
**exactly one** per accepted rank mutation in the group (a
plain allocation and a rebalance each count once), in the
same transaction as the ranks it covers. The epoch is the
group's optimistic-concurrency token: §4.2(4) makes every
move name the epoch it read, so two moves composed against
the same snapshot cannot both land — the second sees
`rank_epoch_conflict` and re-reads. An emptied group keeps
its epoch row: epochs **never reset**, so a group that
later refills cannot accidentally validate commands
composed against its previous incarnation. The epoch is a
command-precondition fact; it is deliberately **not** a
field of the selection-order object (§3.4), whose snapshot
reference already binds the derivation.

### 5.4 Atomicity and audit

**Decision.** A move — with or without rebalance — commits
in **one store transaction**: the rank writes, the epoch
increment, and the audit record land together or not at
all. There is no interleaving in which ranks changed but
the epoch did not (which would admit a stale command
against moved ranks) or the epoch moved but ranks did not
(which would refuse fresh commands against unmoved ranks).

**Decision — audit.** Every accepted mutation appends one
record to the broker's existing `events` surface (CEN-04
§3.6 — the broker events table is the journal-adjacent
record of broker effects; this slice adds event *kinds*,
not a store): `peer_rank.moved` for a plain allocation,
`peer_rank.rebalanced` for a move that re-spread a run.
The record carries the operation id, the group (queue +
group key), the epoch before/after, the acting principal
(not merely `actor_ref`), the reason code, and — for a
rebalance — **every** changed identity with its old and new
rank, not a summary count. An audit that cannot enumerate
what changed is not an audit; §8 asserts the enumeration
against the fixture's changed set. Refusals append nothing
to this trail (a refusal is visible in its `Result`; the
events surface records effects, and a refused command has
none).

## 6. Concurrency: stale neighbors and epochs, restated as one model

The failure family the slice body names — stale input,
duplicate/replay, partial remote effect — reduces here to
one rule: **a move is a function of a snapshot, and it
commits only against the snapshot it names.** The named
snapshot is (group membership by current effective urgency,
anchor adjacency, both work versions, group epoch). Any
element drifting refuses the whole command under §4.5's
vocabulary, with the drifted element identified. Retrying
is the caller's explicit act against a fresh snapshot —
the broker never re-targets a stale command onto a new
anchor, never widens a stale neighborhood into a rebalance
to make it fit, and never treats a replayed command as a
new intent: a duplicate submission carries the same
operation id and is answered from the original outcome
under CLI-01/REL-03 idempotency, not re-executed against
moved ranks. Partial remote effect is impossible by
construction (§5.4's single transaction on the broker
path); on any future hosted path, the same rule binds —
the hosted door answers the broker's committed outcome, it
does not maintain a second rank state that could diverge
(component #9199 V4 consumes this contract; it does not
re-decide it).

## 7. Persisted fields: CEN-04 §5 admission entries

This slice introduces exactly two persisted facts. Their
admission entries, in CEN-04 §5's shape, are supplied here
so the CEN-04 amendment PR can carry them verbatim; per
§9, **no rank write merges before its matrix row does**.

### 7.1 `tasks.peer_rank`

1. **Fact:** the explicit peer rank of one task within its
   (queue, effective-urgency) group — §2's `PeerRank`.
2. **Owner:** the queue broker (`fleet-queue`); design
   authority #9199 / this record (PRI-03).
3. **Home:** the broker store's existing `tasks` table —
   one additive nullable `INTEGER` column, `peer_rank`.
   No new store, table, or database.
4. **Write API:** the broker's rank commands of §4 only.
   Enqueue never sets it; urgency edits never touch it; no
   other verb writes it. **Read API:** claim/list ordering
   (§3.3), the selection-order object (§3.4), broker
   `show`/`list` display.
5. **Legacy mapping:** legacy rows hold `NULL`, normalized
   to `0` on read (§2). Refusals, per CEN-04 §4's standard:
   `tasks.priority` is **not** rank (it is urgency — CEN-04
   §3.3's row); `created_at` is a tie-break, not a rank;
   a Project board `POSITION` is a display projection,
   not rank; label bands (`expedite`/`next-up`) are
   urgency inputs, not rank.
6. **Disposition:** **absent — gated** on (a) this record
   merged, (b) the CEN-04 matrix amendment carrying this
   entry merged, (c) the separately scoped implementation
   issue (§9). **Rollback:** disable-only — the comparator
   flag returns every rank to normalized `0` (order falls
   back to urgency, creation, identity: exactly §3.3's
   repaired order without step 2), writes stop, the column
   and its values are retained for re-enable or later
   retirement under the owner's sign-off (CEN-04 §7's
   shape).

### 7.2 `peer_rank_epochs.epoch`

1. **Fact:** the rank epoch of one (queue,
   effective-urgency) group — §5.3's counter.
2. **Owner:** the queue broker (`fleet-queue`); design
   authority #9199 / this record (PRI-03).
3. **Home:** the broker store — one additive table,
   `peer_rank_epochs`, keyed by `(queue, group_key)` where
   `group_key` is the canonical decimal string of the
   group's effective urgency, or the literal `untriaged`
   for the null group; `epoch INTEGER NOT NULL`. Rows are
   created lazily at a group's first rank write and never
   deleted (§5.3).
4. **Write API:** the broker's rank commands of §4 only,
   as the atomic increment of §5.4. **Read API:** the
   §4.2(4) precondition check; broker `show` for operator
   visibility.
5. **Legacy mapping:** none exists. Refusal:
   `leases.lease_generation` (CEN-04 §3.4) is a fencing
   fact about claims, **not** a rank epoch; a task's work
   version is a per-item fact, not a group counter.
6. **Disposition:** **absent — gated**, same three
   conditions as §7.1. **Rollback:** disable-only, together
   with §7.1 (the two facts land and retire as one
   capability); rows retained.

### 7.3 What is deliberately not persisted

The audit trail of §5.4 rides the existing `events` table
as new kinds — no field is admitted for it, because no new
fact is created: the event row is the broker's existing
effect record, and CEN-04 §3.6 already owns its row.
Rebalance runs, midpoint choices, and refused commands are
**not** persisted: they are derivable from, or answered by,
the records above, and persisting derivations would create
the second-authority smell CEN-04 §2 rule 1 forbids.

## 8. Acceptance oracle, specified

The slice's first failing tests, specified here for the
separately scoped implementation issue (which owns them at
the slice body's named targets in `work/priority.rs` and
`turso_queue.rs`). Per the body, each test is first listed
by exact name with `-- --list` and asserted present; a
zero-test filter passing is not acceptance. The done
commands are the body's own:

```text
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue \
  --lib --features client,queue-broker pmo_peer_rank_rebalances_local_range
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue \
  --lib --features client,queue-broker pmo_work_id_decimal_strings_order_numeric_not_lexical
```

- **`pmo_peer_rank_rebalances_local_range`** — the
  rebalance oracle. Fixtures: (a) a group whose bounding
  ranks are consecutive integers (`0`, `1`) with a move
  into the gap: a local rebalance occurs, the rewritten
  set is exactly the §5.2 run and no larger, every new
  rank respects the stride spread, the epoch increments
  by one, and the `peer_rank.rebalanced` audit record
  enumerates precisely the changed identities with
  old/new ranks; (b) the same snapshot permuted yields
  the identical final order and identical changed set;
  (c) a group engineered past `REBALANCE_MAX_ITEMS`
  refuses with `rank_rebalance_required` and **every**
  rank and the epoch are byte-identical afterwards (no
  partial updates); (d) a stale-anchor move and an
  epoch-moved move refuse as `stale_neighbor` /
  `rank_epoch_conflict` with zero writes; (e) a
  cross-group move refuses unconfirmed, and the
  confirmed compound records its two effects separately;
  (f) protected-member moves refuse under the §4.3 gate.
- **`pmo_work_id_decimal_strings_order_numeric_not_lexical`**
  — the comparator oracle. Fixtures: (a) two candidates
  equal on urgency, rank, and creation time, with work
  IDs `9` and `10`: `9` orders first, in the Rust
  comparator, in the claim path's order, and in the list
  path's order (§3.5's three executions over one fixture);
  (b) IDs beyond float53 (`9007199254740993` vs
  `9007199254740992`) order by true numeric value,
  proving no float coercion anywhere on the path;
  (c) `NULL`-rank legacy rows interleave at normalized
  `0` by creation time and identity; (d) the fixture's
  `order_hash` matches the architecture's golden framing
  over the full population.
- **Negative coverage** (the slice body's list, mapped):
  stale input (fixtures d/e above), duplicate/replay (§6's
  idempotent answer), ambiguous resource (an identity pair
  resolving to zero or two rows refuses, writes nothing),
  unauthorized actor (the §4.3 and shared authorization
  refusals), partial remote effect (impossible by §5.4;
  asserted by killing the transaction mid-fixture and
  observing all-or-nothing), unknown version (an unknown
  tuple or command version refuses, never degrades to a
  guessed order).

## 9. Gates and sequencing

Implementation against this record starts only when **all**
of the following hold:

1. This record is merged.
2. A separately scoped implementation issue is filed
   against it, carrying §8's oracles as its acceptance.
   #9511 itself authorizes no code; it stays open as the
   slice tracker.
3. The shared-file sequencing edges from the slice body
   are honored: **PRI-02** ([#9479](https://github.com/developing-today/code/issues/9479))
   lands its `work/priority.rs` foundations first (the
   profile and dimension types this slice's comparator
   consumes), and **HYD-03** ([#9460](https://github.com/developing-today/code/issues/9460))
   lands its `turso_queue.rs` changes first. Both are
   implementation-sequencing edges; neither gated this
   record, per the sibling pattern (HYD-05, REL-06, CEN-04
   all landed as records while their implementation
   prerequisites stood open).
4. The CEN-04 amendment carrying §7's two entries is
   merged **before the first rank write** in the
   implementation series (CEN-04 §5 mechanized: a
   persisted field with no matrix row may not be written).
   The amendment is a small PR against the CEN-04 record
   (a §3 row beside §3.3, plus the §5 pre-filed list
   updated); this record supplies its text.
5. The schema step itself (two additive changes in the
   broker store) is executed under the program's
   migration gates with its own evidence — additive and
   reversible per CEN-04 §7, prior readers working
   throughout. Production rollout and live acceptance
   remain separate stages again.

**Schedules:** this record creates, changes, and retires
no schedule on any surface; `doc/fleet-schedule-registry.md`
is untouched.

## 10. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records only which part of this design speaks
to each group — it claims design coverage, not
implementation.

| IDs | Where addressed |
| --- | --- |
| C-R002, C-R041, C-R046 | §2, §3.1 — exact integer rank and the deterministic total order within equal effective urgency |
| C-R005, C-R009 | §3.3, §3.5, §8 — claim/list parity and the comparator oracle carried into implementation acceptance |
| C-R049 | §4.4 — cross-group moves as confirmed compounds, never silent rank effects |
| C-R053 | §8 — the golden-tie example (`9` before `10`) as a pinned fixture |
| C-R107, C-R108 | §4.1, §4.5 — the allocated command keys and shared flag vocabulary this contract serves (spelling itself is CLI-02's) |
| H-R004, H-R007 | §4.2, §6 — before/after operations against versioned neighbors; stale input refused, never re-targeted |
| H-R017, H-R024, H-R031 | §8, §9 — oracle, compatibility, and gate coverage carried into the implementation issue's acceptance |
| O-R003, O-R004, O-R005, O-R006, O-R007, O-R009 | §2–§5 — candidate occurrences behind the rank, allocation, and rebalance rules and their bounds |
| O-R017, O-R023 | §3.4, §6 — selection-order fingerprint inputs and replay/idempotency behavior |
| O-R038, O-R054, O-R055, O-R056, O-R057, O-R059, O-R066, O-R067, O-R068, O-R069 | §4, §5 — proposal occurrences behind the move preconditions, epoch conflicts, and bounded atomic rebalance |
| P-R009, P-R014 | §3, §7 — proposal occurrences behind comparator parity and the persisted-field admissions |
| S-R005, S-R007 | §5.2, §5.4 — bounded range and atomic, audited mutation as the safety case |

## 11. Non-goals

- No urgency semantics: no profile values, no importance
  or effective-pressure derivation, no aging, hysteresis,
  or triage sweep (PRI-01/PRI-02/PRI-05). This record
  consumes effective urgency as a pinned input.
- No dependency pressure or cycle semantics (PRI-04), no
  PM/projection adapter (PRI-06), no shadow report or
  cutover (PRI-07). Hosted conformance (#9079/#9081, V4)
  consumes §3/§6; it is not designed here.
- No new queue, database, scheduler, service, resolver,
  executable, or GitHub client (the slice body's boundary,
  restated). The two §7 additions live in the broker's
  existing store.
- No whole-group renumber facility (§5.2 reserves it to a
  separate operator action and issue), no position-number
  move form (§4.1), and no uniquifying of urgency scores
  to force an order.
- No change to #6533 or its live acceptance, and no edit
  to any other slice's owned files: `work/priority.rs`
  and `turso_queue.rs` are the only implementation
  surfaces, and only under §9's gates.
