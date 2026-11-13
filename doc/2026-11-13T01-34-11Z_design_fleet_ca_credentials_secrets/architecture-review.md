# Fleet credentials and CA: architectural synthesis

**Design mission, not a production rollout.** Starter #9155; task 15345.
V1 means the pinned existing system, not a proposed first release. V2–V4
are additive design layers; the source package's `v0.3` is an input version,
not an assertion that those layers have shipped.

## Architectural decisions retained by the Summit

### ADR-S01 — One policy contract, separate authority planes

**Problem.** A convenient shared resolver must not turn every readable secret
into a replicable secret or every stored root key into a usable signer.

**Decision.** Separate the read plane, administrative mutation plane, signing
plane and escrow plane. Share logical identities, typed policy and redaction;
do not share implicit authority. A provider's capability report says what it
can do, an authorization grant says what this principal may do, and an
operation plan says what it intends to do. All three must agree at execution.

An ordinary `SecretRef` does not implement a root signer capability. A
`RootEscrowPolicy` is not an ordinary replication policy. Discovery of root
material inside an otherwise authorized compartment must fail closed or
quarantine it for review, not silently enroll it in daily reconciliation.
Root private material is excluded from ordinary hot caches, prefetch,
benchmark datasets and metadata equality indices. An explicitly authorized
signer session can hold material for its bounded operation; that exception
does not enable general caching.

**Root signing authorization is transaction-bound (review finding A-2).** An
explicit signer reference plus a generally authorized principal is not
enough. A root signing operation consumes a `SigningAuthorization` bound to:
the root key fingerprint or handle; the subject public-key identity and the
requested signature algorithm; profile and purpose; exact validity; the
digest of the artifact plan; and any authorized output or escrow
destinations. It carries an expiry and is single-use by default (bounded-use
only when the plan enumerates every signature in advance). The signer adapter
refuses on any mismatch and returns a receipt naming the transaction it
executed, so an offline or hardware signer can attest the plan it signed
without exposing key material. Public inventory lookup, escrow read access
and a valid certificate path never mint this authorization, and holding key
bytes is not authorization. The concrete token format and ceremony are
implementation work; the binding fields and the refusal cases are fixed here.
Negative tests: ambient discovery, signer substitution, changed plan digest,
wrong purpose, expired grant, replayed grant, and a signer handle presented
as an ordinary secret reference.

**Alternatives.** One universal provider object is initially simpler but
creates confused-deputy and future-provider expansion risks. Independent
unrelated tools avoid coupling but duplicate policy and break library/CLI
consistency. Capability-separated interfaces behind Fleet preserve reuse
without granting blanket authority.

**Consequences/reversal.** Every adapter and CLI entry must enforce the same
typed policy. Tests attempt cross-plane escalation with ambient credentials,
misclassified records and new providers. Revisit interface boundaries if a
real HSM or broker integration requires an additional authority type, not by
making a general secret handle implicitly sign.

### ADR-S02 — Reconciliation is a recoverable saga, not distributed ACID

**Problem.** Independent hosted providers do not offer one joint transaction,
and not every adapter offers conditional updates.

**Decision.** Plan from a last **verified** baseline and complete authorized
scope observations. Make conflict decisions per field. Persist encrypted
recovery material and non-secret intent before mutation; preserve immutable
history before overwriting the current value. Apply with provider-specific
concurrency checks, read back, record receipts, and advance only the verified
baseline components. A timeout after a write is an uncertain outcome, not a
failed write to repeat blindly. Resume by native IDs and readback evidence.

**Conditional-write capability (review finding A-1).** Each adapter declares,
per operation, one of three write-guard levels, established by primary
documentation plus an isolated test, never assumed from a revision timestamp:

- `atomic-precondition` — the provider itself rejects a write whose observed
  native revision is no longer current, for every writer.
- `serialized-by-contract` — every writer of that scope is known to go through
  one mechanism that serializes them. A Fleet-local lock or lease qualifies
  only if no human, app or other tool can write the scope.
- `unguarded` — neither holds. A preflight read is not CAS, a Fleet lease does
  not bind external editors, and readback detects an overwrite only after the
  peer's value is gone. A saved pre-write snapshot is the old value, not
  necessarily the concurrently written one.

