# Decision: owner override mechanics & the encryption boundary (#5819)

Status: decided (this record). Parent design: #5675 (querying ID tables,
local and remote). Dependency: #5817 (sensitivity classification model —
S0–S3 levels, raise-only refinement, S3 local-only). Siblings: #5816
(query shape), #5818 (filtered remote execution, withheld-result honesty,
audit), #5820 (client surfaces), #5821 (result shaping), #5822 (mutating
surface). Programme: epic #2056.

This record settles only what #5819 owns: **the owner override's
activation shape, its audit, the multi-principal rule, and how it
composes with S0–S3**. It does not re-decide the parent's fixed boundary,
which it honours verbatim:

> The override applies only on the owner's **own server**, only to data
> that is **not encrypted**, and **encryption defeats the override** — no
> override, admin flag, or local access returns plaintext under keys the
> querying identity does not hold. Classification is a policy filter;
> encryption is a cryptographic boundary, and policy never outranks
> cryptography.

Mechanically: the override is a **policy-level elevation evaluated after
decryption, never a decryption capability**. It can move the effective
clearance used by the classification filter; it cannot move a key.

## 1. What the override is

The override is a **declared act by a principal on its own server** that
raises the clearance used to evaluate classification filtering for one
query or one session, above what the identity's ordinary clearance would
be. It changes exactly one input to the filter — the effective clearance —
and nothing else:

- it does **not** widen a selective-sharing grant (§10 of the parent:
  grants decide reach, classification decides what a query returns; the
  override never makes remote data reachable);
- it does **not** operate over the wire — it is a local capability and
  has no remote activation path, by construction (own server only);
- it does **not** decrypt, and it is evaluated only over data the server
  can already read in plaintext for that principal;
- it does **not** appear in query text, so the query language gains no
  mutation-like verb and §7's read-only-by-construction property holds.

**An override that produces no elevation is not an error.** Asking for
S3 clearance when the identity already holds it is idempotent; asking
for it on a server where the identity is not an owner is a refusal (§4),
not a silent success.

## 2. Activation shape: explicit, deliberate, bounded

Chosen shape: **explicit opt-in, per query or per session, never a
persisted setting, never automatic.**

- **Per query.** The requester names the override on the query
  invocation itself (a flag on the CLI/API surface, a field on the
  agent-facing query object). Its effect ends with that one execution.
- **Per session.** A requester may open a session with the override
  active; it lasts exactly as long as that session and is dropped on
  process/session end. A per-session override must be re-declared each
  session — there is no config key, environment variable, dotfile, or
  server-side default that turns it on.
- **Never a permanent setting.** No `override = true` in a config file,
  no "remember this" prompt, no inheritance into child sessions spawned
  by the overriding one. A surfaced text that offers it must say it is
  for this action/session only.
- **Never silent.** An override query that returns elevated results is
  only valid if the audit entry (§3) was written; a failed audit write
  fails the query (§3, invariant 4). This is what makes "explicit and
  deliberate" observable rather than aspirational.
- **Confirmation on interactive surfaces.** The human CLI requires an
  explicit confirmation gesture the first time an override is used in a
  session, stating the target data and the level being assumed. The
  script/API and agent surfaces have no prompt (non-interactive safety,
  parent §6) — for them the declared opt-in plus the audit record is the
  deliberate-act evidence.

Rejected: a persistent per-identity "owner mode", and an implicit
override triggered by local socket/loopback detection. The first is the
"permanent silent setting" the parent forbids; the second makes a
privilege elevation a side effect of network topology, which is
indistinguishable from a bypass and is exactly what "no privileged back
door" (parent §6) rules out.

## 3. Audit: mandatory, local, owner-visible

Every activation writes an audit entry **before** results are returned,
on the server that evaluated the query.

**Content** (minimum; a superset is allowed):

- timestamp and query/session identifier;
- the acting principal (identity) and the channel (CLI, script/API,
  agent);
- the requested effective clearance (e.g. `S3`) and the principal's
  ordinary clearance;
- the scopes touched: tables/collections queried, and counts per
  sensitivity level actually returned;
- for a per-session activation: the session's start and end, and the
  number of queries executed under it.

The audit records **that elevated data was returned and how much per
level** — never the data itself, never a field value, never a query
result body.

**Location and visibility.** The audit lives on the serving server, in
the same audit stream the parent requires for remote queries (§4) — one
local audit mechanism, not two. It is visible to the server's owner, and
to no one else. It is append-only. For the multi-principal case (§4) the
entry carries the data-owner attribution so an owner can see overrides
taken over their data and an operator can see their own acts.

**Failure behaviour.** If the audit entry cannot be written, the
override does not apply and the query fails with a machine-readable
error. Fail-closed: an unaudited elevation is not a permitted state.

## 4. Multi-principal rule

The question the parent leaves here: on a server with more than one
principal, **whose override applies to whose data**, and can the model
prevent an operator from overriding other principals' unencrypted data?

