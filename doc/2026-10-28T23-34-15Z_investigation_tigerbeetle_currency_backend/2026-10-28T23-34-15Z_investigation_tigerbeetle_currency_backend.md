# TigerBeetle as the accounting backend for dynamic ID currencies — investigation

**Date:** 2026-10-28 · **Question owner:** Drew's "ID currencies / tokens" directive (LIST 11)
**Scope:** Can one TigerBeetle cluster back a system where any ID can issue named
tokens on the fly, users hold/transfer them, issuers authorize conversions at
issuer-set rates, tokens redeem for real services (e.g. Steam key → premium
tokens), with refunds/revocations — without hand-rolling double-entry accounting?
**Method:** TigerBeetle official documentation (docs.tigerbeetle.com, incl. the
single-page edition), plus the independent Jepsen analysis where noted.
No GitHub issues were filed (rate-limited per task instructions); this file is
the deliverable.

---

## TL;DR verdict

**TigerBeetle fits the _ledger core_ of this design well, and does NOT do most
of the things around it.** It is an excellent, very fast, strictly-serializable
double-entry _engine_: accounts, transfers, balance invariants, atomic linked
batches, two-phase holds, idempotent creation. Everything that makes Drew's
idea a _product_ — currency names, who may issue, exchange rates and who
authorized them, conversion logic, per-coin identity, refunds-as-policy,
revocation/freezing, user authentication, the ID-graph representation — lives
**outside** TigerBeetle, in an application/control-plane layer we would have
to build and operate ourselves.

| Drew's hope                                        | Reality                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| "Supports any number of currencies"                | **Mostly true, with a hard ceiling and no registry.** A currency = a `ledger` value, a bare `u32` integer on accounts/transfers. No creation call, no name, no metadata in TigerBeetle. Ceiling = 2³²−1 ledgers per cluster, shared with every other use of ledgers (tenancy partitioning etc.).             |
| "Supports conversions, so we don't hand-roll them" | **False as stated.** Cross-ledger transfers are _forbidden_. A conversion is an application-composed pair of linked same-ledger transfers through a liquidity provider's two accounts, at a rate the _application_ computes. TigerBeetle only guarantees the pair is atomic.                                 |
| "Handles double-entry so we don't have to"         | **Half true.** TigerBeetle _enforces_ the invariant (every transfer is one debit + one credit; sums stay equal; balance-limit flags are checked atomically). It does **not** design your chart of accounts: issuer, liquidity, fee, burn, user accounts and their flags/codes are your design, per currency. |
| "A cluster of 3, one running, others backups"      | **Wrong mental model.** All replicas run and participate in Viewstamped Replication consensus continuously. There is no passive backup. And the vendor's production recommendation is **6 replicas across 3 sites**, not 3 replicas total.                                                                   |
| "Maybe it IDs every single coin"                   | **No.** Balance-only. There is no coin, UTXO, serial number, or lot object. A transfer moves an integer amount between two accounts; individual units are indistinguishable.                                                                                                                                 |
| "Refunds / revocations supported"                  | **Only as compensating transfers you issue.** Posted transfers are immutable and undeletable. A "refund" is a new opposite transfer; a "revocation" is a clawback transfer that can _fail_ if the holder has already spent the balance. No freeze/clawback primitive exists.                                 |
| "Users issue tokens of themselves"                 | **Modelable, not built-in.** Minting = a transfer out of an issuer account you designate, on that currency's ledger. Who is allowed to mint, supply caps, and the link to an Iroh/ID identity are entirely application-layer (TigerBeetle has **no authentication at all**).                                 |

**Bottom line:** Adopt TigerBeetle, if at all, as the _settlement core behind
our own currency service_ — never exposed to users, fronted by a control plane
that owns currency registration, issuance policy, rates, and corrections. Budget
the real work accordingly: the cluster is the easy part; the currency service
around it is the project.

---

## 1. Dynamic currencies / ledgers

### How a "currency" is represented

> "Distinct `ledger` values denote different currencies (or other asset
> types)." — [Currency Exchange recipe](https://docs.tigerbeetle.com/coding/recipes/currency-exchange/)

> "Ledgers partition accounts into groups that may represent a currency or
> asset type or any other logical grouping… Ledgers are only stored in
> TigerBeetle as a numeric identifier on the account and transfer data
> structures. You may want to store additional metadata about each ledger in a
> control plane database." — Data Modeling, [single-page docs](https://docs.tigerbeetle.com/single-page/)