Automatic apply of a change to a conflict-sensitive field (any secret value,
and any field both sides may edit) requires `atomic-precondition` or
`serialized-by-contract`. On an `unguarded` target the plan step stops as
`needs-review`, or runs under a separately approved, explicitly named weaker
policy whose residual lost-update window is stated in the plan and receipt.
Additive operations that cannot destroy a peer value (create a new item,
write a new immutable history record) are not gated this way. No adapter is
treated as guarded by default; until evidence exists, BWS, `op` and `bw` are
all `unguarded`. Synthetic model tests cover every crash boundary and the
schedule preflight → external edit → apply, whose oracle is that the external
value stays recoverable and the step reports conflict, not success.

**Alternatives.** Timestamp-wins loses concurrent password edits; wholesale
item replacement destroys native identity/autofill; a centralized universal
vault changes custody and creates a migration prerequisite. None is adopted.
The common model can later support dynamic credentials, but leases are not
copied as static passwords.

**Performance.** Bound concurrent scopes, page and batch where adapters
actually support it, coalesce reads, and persist resumable cursors. A complete
inventory token records pagination, grants and scope identity; a successful
HTTP status alone is not completeness. Full-census schedules and incremental
lookback are distinct obligations.

### ADR-S03 — Freshness is part of the result, not a cache implementation detail