Decided: **ownership is per data, not per host.** A principal may
override filtering only over data they own, plus data with no other
attributed owner (single-user servers, and server-scoped system data
whose owner is the server's operator).

- **Data owner overrides their own data.** Unconditional within the
  boundary: own server, unencrypted.
- **A second principal's data is not reachable by another principal's
  override** — not by an operator, not by an admin flag, not by a
  "superuser" identity. The operator of the server is an **administrative
  role over the machine, not a data owner over other principals' data**.
- **What an operator can do** is deliberately narrow and visible:
  administer the server, read the audit stream (including overrides
  taken over their own data), and override only data attributed to them
  or to the server.
- **Prevention is structural, not aspirational.** The override path
  resolves an effective clearance per principal per data scope; there is
  no code path in which an operator identity's override is applied to
  another principal's scope. If a deployment genuinely has no
  multi-principal separation (one human, one identity), that is the
  single-user case, not a licence to generalize the rule.
- **Shared/copied data follows the copy.** If principal A shared data to
  principal B on B's server, B's override reaches B's copy as B's data;
  A's override reaches nothing on B's server (parent §5 scope), and data
  B never received is unreachable to B's override regardless of grants.
- **Encrypted data is out of scope everywhere.** B's copy of A's data,
  if encrypted under A's keys, is not openable by B's override — the
  boundary is cryptographic and precedes the multi-principal rule.

Rejected: **operator-overrides-all** (turns server administration into a
universal read capability — the boundary's whole point is that policy
cannot manufacture plaintext, and this manufactures plaintext access);
**no-override-on-shared-hosts** (collapses the single-user case into a
worse UX for no security gain, since per-data ownership already
expresses the separation).

## 5. Composition with S0–S3 (#5817)

The override changes the **effective clearance** the filter compares
against; the levels and their semantics are #5817's and are not
re-defined here.

| Level | Ordinary remote query | Local query at own clearance | Override effect |
| ----- | --------------------- | ---------------------------- | --------------- |
| S0 Public | returned | returned | none needed |
| S1 Internal | returned if cleared | returned if cleared | elevates clearance only |
| S2 Confidential | returned if cleared | returned if cleared | elevates clearance only |
| S3 Restricted | **never** (local-only, #5817 §5.4) | returned if cleared | **the S3 path** |

- **S0–S2:** the override is equivalent to holding the next clearance up.
  Withheld-result signalling follows #5817 §6 at the effective clearance:
  an override that lifts the requester to S2 stops producing S2
  withholding notices for data it can now see.
- **S3 is the override's reason to exist.** #5817 makes S3 local-only and
  reachable "by local query and by the owner-override path". This record
  fixes that path: an S3-cleared principal's override, on their own
  server, over unencrypted data they own, returns S3 data to a **local**
  query. It does not make S3 cross a remote query (that stays a #5818
  execution rule and #5817 invariant 6), and it leaves no existence
  signal to any remote party.
- **Invariants that do not bend:** refinement stays raise-only
  (#5817 inv. 1); unclassified is S2 (inv. 2); aggregates never span
  withheld data (inv. 5); classification never widens a grant and a grant
  never widens a classification (inv. 8).
- **Withholding under an unsuccessful elevation** is reported exactly per
  #5817 §6 for the level that remains withheld — an override that fails
  to reach a level must not announce the failure in a way that signals
  the withheld data's existence at S3.

## 6. Invariants implementation must hold

1. **Own server only.** No activation path exists for a query whose data
   is served by a peer; the override is local execution only.
2. **Unencrypted only.** The override is evaluated after decryption for
   data the acting principal can already read; it never selects, derives,
   or supplies a key, and never returns ciphertext or plaintext under
   keys the principal does not hold.
3. **Encryption defeats the override.** No combination of override,
   admin role, operator status, or local access returns plaintext under
   keys the acting principal does not hold. The boundary precedes every
   rule in this record.
4. **Explicit and audited, or refused.** An override result is returned
   only if its audit entry was written; audit-write failure fails the
   query (fail-closed).
5. **Bounded lifetime.** No persisted, inherited, or default-on override;
   per query ends with the query, per session ends with the session, and
   confirmation is never "remembered".
6. **Per-data ownership.** A principal's override applies only to data
   they own (or unowned server data); no operator/admin identity
   overrides another principal's data.
7. **Clearance only.** The override changes the filter's effective
   clearance and nothing else: not grants, not reachability, not
   classification values, not the query language (still read-only).
8. **Honesty follows #5817 §6** at the effective clearance, including
   S3's no-existence-signal rule.
9. **One audit stream.** Override and remote-query auditing are the same
   local mechanism; every elevated result is attributable to an entry.

## 7. Alternatives rejected (summary)

- **Persistent per-identity "owner mode":** the parent's forbidden
  permanent silent setting; one forgotten flag silently disables the
  filter.
- **Implicit override on loopback/local socket:** privilege elevation as
  a side effect of topology — a privileged back door (parent §6).
- **Operator-overrides-all:** manufactures plaintext access across
  principals; contradicts per-data ownership and the boundary's intent.
- **Override as a decryption capability:** policy would outrank
  cryptography, which the parent fixes as impossible.
- **No override at all:** leaves S3 unreachable for its own owner and
  contradicts parent §5, which states the override exists.
- **Per-server blanket override for the "primary" user:** untestable as
  a multi-principal rule and indistinguishable from operator-overrides-all
  in effect.

## 8. What this record does not decide

- **Withheld-result signalling mechanics across a remote query** — #5818
  (this record only fixes the S3 no-signal invariant it must honour).
- **Query language shape** — #5816.
- **Surfaces and their presentation** — #5820 (this record fixes that all
  surfaces carry the same declared opt-in and audit, no surface gets an
  extra path).
- **Cursor/result shaping** — #5821.
- **Mutating surfaces** — #5822.
- **Implementation timing** — a separate decision; this record is the
  design the implementation must hold.