Facts:

- `ledger` is a **32-bit unsigned integer** field on both `Account` and
  `Transfer`; it must be non-zero, and a transfer's ledger must match both
  accounts' ledgers. (Field types per the client bindings reproduced in the
  TigerBeetle Go DeepWiki: `Ledger uint32`, "Must not be zero";
  `Code uint16`.) So the theoretical maximum is **4,294,967,295 distinct
  ledgers in one cluster** — far beyond plausible need, but it is a finite,
  cluster-wide namespace, and the same namespace is also the documented tool
  for _other_ partitioning (e.g. one ledger per tenant, per-tenant-per-currency
  in multi-tenant setups, per the Data Modeling page). Spending one ledger per
  user-created token is the intended pattern, but ledger allocation becomes a
  governed resource, not an infinite one.
- **There is no "create currency" operation, and none is needed to start.**
  A ledger springs into existence implicitly when the first account is created
  with that integer. Creating a new token at runtime = allocate an unused u32
  in _our_ registry, then create accounts on it. No DDL, no restart, no cluster
  reconfiguration. This part of Drew's "on the fly" requirement genuinely works.
- **Everything human about the currency lives elsewhere.** Name ("Chuck E.
  Cheese tokens", "dt", game coins), symbol, issuer ID, asset scale (decimal
  places), supply policy, icon, description: TigerBeetle stores none of it.
  Amounts are unsigned **128-bit integers**; fractional amounts are represented
  by mapping the smallest unit to 1, and the scale is an application-side
  convention (Data Modeling, single-page docs). The docs' System Architecture
  page is explicit that ledger/account/transfer string names and metadata
  belong in a general-purpose (OLGP) database — the "control plane" — with
  TigerBeetle in the data plane, and that the API service should _cache_ ledger
  metadata rather than look it up per transfer.
- **A ledger's scale is effectively immutable.** "Accounts and transfers are
  immutable once created. In order to change the asset scale of a ledger, you
  would need to use a different `ledger` number and duplicate all the accounts
  on that ledger over to the new one." (Data Modeling, single-page docs.)
  Decide scales generously at issuance; a token launched at scale 2 cannot be
  "upgraded" in place.
- **One account per (holder × currency).** "If a single logical entity holds
  multiple currencies, each currency must be held in a separate TigerBeetle
  `Account`." ([Currency Exchange](https://docs.tigerbeetle.com/coding/recipes/currency-exchange/))
  A user holding 50 game tokens has ≥50 TigerBeetle accounts, plus the issuer/
  liquidity/fee accounts each currency needs (see §2). Account count therefore
  grows as users × currencies, in a single unsharded cluster (see §5).

### What "issuing" looks like

There is no mint verb. The conventional pattern:

1. Create an **issuer account** on the new ledger (the source of supply). Its
   flag choice defines supply discipline: an unconstrained issuer account can
   go arbitrarily "negative" (debit-normal), which is how most ledger systems
   model issuance; a `credits_must_not_exceed_debits`-style constraint would
   cap it. TigerBeetle does not pick for you.
2. Mint = a transfer issuer → recipient. Burn = a transfer recipient → issuer
   (or → a dedicated burn account).