**Problem.** An always-available stale password can defeat rotation or
revocation. Existing broker staleness (#6767) is evidence against treating a
warm hit as correctness.

**Decision.** Read requests carry purpose, maximum age, offline allowance and
revocation/security class. Results carry source revision (if available),
verified-at time, freshness and degraded-use status without exposing values.
Single-flight keys include authorization scope and policy, not just a name.
Negative-cache TTLs, stale fallback, invalidation and session expiration are
explicit. No silent downgrade from fresh-only to stale-allowed.

Benchmark CLI, direct SDK/API, Fleet broker and optional fnox separately.
fnox is evaluated urgently but embedding depends on actual reusable APIs,
maintenance and capability tests; it does not become a second control plane.
No provider wins by anecdote, and no library is presumed write-capable.

**Reversal triggers.** Measured regressions, rotation failures, unacceptable
memory exposure, incompatible licensing/API churn, or inability to enforce
scope/freshness justify replacing an adapter, not rewriting the public policy
contract. Performance budgets remain proposals until measured.

### ADR-S04 — A PKI graph is not a trust guarantee

**Decision.** Model key identity, certificate variant, generation, partition,
purpose, signer and trust profile separately. The reference graph has
`4 + (2 purposes × 2 partitions × 4 algorithms) = 20` CA keys. The root
certificate count is `4 + 3×2 + 3 = 13`; each purpose/partition has
`3 classical issuer keys × 3 classical root signers + 1 ML-DSA issuer key ×
4 root signers = 13` certificate variants. Thus `13 + 4×13 = 65`, before
leaves. New purposes/partitions grow independently; avoid materializing
unrequested complete path combinations.

The 65 variants are a bounded reference compatibility obligation, not an
instruction to install 65 trust anchors. Trust kits install only selected
self-signed roots. Cross-certificates are untrusted path candidates. For a
strict PQ authentication claim, require the ML-DSA trust anchor, ML-DSA
signatures throughout, and an eligible PQ leaf authentication key/protocol;
a classical leaf key under a PQ issuer is not quantum-resistant endpoint
authentication. PQ key exchange is a separate property.

Proactively emit known-good chains, normally omitting the self-signed root
from TLS presentation. AIA is a best-effort aid, not a universal path-search
oracle. Revocation is indexed by actual signing issuer/key and serial, not
the human issuer-set name. Cross variants are distinct certificates with
distinct issuer/serial identities.

**Alternatives.** Roots-only is retained for ordinary future profiles because
direct variants add revocation/publication burden. It cannot replace the
required reference `both` profile. MTC remains optional and does not change
the reference graph or baseline cryptography.

### ADR-S05 — Durable ciphertext and independent restoration precede convenience

**Decision.** Prefer existing age/SOPS formats and secure provisioned key
locations over a new envelope format without demonstrated advantage.
SecureStore is a comparative evaluation, not automatic adoption. Initial
root escrow has exactly three named approved destinations: encrypted laptop
storage, authorized BWS project and authorized 1Password vault. Archive
integrity/readback is necessary but insufficient: restore each root and the
whole bundle using separately protected recovery material in an isolated
synthetic test. Do not confuse successful access through one live provider
with independent recoverability from that provider's outage.

The three destinations are a closed, enumerated set inside a dedicated
root-escrow policy type, never a provider list, a wildcard or "all authorized
replicas" (review finding A-3). Custody is reported complete only when all
three declared copies are present and each has passed its own isolated
restore with the other two unavailable; a missing destination or one failed
restore is reported as incomplete custody, not as success with a warning.

Draw the recovery dependency graph including recipient keys, provider auth,
MFA, laptop loss and WAN loss. Reject circular dependencies. A future
offline/HSM migration separates active signing handles from escrow and
retains historic public artifacts. R2 needs an independent research/security
decision; adding its adapter cannot expand root custody.

### ADR-S06 — Reuse external control planes; do not recreate them inside Fleet

Fleet owns composable policy, artifacts, local orchestration, adapters and
evidence. OpenBao may own leases/auth/Transit/issuance when justified;
Kubernetes operators may own workload delivery and renewal. Fleet must not
pretend a CLI generator is a continuously operated CA service. Existing
id/iroh identity, PM queue/outbox and SSO/OIDC programs are integration seams,
not parallel implementations or automatically expanded epics.

Three containers on one host demonstrate protocol behavior, not availability.
HA/DR claims require fault-domain, quorum, seal, backup/restore, operator and
site-loss evidence. Research future options on their own merits, with
falsifiable go/no-go gates and no baseline dependency on their success.

## Version layers and implementation authority

| Layer | Meaning | Exit evidence |
|---|---|---|
| V1 | Existing Fleet/Nix/broker behavior at the census commit, including defects and absent APIs | Code/test references; deployment claims separately qualified |
| V2 | Typed contracts, capability adapters, synthetic reconciliation/graph models, benchmark harness, generators and format proofs | Pure/model tests, mock failures, independent format validators; no production authority |
| V3 | Full administration, guarded reconciliation/custody/issuance/publication and operator kits | Synthetic end-to-end acceptance first; separate approved rollout and restoration records before production operations |
| V4 | Dynamic credentials, offline/hardware signing, independent-site federation and controlled-stack experiments | Separate research decisions, threat models and operations ownership; no retroactive dependency on V2/V3 |

Versions are not all-or-nothing waterfall releases. Foundations can proceed
in parallel when interfaces and file ownership permit it. A native blocking
edge expresses an actual prerequisite, not merely a nice-to-have ordering.
Approval-blocked rollout does not block a mock or a synthetic implementation
that never exercises production authority.

## Cross-cutting acceptance and reliability

- Every significant state machine has a reference model and generated fault
  sequences. Every automated test names a decidable oracle. Example-only
  happy paths do not establish reconciliation or cryptographic invariants.
- Parse and validate the **entire** operator command before file creation,
  subprocesses, network calls, locks or secret access. Reject unknown flags
  and contradictory policies. Plan/apply modes and authorization are separate.
- Cancellable, bounded work replaces unbounded fan-out. Retry only classified
  operations with idempotency/recovery evidence; observe rate limits without
  treating token rotation or batching as unlimited quota.
- Public journals/SQL/metrics contain no secret values or password diffs.
  Encrypted recovery payloads have independent access/retention policy.
  Blinded equality tokens still leak equality and require scoped keyed design;
  do not describe a hash of a low-entropy password as safe anonymization.
- Root operations, trust/DNS changes, actual schedules and deployments retain
  explicit approval and rollback boundaries. Future timer implementation must
  update `doc/fleet-schedule-registry.md` in the same change; this design run
  creates no operational schedule.
- Merged design, published issue, passing synthetic fixture, implemented API
  and deployed/authorized production behavior are separate evidence states.

Domain designs, research, inventories and issue receipts add the concrete
interfaces and tests. Any conflict with these authority decisions is resolved
explicitly, not by silently broadening a provider capability or trust claim.
