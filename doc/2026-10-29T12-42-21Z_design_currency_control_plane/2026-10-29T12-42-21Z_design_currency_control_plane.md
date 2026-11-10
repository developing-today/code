# Currency control plane — design: registry, account factory, gateway

**Date:** 2026-10-29 · **Issue:** [#2062](https://github.com/developing-today/code/issues/2062) (ID program wave-1, LIST 11)
**Status:** draft for review — this is a design pass, not an implementation. Nothing here is
built; the buildable pieces are filed as follow-up issues (§10).
**Feeds:** the currencies v1 synthesis [#2043](https://github.com/developing-today/code/issues/2043).
**Siblings that build on this:** conversion engine [#2063](https://github.com/developing-today/code/issues/2063),
hosted-edition boundary [#2066](https://github.com/developing-today/code/issues/2066).
**Sources:** the TigerBeetle investigation
([`doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/`](../2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend.md))
and the Cloudflare Workers fit investigation
([`doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/`](../2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit.md)).

---

## TL;DR

TigerBeetle is the settlement engine and **owns nothing that makes a currency a
currency**: no name, no symbol, no scale, no issuer, and — critically — **no
authentication at all**. A currency is a bare `u32` `ledger` value on accounts
and transfers, and any process that can reach the cluster can move any money on
any ledger.

So the control plane is not an add-on; it is *the product*. It has three parts:

1. **Currency registry** — a control-plane database that allocates `ledger`
   IDs and holds the metadata TigerBeetle refuses to: name, symbol, scale,
   issuer, supply policy, status. It is the authority for *which u32 means
   what*, and it is the only thing standing between "a u32" and "a currency".
2. **Account factory** — a deterministic scheme that maps an ID identity to a
   TigerBeetle account, plus the per-currency system accounts (issuer,
   liquidity, fee, burn). Account IDs are *derived*, not allocated, so account
   creation is idempotent and needs no coordination round-trip.
3. **Gateway service** — the **only** TigerBeetle client. It verifies the
   caller's ID signature, checks issuance/transfer/conversion permission
   against the registry, enforces abuse controls, and only then submits a
   batch. TigerBeetle's lack of auth is not a gap we paper over; the gateway
   *is* the auth boundary, and the threat model (§1) is the design.

The sharp edges, stated up front:

- **One gateway, or none.** Two processes with a TigerBeetle client key is two
  authorities. The gateway must be a single logical writer (an HA deployment of
  one service, not a fleet of peers that each hold the client key), or the
  registry checks are advisory.
- **The registry is a second source of truth and can drift.** The ledger is
  balance truth; the ID graph is identity truth; the registry is *metadata*
  truth. §7 designs the linkage (`user_data_128` + a signed graph object per
  currency) and the reconciliation job, because "the DB and the ledger
  disagree" is a failure mode we must be able to detect, not one we can assume
  away.
- **Scale is ours and is effectively immutable.** TigerBeetle has no scale
  field; a currency's scale lives only in the registry, and changing it after
  balances exist is a migration, not an update.
- **Ledger IDs are finite and never recycled.** `u32` is large but shared with
  every other use of ledgers; §6 sets a governance rule before we spend any.

---

## 0. Constraints inherited from the backend

These are facts from the TigerBeetle investigation, restated because the whole
design is a response to them:

| Fact | Consequence for this design |
| --- | --- |
| `ledger` is a bare non-zero `u32`; no metadata, no DDL, no creation call | The registry *is* the meaning of a ledger. Allocation is application-side. |
| No authentication, authorization, or user identity anywhere in the DB | The gateway is the sole auth boundary; TigerBeetle must be network-isolated (§1). |
| Cross-ledger transfers are forbidden | A "currency" is exactly one ledger; conversions are app-composed pairs (#2063), not a gateway primitive. |
| Posted transfers are immutable and undeletable | Corrections are compensating transfers; the gateway's write path is append-only. |
| Amounts are `u128`; `code` is `u16`; `user_data_128` is free for us | Amount/scale validation is ours; `code` and `user_data_128` are our linkage channels. |
| Idempotency is client-generated 128-bit IDs, deduplicated by the cluster | The gateway owns ID generation and the replay ledger (§4). |
| Accounts carry flags fixed at creation (`debits_must_not_exceed_credits`, `history`, …) | Flags are a design decision at factory time; they cannot be relaxed later. |
| Pending (two-phase) transfers exist with timeouts | Available for refundable redemption (#2065), not needed for the basic path. |

---

## 1. Threat model

The gateway is the single point where "a signed intent" becomes "a ledger
write". The model is written around that fact.

### 1.1 Assets

- **Balances** — the TigerBeetle ledger is the balance truth and is the thing an
  attacker wants to move.
- **Issuance authority** — the right to mint a currency; the most valuable
  privilege in the system (mint is unbounded supply).
- **Registry records** — ledger↔currency mapping, issuers, supply caps, status.
  Corrupting these redirects value.
- **Client keys (ID private keys)** — held by users/issuers, never by the
  gateway.
- **The gateway's TigerBeetle client key** — the cluster accepts anything it
  sends; this key is effectively root on all balances.

### 1.2 Trust boundaries

```
  user / app  ──(untrusted, signed request)──▶  GATEWAY  ──(trusted, no auth)──▶  TigerBeetle
                                                  │
                                                  └──▶  control-plane registry DB
  ID graph (signed objects) ──read──▶ GATEWAY  (identity/authorization truth)
```

- **Untrusted:** every client, every signed request, every graph object. A
  valid signature proves *who*, never *what they may do*.
- **Trusted:** the gateway process, its registry DB, and the network segment
  TigerBeetle sits on. Trust here is assumed, not verified, and is therefore
  *bounded* by §1.4's isolation requirement.

### 1.3 Adversaries and what the gateway must do

| Adversary action | Gateway control |
| --- | --- |
| Forge an issuance (mint without authority) | Verify the ID signature, then check the caller against the registry's issuer/minter set **and** the supply policy. Signature alone is never sufficient. |
| Replay a captured signed request | Deterministic idempotency ID + a replay ledger; TigerBeetle's dedup is the backstop, not the primary check. |
| Double-submit to race a balance check | The balance check and the write are one submitted batch; `debits_must_not_exceed_credits` makes the overspend fail atomically at the cluster. Never "check then write". |
| Reach TigerBeetle directly and skip the gateway | Network isolation: no public listener, mTLS/allow-list between gateway and replicas, replicas on a private segment. This is the control that makes "the gateway is the boundary" true rather than aspirational. |
| Steal a user's ID key | Out of scope for the gateway (no key custody); mitigated by ID-graph revocation and per-request nonce windows. |
| Compromise the gateway | The gateway holds no user keys, but it does hold the cluster client key. Mitigation is operational (§1.4), not cryptographic. |
| Malicious/greedy issuer over-issuing | Supply policy (`cap`, `mintable`), revocable minter grants, and a supply audit (§7.3). |
| Corrupt registry metadata | Registry writes are themselves signed by an authorized ID and recorded in the graph (§2.4); the DB is a derived index. |

### 1.4 Non-goals and operational bounds

- **Not** protecting against a compromised gateway host. The mitigation is
  operational: the client key lives only on the gateway, the cluster is
  unreachable otherwise, and the registry DB is backed up and append-audited.
- **Not** confidentiality of ledger contents. TigerBeetle data is plaintext on
  disk; treat balances as public-to-operators.
- **Not** a defense against a malicious *cluster operator*. Currency is the
  centralised exception in an otherwise serverless ID architecture
  ([#2066](https://github.com/developing-today/code/issues/2066)); centralisation
  means one operator is trusted.

---

## 2. Currency registry

A control-plane database (Postgres or libsql — the same store the ID server
already runs; the choice is a follow-up, not a design blocker) holding the
metadata TigerBeetle does not.

### 2.1 `ledger_allocation` — the u32 allocator

| column | type | notes |
| --- | --- | --- |
| `ledger_id` | `integer` (u32, PK) | the value written into TigerBeetle. |
| `kind` | `text` | `currency` \| `system` \| `tenancy` — why this u32 is spent. |
| `owner_ref` | `text` | currency id, system-account purpose, or tenant id. |
| `allocated_at` | `timestamptz` | |

Rules:

- IDs are handed out by a **monotonic counter, never recycled** (§6).
- Reserving a range for `tenancy` up front keeps tenant partitioning from
  colliding with currencies later.
- `ledger_id = 0` is invalid in TigerBeetle and never allocated.

### 2.2 `currency` — the currency record

| column | type | notes |
| --- | --- | --- |
| `ledger_id` | PK, FK → `ledger_allocation` | the ledger this currency is. |
| `currency_id` | `text` | stable, human-typable id (e.g. `abc`) used in graph objects and APIs. |
| `name` | `text` | display name. Free-form; not unique. |
| `symbol` | `text` | short ticker. Unique **per issuer**, not globally (§2.3). |
| `scale` | `smallint` | decimal places. Immutable once balances exist (§0). |
| `issuer_id` | `text` | the ID that owns issuance. |
| `supply_policy` | `jsonb` | `{ mintable: bool, cap: u128\|null, policy_uri: text\|null }`. |
| `status` | `text` | `active` \| `frozen` \| `sunset`. `frozen` = gateway refuses new writes; `sunset` = no new writes, redemption window only. |
| `graph_object_id` | `text` | the signed graph object that is the currency's canonical record (§2.4). |
| `created_at` / `updated_at` | `timestamptz` | |

### 2.3 Identity of a currency

- **The ledger u32 is the mechanical identity** — it is what every transfer
  references.
- **`currency_id` is the semantic identity** — what users type and what graph
  objects reference. It is minted by the registry at registration and never
  changes.
- **`symbol` is a display label, unique per issuer** (the issuer's namespace),
  so two issuers may both call their token `ABC` without ambiguity — the
  `(issuer_id, currency_id)` pair disambiguates, exactly as the investigation
  predicted ("the name decision is unconstrained by the backend").
- **`name`/`symbol`/`scale` are registry strings.** None of them is representable
  in TigerBeetle, and none of them is needed there: a client resolves
  `currency_id → ledger_id` through the gateway before any transfer.

### 2.4 The graph is the canonical record; the registry is a derived index

Following the ID trust model
([#2030](https://github.com/developing-today/code/issues/2030): servers accept
all signed objects; views count allowed writers), a currency's authoritative
definition is a **signed graph object** — the issuer's signed claim "I created
currency `X` with this ledger, this scale, this supply policy". The registry row
is an index over that object, holding `graph_object_id` and enough to answer
`currency_id → ledger_id` without a graph round-trip.

Consequence: if the registry DB is lost, it is **rebuildable** by replaying the
graph; the ledger itself is never rebuilt (it is the balance truth, and the
registry never holds balances).

### 2.5 Registry writes are signed too

Registering a currency, granting mint rights, freezing, and sunsetting are
themselves signed operations routed through the gateway (§4) — not direct DB
writes — so every metadata change has the same signature trail as a transfer
and lands in the graph. The DB is never edited by hand in production.

---

## 3. Account factory

TigerBeetle accounts are created explicitly with a 128-bit `id` and a `ledger`,
and their **flags are fixed at creation**. The factory's job is to make account
identity *derivable* and account creation *idempotent*.

### 3.1 Deterministic account IDs

```
account_id = BLAKE3( domain_sep ‖ ledger_id(u32, LE) ‖ kind(u8) ‖ owner_id(len-prefixed UTF-8) ‖ salt(len-prefixed) )[0..16]
```

- **Derived, not allocated.** Any component can compute the account ID for
  `(currency, owner, kind)` without asking the registry, which is what makes
  creation idempotent and the factory stateless.
- **`owner_id`** is the **account id** (§4.1b) — a holder is an account, not a
  device; device keys act on the account's behalf.
- **`salt`** exists only for the rare case of multiple accounts of one kind per
  owner (e.g. per-game escrow); default empty.
- The domain separator (`"id-currency/account/v1"`) is versioned so a future
  scheme can coexist.

Creation is a `create_accounts` call; a duplicate returns the cluster's
"exists" result, which the factory treats as success. **No read-modify-write,
no lock.**

### 3.2 The per-currency account set

| account | kind | owner | flags | purpose |
| --- | --- | --- | --- | --- |
| **Issuer** | `issuer` | `currency.issuer_id` | unconstrained (issuer balance may go negative = net issued) | source of every mint; its counters are the supply audit basis. |
| **Liquidity** | `liquidity` | liquidity provider id | unconstrained | one per (currency, direction) for conversion legs (#2063). |
| **Fee** | `fee` | platform | `debits_must_not_exceed_credits` | collects transfer/conversion fees. |
| **Burn** | `burn` | platform | `debits_must_not_exceed_credits` | destination for retirement; balances here are out of circulation. |
| **Holder** | `holder` | any ID | `debits_must_not_exceed_credits`; `history` where audit matters | one per (currency, user). No overdraft — this is the double-spend guard. |

Two deliberate choices:

- **Holder accounts are constrained** (`debits_must_not_exceed_credits`), so an
  overspend is rejected *by the cluster* even if the gateway's logic is wrong.
  The gateway does not get to be the only thing preventing negative balances.
- **The issuer account is deliberately unconstrained**, because issuance is
  modelled as a debit from the issuer — "money out of thin air" is exactly a
  negative issuer balance, and its magnitude is the circulating supply. A supply
  cap is therefore a *registry* check, not a ledger invariant (TigerBeetle
  cannot cap an unconstrained account's negativity in currency terms).

### 3.3 Factory API

```
ensure_account(currency_id, owner_id, kind, salt = "") -> account_id
ensure_currency_accounts(currency_id) -> { issuer, fee, burn, liquidity[] }
```

Both are idempotent and safe to call on every write path. The gateway calls
`ensure_account` lazily before the first transfer involving a new holder, and
`ensure_currency_accounts` at registration.

---

## 4. Gateway service

The gateway is the only process that holds a TigerBeetle client key. Its API is
a small set of **signed operations**, and its write path is a fixed pipeline.

### 4.1 Signed request envelope — reuse the existing scheme

The ID codebase **already signs requests**, and the gateway must adopt that
scheme rather than invent a second one. `pkgs/id/src/directory_auth.rs` defines

```
Signed { key, at, nonce, signature }
canonical message = "id-request-v1\n{method}\n{target}\n{at}\n{nonce}\nsha256(body)"
```

verified with `verify_strict` under a 5-minute clock window (`SIGNED_WINDOW_MS`)
and a `ReplayGuard` that refuses a replayed nonce (`directory_auth.rs:241-354`).
A currency request is exactly that envelope with an op-specific body:

```
CurrencyRequest {
  op:   register_currency | grant_minter | issue | transfer | convert
      | balance | history | freeze | sunset,
  body: canonical CBOR, domain-tagged "id.currency.request.v1",
}
signed as Signed { key, at, nonce, signature }
```

- **Canonicalisation is part of the protocol.** Follow the repo's event
  convention: canonical CBOR with a domain tag, and decoders that *refuse*
  non-canonical input (`chain.rs:30` tag `id.events.v1`; `chain.rs:321-342`
  `decode` rejects non-canonical CBOR). The bytes signed must be byte-for-byte
  reproducible by both sides; an ambiguous encoding is a forgery surface.
- **Use `verify_strict`, never `verify`.** The repo carries forgery regression
  fixtures precisely because plain `verify` accepts signatures `verify_strict`
  refuses (`src/test_forgeries.rs:18-56`). The gateway inherits that rule.
- **`nonce` + `at`** give a cheap replay window (the existing 5-minute skew and
  `ReplayGuard`); the deterministic idempotency ID (§4.3) is the durable guard
  behind it.

### 4.1b Whose key signs

Two key layers already exist, and the choice decides what "issuer" means:

- **Device/node key** — the Iroh Ed25519 node key, 64 hex chars, the `id id`
  output, per device (`pkgs/id/src/lib.rs:17-18`, `commands/id.rs:11-14`).
- **Account key** — derived, `SigningKey = SHA-256("id-account-v1" ‖ secret)`,
  with a credential string `acct.<id>.<secret>` shown once and only
  `sha256(secret)` kept server-side (`directory.rs:564-580`). Devices are
  attached to an account by signed `device.add` / `device.revoke` events
  (`groups/mod.rs:126-147`).

**Issuance authority binds to the account key, not the device key.** A currency
is owned by an account; a device signs on the account's behalf; `device.revoke`
must be able to withdraw a lost device's minting power *without moving the
currency*. `register_currency` and `grant_minter` therefore record account ids,
and the gateway resolves device → account before the authorization check in
§4.2.

### 4.2 Authorization: signature is *who*, registry is *what*

After signature verification, every operation is checked against the registry
**before** any ledger call:

| op | authorization check |
| --- | --- |
| `register_currency` | caller's ID is the claimed `issuer_id`; `currency_id`/`ledger` allocation succeeds; symbol free in that issuer's namespace. |
| `grant_minter` | caller is the currency's `issuer_id`; writes `currency_issuer(role=minter)`. |
| `issue` (mint) | caller ∈ `{issuer_id} ∪ minters` for the currency; `supply_policy.mintable`; projected supply ≤ `cap`. |
| `transfer` | caller controls `from` (is its `owner_id`, or holds a delegation); `from`/`to` same currency; currency `active`. |
| `convert` | delegated to the conversion engine's rate authorization (#2063); the gateway enforces the same signature + nonce envelope. |
| `balance`/`history` | caller may read its own accounts; operator/issuer reads gated by role. |
| `freeze`/`sunset` | caller is `issuer_id` or platform operator. |

**A valid signature with no matching registry entry is rejected.** This is the
single most important rule: the registry, not the signature, decides authority.

### 4.3 The write path (fixed order)

1. **Verify** the envelope signature and freshness (nonce window).
2. **Derive the idempotency ID** deterministically:
   `idem = BLAKE3(caller_id ‖ op ‖ client_request_id)[0..16]`, where
   `client_request_id` is a client-chosen 128-bit value (or a client monotonic
   counter). Store it in the gateway's replay ledger. TigerBeetle's own dedup is
   the backstop for a crash between submit and record.
3. **Resolve** `currency_id → ledger_id`, scale, status via the registry.
4. **Validate** the amount against `u128` and the currency's `scale`.
5. **Authorize** per §4.2 (issue: minter set + cap; transfer: ownership).
6. **Apply abuse controls** — per-counterparty budgets and rate limits
   ([#2032](https://github.com/developing-today/code/issues/2032),
   [#2033](https://github.com/developing-today/code/issues/2033)). A rejected
   request never reaches the ledger.
7. **Build the batch** — `ensure_account` for any new holders, then the
   transfer(s) with the idempotency ID, `code` from a reserved transfer-code
   range, and `user_data_128` = the ID of the signed graph object recording this
   business event (§7).
8. **Submit once.** The balance check and the write are the same submitted
   batch; the cluster's atomic rejection is the double-spend guard.
9. **Record and return** a **signed receipt** (gateway key) carrying the
   idempotency ID, the ledger transfer IDs, and the graph object id. The receipt
   is what a client or auditor reconciles against.

Steps 1–6 are pure functions of the request + registry snapshot and can be
unit-tested without a cluster; step 7–8 is the only part that needs TigerBeetle.

### 4.4 Transfer codes (`code: u16`)

A reserved, documented range so the ledger history is self-describing:

| range | meaning |
| --- | --- |
| `0x0001–0x00FF` | issuance (mint) |
| `0x0100–0x01FF` | holder transfer |
| `0x0200–0x02FF` | conversion leg (paired; #2063) |
| `0x0300–0x03FF` | fee |
| `0x0400–0x04FF` | correction / refund |
| `0xF000–0xFFFF` | reserved for pending-hold redemption (#2065) |

### 4.5 One gateway, HA as a service

"Only the gateway may write" is only true if there is one authority. The gateway
must therefore be deployed as **a single logical writer** — an HA *service*
(leader election, or a single active with a hot standby that shares no client
key) — **not** as a horizontally scaled fleet where every replica holds the
TigerBeetle client key. A fleet of writers makes the registry checks advisory,
because two replicas can each pass a cap check and both mint.

### 4.6 Reuse, don't reinvent

Everything the envelope needs already exists in `pkgs/id`:

| need | existing code |
| --- | --- |
| envelope, canonical message, clock window, replay guard | `src/directory_auth.rs` (`Signed`, `sign_request`, `verify_signed`, `ReplayGuard`) |
| canonical CBOR with domain tags and strict decoding | `src/chain.rs` (`id.events.v1`; `decode` refuses non-canonical input) |
| `verify_strict` discipline + forgery regression fixtures | `src/test_forgeries.rs` |
| account/device key derivation and delegation | `src/directory.rs`, `src/groups/mod.rs` |
| signed-object recording | `src/artifact.rs`, `src/chain.rs` |

The only *new* pieces are the per-op body encoders (domain-tagged CBOR), the
account-ID derivation (§3.1), and the idempotency-ID derivation (§4.3). Put
those in one Rust crate/module (`id-currency-wire`, alongside `pkgs/id`) so the
gateway and every client share them; nothing reimplements the encoding.

### 4.7 Gateway placement

The existing `id serve --web` axum server already gates writes behind a signed
request: `WorldHub::verify_signed` runs signature + window + replay checks
before a write (`src/web/world_hub.rs:415-458`), with the router at
`src/web/routes.rs:253-288`. The gateway's request pipeline can be a route
group there, reusing that gate — **but only while it stays a single logical
writer** (§4.5). The horizontally scaled web server must not each hold a
TigerBeetle client key, so the ledger-write path is its own deployment (one
active writer; a standby holds no client key), even if the registry API shares
the web server's process. This is the one place where the "just add a route"
instinct must be resisted.

---

## 5. Write path walkthrough

A user issues 100 of a currency, then sends 40 to a friend:

```mermaid
sequenceDiagram
    participant C as Client (ID key)
    participant G as Gateway
    participant R as Registry DB
    participant T as TigerBeetle
    participant X as ID graph

    C->>G: SignedRequest{op:issue, currency, amount:100, nonce, sig}
    G->>G: verify ed25519 + nonce window
    G->>R: caller is issuer/minter? mintable? cap ok?
    R-->>G: allow
    G->>T: create_accounts(issuer, holder); transfer(issuer→holder, code=mint, idem)
    T-->>G: committed (or idempotent no-op)
    G->>X: signed object "issued 100 to <holder>"
    G-->>C: signed receipt{transfer_ids, graph_object_id}

    C->>G: SignedRequest{op:transfer, from:self, to:friend, amount:40, sig}
    G->>G: verify sig; caller owns `from`
    G->>T: ensure_account(friend); transfer(self→friend, code=transfer, idem)
    T-->>G: committed
    G->>X: signed object "transferred 40 to <friend>"
    G-->>C: signed receipt
```

A transfer that would overdraw the sender is rejected **by the cluster**
(`debits_must_not_exceed_credits`), and the gateway returns the cluster's
rejection as a typed error — never as a silent success.

---

## 6. Ledger-ID allocation and scale

- **One ledger per currency, forever.** A currency is a ledger; the registry
  maps `currency_id ↔ ledger_id`.
- **Monotonic, never recycled.** Recycling a ledger would silently attach a new
  currency to old history. Exhaustion (2³²−1) means a new cluster, not reuse.
- **Reserve ranges now.** Partition the space before spending it, e.g.
  `0x00000001–0x000FFFFF` tenancy/system, `0x00100000–…` currencies. The exact
  split is a follow-up decision, but the *rule* — no recycling, documented
  ranges — belongs in the design.
- **Scale is registry-only and immutable in practice.** Changing a currency's
  scale after balances exist reinterprets every stored integer; treat it as a
  migration (new ledger, redeem-and-reissue), not an `UPDATE`.

Cluster size (3 vs the recommended 6 replicas) is [#2064](https://github.com/developing-today/code/issues/2064)'s
decision and is orthogonal to the control plane; the gateway is agnostic to it.

---

## 7. Consistency: graph ↔ registry ↔ ledger

Three stores, three truths. The design keeps them linked and *checkable*.

### 7.1 Linkage

- **Ledger ↔ graph:** every gateway write stamps `user_data_128` with the ID of
  the signed graph object recording the business event. The ledger row and the
  graph object point at each other.
- **Registry ↔ graph:** `currency.graph_object_id` points at the currency's
  signed creation object.
- **Registry ↔ ledger:** `ledger_allocation` is the only place the u32 is
  assigned; the gateway resolves through it.

### 7.2 What can drift, and what it costs

| drift | detection | repair |
| --- | --- | --- |
| Registry row lost/corrupted | rebuild from graph objects (§2.4) | replay graph → registry |
| Graph object missing for a ledger write | reconciliation job: every transfer's `user_data_128` resolves to an object | re-anchor from the signed receipt |
| Ledger and graph disagree on an amount | reconciliation compares transfer counters to graph events | ledger wins (balance truth); flag the graph object for review |
| Two registry rows for one `currency_id` | uniqueness constraint + signed-write path | reject at write time |

### 7.3 Reconciliation and supply audit

A scheduled job (follow-up F4) that:

1. sums issuer-account debits per currency and compares to
   `sum(holder credits) + fee + burn` — the supply audit;
2. resolves each recent transfer's `user_data_128` to a graph object;
3. reports mismatches to an operator surface.

Feasible precisely because the issuer account's counters are exact and the
history flag can be set where it matters.

---

## 8. Abuse cases walked through

1. **Forged issuance.** Mallory signs `issue` with her own ID for someone else's
   currency. Signature verifies; registry says she is not in the minter set →
   rejected. (Signature is who, registry is what.)
2. **Replayed mint.** Eve captures a valid `issue` request and resubmits it.
   Nonce window rejects it immediately; if it slips through, the deterministic
   idempotency ID makes the second submission a cluster no-op.
3. **Double-spend race.** Two concurrent `transfer`s each of the full balance.
   Both are submitted batches against a constrained holder account; one commits,
   the other is rejected by the cluster. No check-then-write window exists.
4. **Direct cluster access.** An attacker who can reach the TigerBeetle port
   bypasses every check. Control: the cluster listens only on the private
   gateway segment, with mTLS/allow-list — the same "untrusted processes must
   not reach the server *or its data file*" rule the investigation quoted.
5. **Greedy issuer.** The issuer mints past a promised cap. Registry cap check
   rejects it; the audit (§7.3) shows any breach of a *lower* real-world promise
   (e.g. an off-chain commitment) as a policy matter, since the ledger cannot
   enforce a cap it was never told.
6. **Registry DB compromise.** Attacker edits a row to point a currency at a
   different ledger. Because registry writes go through the signed gateway path
   (§2.5) and each row carries `graph_object_id`, the forgery is detectable
   against the graph; production never edits the DB by hand.
7. **Spam / griefing.** Per-counterparty budgets and rate limits
   ([#2032](https://github.com/developing-today/code/issues/2032)/[#2033](https://github.com/developing-today/code/issues/2033))
   sit before step 7 of the write path; TigerBeetle's leaky-bucket recipe is an
   optional backstop, but the decision to invoke it is ours.

---

## 9. Open questions

1. **Control-plane store:** Postgres vs libsql (what the ID server already
   runs) — pick for the hosted edition in [#2066](https://github.com/developing-today/code/issues/2066).
2. **Gateway placement:** a new binary vs a route group in `id serve` (§4.7),
   and the single-writer deployment (leader election over a shared client key
   vs one active + keyless standby).
3. **Idempotency ID vs the TigerBeetle monotonic-ID guidance.** TigerBeetle's
   reliable-submission pattern wants client-generated, persisted, *monotonic*
   128-bit IDs; graph object ids are BLAKE3 hashes (non-monotonic). This design
   derives the transfer/idempotency ID by hashing (stateless, stable) and leans
   on the cluster's dedup as the backstop. Decide in F3 whether that is enough
   or a gateway monotonic sequence with durable counter state is required. Note
   this is a *different* id from `user_data_128`, which carries the **graph
   object id** and must be a hash to be content-addressed.
4. **Key type for the gateway's TigerBeetle client:** one key or per-operator
   keys; rotation procedure.
5. **Ledger range split:** the tenancy/currency/system boundaries (§6).
6. **Issuer-account modelling:** unconstrained issuer (chosen here) vs an
   explicit supply ledger. The chosen model makes the audit a counter read but
   puts the cap in the registry, not the ledger — worth a second look in #2043.
7. **Tenant × currency cross-product:** does a tenant get its own currency
   namespace, or share the global one? Interacts with §6's ranges.

---

## 10. Follow-up work (buildable)

Filed as issues linked from #2062; each is a slice of the above, not the whole
feature, and the PoC (F5) is explicitly a PoC that makes its own decisions and
must be reviewed before anything permanent is built on it.

| # | slice | depends on |
| --- | --- | --- |
| F1 | Currency registry schema + migrations (`ledger_allocation`, `currency`, `currency_issuer`, `account`) and the ledger allocator | — |
| F2 | Account factory: deterministic account IDs, `ensure_account`, `ensure_currency_accounts`, transfer-code table | F1 |
| F3 | Gateway envelope: canonical encoding, ed25519 verify, nonce window, idempotency ID derivation, typed errors | — |
| F4 | Reconciliation + supply-audit job (issuer counters vs holders; `user_data_128` → graph object) | F1, F2 |
| F5 | PoC: registry + factory + gateway over a single-currency end-to-end issue→transfer, with the graph object and signed receipt | F1–F3 |

F5 is a proof of concept: it should demonstrate the pipeline and surface
unknowns, **not** be promoted into the permanent currency service without a
review round. Gated syntheses (the currencies v1 design [#2043](https://github.com/developing-today/code/issues/2043))
stay gated on their inputs and are not started here.

**This design unblocks [#2298](https://github.com/developing-today/code/issues/2298)** —
the gated currency PoC, whose gate reads "do not start until … the #2062
control-plane design fixes the gateway's registry/account-factory shape". §2, §3
and §4 now fix that shape, so that gate is satisfied and the PoC may be picked
up (either after the slices above, or directly against this document's
interfaces). It also sits alongside [#2066](https://github.com/developing-today/code/issues/2066)'s
centralised-exception boundary design (its own doc on PR #2297), which this
document does not duplicate: it supplies the registry/factory/gateway shape that
boundary assumes.

---

## Sources

- TigerBeetle as the accounting backend for dynamic ID currencies — investigation,
  [`doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/`](../2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend.md)
  (itself cited to docs.tigerbeetle.com and the Jepsen TigerBeetle 0.16.11 analysis).
- Cloudflare Workers fit — investigation,
  [`doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/`](../2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit.md).
- `pkgs/id/ARCHITECTURE.md` — Iroh-based node identity, `postcard` peer protocol.
- ID program issues: [#2056](https://github.com/developing-today/code/issues/2056) (epic),
  [#2040](https://github.com/developing-today/code/issues/2040) (TigerBeetle investigation),
  [#2043](https://github.com/developing-today/code/issues/2043) (currencies v1),
  [#2063](https://github.com/developing-today/code/issues/2063) (conversion engine),
  [#2066](https://github.com/developing-today/code/issues/2066) (hosted boundary),
  [#2030](https://github.com/developing-today/code/issues/2030) (trust/visibility),
  [#2032](https://github.com/developing-today/code/issues/2032)/[#2033](https://github.com/developing-today/code/issues/2033) (abuse controls).