3. Enforcing "only ID X may mint token X" is **our API's job**: "TigerBeetle
   does not support authentication. You should never allow untrusted users or
   services to interact with it directly." (System Architecture, single-page
   docs.) The ID-graph signature layer (a signed "I issue 100 of myself to ID
   Y" object) would be verified in our service _before_ it submits the
   TigerBeetle transfer; TigerBeetle never sees the signature.

**Fit to the ID graph:** TigerBeetle knows nothing about the graph. The
signed issuance/transfer objects in the ID system and the TigerBeetle transfers
would be two records of the same event, linked by storing the ID-graph object
ID in a transfer's `user_data_128` (128-bit, indexed) — the docs explicitly
sanction `user_data_*` (128/64/32) as pointers to control-plane records and as
group identifiers for related transfers. Reconciliation between the two
systems is our problem to design (e.g. TigerBeetle is authoritative for
balances of _centralized_ tokens; the graph is authoritative for claims).

---

## 2. Conversions between currencies

### There is no cross-ledger transfer

> "Transfers between pairs of accounts with different `ledger`s are not
> permitted. Instead, currency exchange is implemented by creating two
> atomically linked different-ledger transfers between two pairs of
> same-ledger accounts." — [Currency Exchange](https://docs.tigerbeetle.com/coding/recipes/currency-exchange/)

A conversion is a **composed pair (or chain), not one transfer**:

- Four accounts: source user account A₁ (ledger 1), destination account A₂
  (ledger 2), and a **liquidity provider** holding L₁ (ledger 1) and L₂
  (ledger 2).
- Transfer T₁: A₁ → L₁, amount in currency 1, `flags.linked = true`.
- Transfer T₂: L₂ → A₂, amount in currency 2, last in chain (linked = false).
- "The transfer amounts vary according to the exchange rate" — i.e. **the
  application computes both amounts**; the rate exists nowhere in TigerBeetle.
  The docs advise rounding in the liquidity provider's favour to deter
  arbitrage, and recording any fee/spread as an _additional_ linked transfer
  (A₁ → L₁) rather than folding it into the rate, so rate and spread remain
  auditable.
- Atomicity is real: linked chains execute in order, each step's effects
  visible to the next, and "either both succeed or both fail"
  (Linked Events / Currency Exchange, single-page docs). A failed leg returns
  its specific error; the other legs return `linked_event_failed`.
- Caveat: **the linkage itself is not persisted.** "After the chain of linked
  events has executed, the fact that they were linked will not be saved. To
  save the association… it must be encoded into the data model, for example by
  adding an ID to one of the user data fields." So the conversion _as an
  event_ (rate, parties, authorization) must be recorded by us, conventionally
  by stamping the same conversion ID into `user_data_128` on both legs.

### What this means for Drew's "authorized conversions"

Drew's example — "ID 123 authorizes ABC→123 at 0.8123 per ABC; ABC sets its
own reverse rate" — maps onto this pattern, but **all of the authorization
semantics are ours**:

- The "liquidity provider" for a user-token pair is whoever holds inventory of
  both tokens — realistically our central finance service, or the issuing ID
  itself. Somebody must _fund_ L₁/L₂; conversions fail (atomically, cleanly)
  when the relevant liquidity account lacks balance. TigerBeetle provides no
  AMM, no order book, no rate discovery, no standing-offer object.
- An "authorization" (rate + who may use it + expiry + caps) is a control-plane
  record. Our API reads it, computes the two amounts, checks the balances/
  invariants, and submits the linked pair. TigerBeetle's System Architecture
  page lists "Apply exchange rates for currency exchange transactions" as a
  duty of the _stateless API service_, confirming the split.
- Each direction is a separate authorized rate, exactly as Drew sketched —
  because each direction is just a separate pair of transfers our service
  chooses to offer.

### Two-phase (pending) transfers — the other native primitive

TigerBeetle _does_ natively support two-phase transfers (Two-Phase Transfers,
single-page docs):

- `flags.pending` creates a transfer that reserves amounts in
  `debits_pending`/`credits_pending` without touching posted balances.
- A second, distinct transfer with `flags.post_pending_transfer` +
  `pending_id` posts all or part of it (posting less than the reserved amount
  restores the remainder); `flags.void_pending_transfer` releases the hold.
- A pending transfer may carry a `timeout` (seconds, relative to cluster time);
  on expiry the full amount is automatically restored.
- A pending transfer resolves **at most once**; double-post/void attempts
  return `pending_transfer_already_posted` / `…_voided` / `…_expired`.
- Balance invariants are checked at _reserve_ time, so a hold that would break
  `debits_must_not_exceed_credits` fails immediately rather than at posting.

This is genuinely useful for the token system: holds for purchases ("reserve
500 premium tokens while the Steam key is validated"), escrowed game stakes,
and the closing-account mechanism (§5). It is a same-ledger primitive; a
two-phase _conversion_ would be two pending flows our service orchestrates,
with the linked-chain atomicity applying within each submission batch.

---

## 3. Per-coin identity: no

The data model has exactly two object types:

> "The TigerBeetle data model consists of `Account`s, `Transfer`s, and
> ledgers." — Data Modeling, [single-page docs](https://docs.tigerbeetle.com/single-page/)

An account's entire state is four 128-bit counters: `debits_pending`,
`debits_posted`, `credits_pending`, `credits_posted`. "TigerBeetle tracks each
account's cumulative posted debits and cumulative posted credits. In
double-entry accounting, an account balance is the difference between the
two." There is:

- **No coin/unit object**, no serial numbers, no UTXO set, no lot/FIFO
  tracking. Transferring 10 of token X from A to B does not identify _which_
  10; units of a ledger are perfectly fungible by construction.
- **No per-unit provenance.** You cannot express "this specific premium token
  came from Steam key K and is revocable if K is refunded" inside TigerBeetle.
  That linkage would live in the control plane (Steam key → issuance transfer
  ID → holder account), and revocation becomes the clawback problem of §5.
- Emulation options, both bad at scale: (a) one account per coin — turns every
  coin into a 128-bit account record in an unsharded cluster and every payment
  into an account-selection problem; (b) treat `user_data_*` on transfers as a
  batch/lot tag — gives lot-level _annotation_, still not enforceable per-coin
  balances. Neither is a documented pattern. **Design assumption should be:
  tokens on TigerBeetle are fungible balances, full stop.** If a game truly
  needs individually-tracked items, those are ID-graph objects, not currency.

---

## 4. Cluster topology — what "3 machines" actually means

### The protocol

TigerBeetle clusters run **Viewstamped Replication (VSR)** — state-machine
replication with a primary that orders operations and backups that replicate
them, deterministic view changes (leader election) on primary failure, and
strict serializability: "to an external observer, [a] TigerBeetle cluster
behaves as if it is just a single machine which processes the incoming
requests in order" (Time / VSR, single-page docs). It uses flexible quorums
(Heidi Howard's research): for the recommended 6-replica cluster, commits need
3/6 replicas but electing a new primary needs 4/6 (Cluster Recommendations;
Safety, single-page docs).

**Correcting Drew's assumption directly:** in a 3-node cluster it is _not_
"only one ever running, others are backups" in the cold-standby sense. All
three replicas run the server continuously, receive the replicated log, apply
the same state machine, and participate in quorum and the fault-tolerant
"cluster time". The primary is a _role_, not a machine class; if it fails, a
view change promotes a backup automatically — provided a quorum survives. If
too many replicas are lost, the cluster "will correctly remain unavailable…
designed to operate correctly or else to shut down safely" rather than risk
inconsistency ([Cluster Recommendations](https://docs.tigerbeetle.com/operating/cluster/)).

### What a 3-replica cluster buys and loses

- A 3-replica cluster is a documented, supported shape (the Deploy page shows
  a 3-replica cluster verbatim) — but the same page states production differs
  in three ways: "**Each replica runs on a dedicated machine. Six replicas are
  used rather than three.** There's a supervisor process to restart a replica
  process after a crash." (Deploy, single-page docs.)
- With 3 replicas, fault tolerance is _one_ replica: the cluster tolerates a
  minority being offline (Jepsen's analysis likewise notes TigerBeetle
  "can continue running safely with a minority of nodes offline" —
  [Jepsen: TigerBeetle 0.16.11](https://jepsen.io/analyses/tigerbeetle-0.16.11)).
  Losing any 2 of 3 — one whole cloud out of three, plus a hiccup, or a cloud
  - a botched upgrade — halts all currency operations until recovery.
- The recommended topology is **6 replicas across 3 sites, 2 per site**, so
  "the loss of an entire site would not impair the availability of the
  cluster", with sites "within a few milliseconds of each other, since each
  transaction must be replicated across sites before being committed"
  ([Cluster Recommendations](https://docs.tigerbeetle.com/operating/cluster/)).
  "One in each cloud" with 3 total replicas gives site-loss _survival of
  data_, but with only 1 replica per site in a 3-node cluster, losing one
  site leaves 2 — available, with zero remaining fault tolerance.
- **Cross-cloud latency is a real cost.** Every commit waits for quorum
  replication. Spreading 3 replicas across three distant cloud providers/EC2
  regions makes every transfer pay inter-provider round trips; the docs'
  guidance is co-located-ish sites (a few ms). Multi-cloud is endorsed for
  the 6-replica shape precisely because flexible quorums let the primary wait
  only for the fastest quorum members.

### Operational requirements (per replica)

From the official Hardware / Deploy docs (single-page):

- **Linux ≥ 5.6 is the only supported production environment** (macOS/Windows
  are development-only).
- One statically-linked binary + one data file per replica; format each data
  file once with matching `--cluster` (128-bit cluster ID), `--replica-count`,
  and a unique `--replica` index; start with the _same ordered_ `--addresses`
  list on every replica and client.
- **Cluster size cannot be changed after creation** in the current version:
  "In the current version of TigerBeetle, cluster size can not be changed
  after creation, but this limitation will be lifted in the future." Choosing
  3 vs 6 is a format-time, effectively permanent decision for that cluster's
  data.
- Each replica's data file on a **separate disk (required) and machine
  (required)**; local NVMe recommended, no RAID required; **ECC memory
  required**; ≥ 6 GiB RAM minimum, 16–32 GiB recommended for cache;
  **a single core per replica** (TigerBeetle is single-threaded by design)
  plus a spare core for the OS; Direct I/O required in production; a
  supervisor (e.g. systemd) to restart crashed replicas.
- Data file preallocates and grows automatically; capacity is substantial but
  finite — docs cite ~16 TiB for ~40 billion transfers, and note ext4's
  16 TiB file limit (XFS supported, less tested).
- Replica _replacement_ after total data-file loss is a sharp edge: Jepsen
  found no documented membership-replacement procedure; the undocumented
  reformat-and-repair path "may be unsafe" in a narrow quorum scenario
  ([Jepsen](https://jepsen.io/analyses/tigerbeetle-0.16.11)). Current CLI does
  include a `recover` command; operating procedure for a dead replica should
  be written and rehearsed before real value sits in the cluster.

**Translation to Drew's sketch:** "three machines, currency ops go to those
EC2s" is buildable and cheap (3 small Linux VMs, 1 core each, NVMe, ECC where
available). But price it as an _always-on consensus cluster that halts if 2 of
3 die_, not as primary+backups — and strongly consider the vendor-recommended
6 replicas / 3 sites if real money (Steam revenue) depends on it. Either way,
_all_ currency traffic funnels to this one cluster: it is a central system by
design, in tension with the ID project's peer-to-peer direction (see §6).

---

## 5. Limits and gotchas

### Scale limits

- **Ledgers:** ≤ 2³²−1 per cluster (u32, non-zero) — not a practical ceiling
  for token count, but ledger IDs are a managed allocation.
- **Accounts & transfers:** IDs are 128-bit, unique _per cluster_ (not per
  ledger). Throughput is the design centre (docs target ~1M transfers/sec via
  batching; max batch 8,189 events per request). The binding constraints for
  us are structural, not TPS:
  - **No sharding.** One cluster holds every ledger, account and transfer.
    TigerBeetle "uses a single-core design" per replica and rejects horizontal
    scaling by design; you scale by bigger/faster single machines, not by
    adding clusters (a second cluster = a second, disconnected money supply
    with cross-cluster conversions becoming an external-systems problem).
  - **Accounts/transfers are never deleted.** Every user-currency account and
    every transfer lives in the data file forever; storage grows monotonically
    (~16 TiB ≈ 40B transfers per docs).
  - **Account history is opt-in.** Historical balances (`get_account_balances`)
    only work for accounts created with `flags.history`; decide at issuance
    which account classes need it (it costs storage).
  - **Queries are narrow.** Lookup by ID, account-transfer listing (history
    flag territory), and `query_accounts`/`query_transfers` filtered by
    `user_data_*`/`code`/timestamps. Anything richer — "all holders of token
    X", leaderboards, issuer dashboards — is control-plane indexing work.

### Idempotency — strong, with one discipline required

- `Account.id` and `Transfer.id` _are_ the idempotency keys: "Events that
  create objects are idempotent. The first event to create an object with a
  given `id` will receive the `ok` result. Subsequent events… will receive the
  `exists` result." Requests execute at most once; clients retry until reply
  (Requests, single-page docs).
- The discipline: **the ID must be generated by the originating client and
  persisted before submission**, then reused on every retry. "The client
  software… should generate the `id` (not your API). This `id` should be
  persisted locally before submission." If the API generates a fresh ID per
  retry, a lost reply becomes a double-spend — TigerBeetle cannot recognise
  the retry as the same operation (Reliable Transaction Submission,
  single-page docs).
- Resubmitting an existing ID with _different_ fields returns
  `exists_with_different_<field>` errors rather than silently succeeding —
  good for detecting bugs, but our API must treat `exists` (identical) as
  success and the `exists_with_different_*` family as a hard error.
- Prefer TigerBeetle's time-based IDs: "Random identifiers are not
  recommended – they can't take advantage of all of the LSM optimizations.
  (Random identifiers have _significantly_ lower throughput than
  strictly-increasing ULIDs)." For a system whose native IDs are Iroh keys,
  the mapping ID-identity → TigerBeetle account ID and → transfer IDs must be
  generated in TB-friendly (monotonic) form by our service, not derived
  randomly per operation.

### Corrections, refunds, revocations — the sharpest mismatch

- **Posted transfers are immutable and undeletable.** "Transfers _cannot be
  modified_ after creation… Transfers _cannot be deleted_ after creation."
  (Transfer reference, single-page docs.) The only "undo" for a _pending_
  transfer is voiding it before it posts.
- **Refunds = correcting transfers.** The documented pattern: submit new
  transfer(s) in the opposite direction, tagged with a dedicated
  `Transfer.code` (e.g. a corrections range) and the same `user_data_128`
  linking them to the original business event. The original, the reversal and
  any re-correction all remain in the permanent record; reporting layers
  "downsample" to the net view (Correcting Transfers, single-page docs).
- **A refund/revocation can fail.** A clawback is an ordinary transfer: if the
  holder's account has `debits_must_not_exceed_credits` (as user accounts
  should) and they have already spent or moved the tokens, the revocation
  transfer is _rejected_ (`exceeds_credits`). TigerBeetle offers no override,
  no freeze flag that seizes funds, and no negative-balance forcing for
  constrained accounts. Practical mitigations are all design choices we must
  make up front:
  - Delay finality: deliver redeemable value via _pending_ transfers with a
    timeout window (voidable while the Steam refund window is open) — fits
    §2's two-phase primitive, at the cost of delayed usability.
  - Keep premium-token accounts _unconstrained_ and accept negative balances
    as debt records — weakens the core invariant for exactly the accounts
    that matter most.
  - Control-plane holds/freezes: our API refuses new transfers from a flagged
    account (TigerBeetle itself will still happily process anything our API
    submits — enforcement is only as strong as our gateway).
- **Account closure exists but is narrow.** An account can be closed (via
  `flags.closing_debit`/`closing_credit` pending transfers) once its net
  balance is zeroed; a closed account "will reject further transfers, except
  for voiding two-phase transfers that are still pending", and closure is
  reversible by voiding the closing pending transfer (Closing Accounts,
  single-page docs). Useful for retiring a token (zero everyone out, close
  issuer accounts) — but _retiring a currency_ has no primitive: the ledger,
  its accounts and history persist forever; "sunsetting" is a control-plane
  flag that stops our API honouring it.
- **No built-in supply audit.** Proving "total issued = Σ balances" is a
  query/reporting job over the issuer account's counters plus holder accounts;
  feasible (issuer debits/credits are exact) but it's our report to build.

### Security boundary

- No authentication, no authorization, no per-user permissions, no TLS-level
  user identity in the database itself; the docs warn untrusted processes must
  not reach the server _or its data file_. Every ID signature check, issuance
  permission, conversion authorization and abuse control (rate limits, blocks)
  sits in the gateway service. (TigerBeetle can, amusingly, _implement_ rate
  limiting via its leaky-bucket recipe — pending transfers on a dedicated
  ledger — but the decision to invoke it remains ours.)

---

## 6. Fit against Drew's design, point by point

1. **"Any ID issues tokens of itself, on the fly"** — ✅ mechanically (new
   ledger + issuer account), provided our service allocates ledger IDs,
   registers name/scale/issuer in the control plane, and verifies the ID's
   signed issuance statement before minting transfers.
2. **"Transactions in the ID graph"** — ⚠️ dual-write reality. TigerBeetle
   transfers are the balance truth for centralized tokens; graph objects are
   the identity/authorization truth. Link via `user_data_128`; design
   reconciliation. TigerBeetle contributes nothing graph-shaped (Jepsen's
   summary: "no way to represent arbitrary rows, objects, graphs, blobs" —
   [Jepsen](https://jepsen.io/analyses/tigerbeetle-0.16.11)).
3. **"Central issuer / issuer snapshots instead of a blockchain"** — ✅ this
   is exactly TigerBeetle's shape: one logical, strictly-serializable central
   ledger with an immutable, totally-ordered history. "Snapshots" of an
   issuer = reads of account counters/history at a timestamp; export/reporting
   is our tooling.
4. **"Authorized conversions at issuer-set rates"** — ⚠️ pattern exists
   (linked pairs through liquidity accounts), but rates, authorization,
   inventory funding, rounding and fees are our service's code (§2).
5. **"Tokens buy real services (Steam key → 500 premium tokens)"** — ✅ as
   mint-on-redemption transfers, with the refund caveat: Steam refunds map to
   correcting/clawback transfers that _can fail after spend-down_; the
   pending-transfer hold pattern is the cleanest mitigation and should be in
   the design from day one.
6. **"One TigerBeetle cluster, 3 machines across clouds"** — ⚠️ buildable,
   but re-scope expectations: always-on VSR replicas, halt-on-2-failures,
   commit latency = cross-cloud RTT, vendor recommendation is 6 replicas /
   3 sites, cluster size frozen at format time, Linux/NVMe/ECC/single-core
   per replica.
7. **"Open source everything"** — ✅ TigerBeetle itself is Apache-2.0 open
   source with official clients (.NET, Go, Java, Node.js, Python, Rust, Ruby);
   no licensing obstacle found in the docs.
8. **Interaction with the Cloudflare/isolates direction (LIST 12)** — ❌
   tension to flag: TigerBeetle is a stateful consensus cluster needing
   Direct I/O, local disk and persistent processes. It cannot run in Workers
   isolates/Durable Objects. The serverless ID edition would need either a
   hosted TigerBeetle reachable over the network (our API service in front of
   the cluster — fine) or a different, weaker ledger for that tier. Currency
   is inherently the centralised exception in the ID architecture; say so in
   the design rather than discovering it later.

## 7. What we would still have to build (the actual project)

1. **Currency registry (control plane DB):** ledger allocation, name/symbol,
   issuer ID, asset scale, supply policy, status (active/frozen/sunset).
2. **Account factory:** per currency — issuer, liquidity (per conversion
   pair/direction), fee, burn accounts; per user — holder accounts with
   `debits_must_not_exceed_credits`, history flags where needed.
3. **Gateway API:** the _only_ TigerBeetle client; verifies ID signatures,
   issuance/conversion authorizations, abuse controls; generates/persists
   idempotency IDs correctly; batches transfers.
4. **Conversion engine:** rate records, amount computation + rounding policy,
   liquidity inventory management and rebalancing, linked-pair submission,
   conversion event records (rate/authorization snapshot at execution time).
5. **Corrections service:** refund/revocation workflows as correcting
   transfers with dedicated codes; spend-down failure handling; pending-hold
   redemption for refundable purchases (Steam).
6. **Reporting/reconciliation:** supply audits, holder listings, ID-graph ↔
   TigerBeetle consistency checks, issuer dashboards.
7. **Cluster operations:** provisioning (3 or 6 replicas), supervision,
   upgrades (multiversion binaries exist; procedure still ours), dead-replica
   replacement runbook, backups/exports of the control plane (TigerBeetle's
   durability does not cover the registry DB).

## 8. Open questions for the design phase

- 3 replicas (cheap, fragile) vs the recommended 6 across 3 sites — decide
  before formatting; the size cannot be changed afterwards.
- Which tokens are TigerBeetle-backed (central issuer) vs graph-only claims?
  Drew's model already implies both exist; the boundary is a design decision,
  not a TigerBeetle feature.
- Redemption finality policy for refundable purchases: pending-hold window
  length vs instant spendability.
- Ledger-ID governance: who allocates, how are IDs recycled (never?), what
  happens at tenant × currency cross-products.
- Naming: none of "id bucks / d3v / dt / 2d / d2d" is representable in
  TigerBeetle (names are control-plane strings) — the name decision is
  unconstrained by the backend.

## Sources

- TigerBeetle docs — Currency Exchange recipe: https://docs.tigerbeetle.com/coding/recipes/currency-exchange/
- TigerBeetle docs — Cluster Recommendations: https://docs.tigerbeetle.com/operating/cluster/
- TigerBeetle docs — single-page edition (System Architecture, Data Modeling,
  Requests, Reliable Transaction Submission, Two-Phase Transfers, Linked
  Events, Time, Correcting Transfers, Closing Accounts, Deploy, Hardware,
  Account/Transfer references, Safety): https://docs.tigerbeetle.com/single-page/
- Jepsen — TigerBeetle 0.16.11 analysis (independent; data model limits,
  minority fault tolerance, replica-replacement gap): https://jepsen.io/analyses/tigerbeetle-0.16.11
- TigerBeetle Go bindings / DeepWiki (Account/Transfer field types — `ledger`
  u32 non-zero, `code` u16, amounts u128): https://deepwiki.com/tigerbeetle/tigerbeetle-go/4.2-account
