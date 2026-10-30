# Grant lifecycle implementation spec — pending holds, fingerprint registry, clawback/shortfall, revocation policy

- Date: 2026-10-30
- Status: **design spec complete; implementation not started**. Spec only — no permanent grant feature is built on the unreviewed #2042 investigation (program rule, epic #2056). Implementation is filed as its own issue (linked from #2254).
- Issue: [#2254](https://github.com/developing-today/code/issues/2254). Parent investigation: [#2042](https://github.com/developing-today/code/issues/2042) (state machine + abuse cases 1–6, queue task 3353). Ledger basis: [#2040](https://github.com/developing-today/code/issues/2040) (TigerBeetle: posted transfers immutable, refunds are compensating transfers, clawback can fail post-spend-down, no freeze primitive, two-phase pending transfers native). Currency definition basis: [#2041](https://github.com/developing-today/code/issues/2041) (`revocation_policy` field, `accept_payment` layer). Synthesis that consumes the open parameters: [#2043](https://github.com/developing-today/code/issues/2043).

## 0. The rule everything follows

From #2042, verbatim in effect: **a purchase grant is born revocable and restricted; it earns unrestricted status only as the external refund/chargeback risk decays. Never let a grant become freely spendable faster than the money that backed it can be taken back.**

Consequences this spec is not allowed to trade away:

- Every grant passes through PendingHold. No source, partner, or operator action skips it.
- Liability for a clawback attaches to the **grantee ID**, never to the tokens and never to third-party holders who received them in good faith.
- Shortfall is a first-class state with a signed debt record, not an error path.
- For-cause clawback, issuer-discretion revocation, and delisting are three different acts with three different mechanisms (§5, §6). Delisting never touches a balance.

## 1. PendingHold mechanics — decision: TigerBeetle pending transfers

### 1.1 The choice

PendingHold is implemented as a **TigerBeetle two-phase pending transfer**, not as tokens in a flagged grant sub-account.

- Chosen: pending transfer from the issuer's grant-issuance account to the grantee's account for the currency, created at grant registration (§2.3), with `timeout` per §1.2. It is posted (phase two) when the refund window lapses clean, or voided when an adverse attestation arrives (§3). TigerBeetle guarantees a pending transfer resolves at most once — post and void cannot both land — which is exactly the invariant PendingHold needs.
- Rejected: grant sub-account with non-transferable/non-spendable flags. The flags would have to be re-checked at every spend and transfer path forever, the tokens would be visible as a balance the holder cannot use (a standing support and honesty problem), and posting the hold becomes a second compensating transfer instead of a phase transition — two mechanisms to audit where the ledger already provides one.
- Consequence: a currency with no TigerBeetle-backed ledger cannot offer purchase grants under this spec. Graph-native currencies (#2041) get grants only when their ledger leg exists; that is a scope limit of this spec, recorded here so it is a decision and not a surprise.

### 1.2 Window parameters (configuration, never constants)

Windows live in the currency service configuration, keyed by **grant source**, and are copied onto the grant record at registration (later config changes do not move an existing grant's window):

| Parameter | Meaning | Reference value (from #2042) | Set by |
|---|---|---|---|
| `refund_window[source]` | PendingHold length per grant source | Steam direct purchase: 14 days (Steam's standard terms; the under-2-hours-played condition is the storefront's, not ours — we hold for the time window only) | operator, per source |
| `refund_window[steam_retail_key]` | Retail-key sources vary by publisher | publisher-declared at source registration | operator + publisher |
| `posting_grace` | Time after window end during which the service must post the pending transfer before its TigerBeetle `timeout` voids it | — (open parameter OP-2) | operator |
| `chargeback_horizon` | How long an Active grant carries the contingent-liability marker (§4.1) | months, payment-system scale — not days | operator (OP-3) |

The pending transfer's TigerBeetle `timeout` is `refund_window + posting_grace`, in seconds, set at creation.

### 1.3 Post flow (window lapses clean)

1. A scheduler sweep finds grants in PendingHold whose window ended at least `posting_grace/2` ago and which have **no** adverse attestation (§3) recorded against their fingerprint or grantee.
2. For each, the service creates the post-pending transfer (phase two) referencing the pending transfer id. Grants move to **Active**; the grant record stores `posted_transfer_id` and `active_since`.
3. The sweep is idempotent and **resolves, never skips**: on encountering a grant still in PendingHold whose pending transfer the ledger says is already resolved, the sweep reads the ledger outcome and completes the matching grant-state update — posted → **Active** (recording `posted_transfer_id`); voided → the §1.4/§1.3-step-4 path for the recorded cause. A crash between the ledger post and the grant-record update therefore heals on the next sweep instead of stranding spendable funds behind a PendingHold label (which would route a later refund down the void-only path and lose the posted value). The at-most-once resolution makes a double-post impossible, so a crashed sweep is retried whole.
4. Fail-safe: if the sweep never runs and the `timeout` fires, TigerBeetle voids the pending transfer. The grant is **not** silently lost: the void is an auditable ledger event, and the service re-issues the grant as a directly posted transfer after re-checking the attestation log for the full window + horizon-to-date. Re-issuance after a lapsed window is within policy precisely because the window has lapsed; the re-issue references the voided transfer id in the new grant record version.

### 1.4 Void flow (adverse event inside the window)

1. An attestation (§3) of type `refund_executed`, `chargeback_filed`, or `publisher_revocation` arrives naming the grant's fingerprint while the grant is in PendingHold.
2. The service creates the void-pending transfer. Grant moves to **Revoked** with `revocation_cause = for-cause` and the attestation id recorded.
3. Nothing was ever spendable, so there is no clawback, no debt, and no third-party exposure. This clean totality is the reason §0 forbids skipping PendingHold.

## 2. Fingerprint registry

### 2.1 Hash construction

- Canonical form of a key: strip all whitespace and separator characters (`-`, space), uppercase ASCII. **This canonicalisation is frozen for the life of a storefront family**: it is the uniqueness identity of a key, so anything that changed it would mint a second identity for an already-granted or already-revoked key and reopen the farming loop. Per-storefront rules in the source configuration are therefore *validation only* (accepted character sets, length, check digits — they may reject a presentation, they may never change its canonical form). If a storefront's real key format ever changes so the frozen canonicalisation no longer fits, that is a new storefront namespace, and namespaces belonging to one storefront form a **storefront family**: the uniqueness (§2.2) and publisher-revocation (§2.4) checks always run across the whole family, so a key granted or revoked under an old namespace is still recognised under the new one.
- Fingerprint: `SHA-256("id-grant-key-v1" || 0x00 || storefront_id || 0x00 || canonical_key)`, hex-encoded. Deliberately no rule version in the hash — see above.
- The raw key exists only in the registration request's memory. It is never stored, never logged, and never appears in an error message or attestation. Logs may carry the first 12 hex characters of a fingerprint for correlation.

### 2.2 Storage and the one-grant rule

Table `grant_keys`:

| Column | Notes |
|---|---|
| `fingerprint` | PRIMARY KEY (with `storefront_id`), the uniqueness that enforces one grant per key |
| `storefront_id` | grant source namespace |
| `grant_id` | the grant this key produced |
| `registering_id` | the ID that registered first — first registration wins |
| `registered_at` | timestamp |
| `key_state` | `granted \| revoked_by_publisher \| refunded` — updated by attestations, never deletes the row |

Enforcement is the primary key, not an application check: grant creation (§2.3) inserts the `grant_keys` row and the grant record in one database transaction. A conflict means the key was already granted → the new presentation is **Rejected** (§7), the existing grant is untouched, and if the existing grant's `key_state` is `refunded` or `revoked_by_publisher` the rejection is additionally flagged as a resale/re-presentation attempt against the registering ID (abuse case 3). A deleted or recycled registry row would reopen the farming loop, so rows are append-only.

### 2.3 Registration flow (Presented → PendingHold / Rejected)

1. Client presents a key + storefront to the currency service, authenticated as the registering ID.
2. **Verification before anything is recorded.** The service validates the key against the storefront's authoritative channel (e.g. the storefront's key/product API) while it still holds the raw key: the key must be real, must not already be consumed in a way the storefront forbids, and the call must return the **product attribution** the grant amount is selected from. A string that merely looks like a key earns nothing — without this step any authenticated caller could present arbitrary distinct strings and harvest grants no refund attestation would ever claw back. A storefront with no authoritative verification channel cannot be enabled as a grant source; that is a source-eligibility rule, not a per-grant judgement. Only after verification succeeds does the service canonicalise and hash (§2.1) and discard the raw key.
3. Service checks the publisher-revocation index (§2.4) across the storefront family: a fingerprint already revoked by its publisher is **Rejected** before any grant exists.
4. **Intent-first creation (§2.5)** — the registry database and TigerBeetle are two systems and share no transaction, so creation is a durable-intent protocol, not a rollback: (a) in one database transaction, insert the `grant_keys` row (conflict → **Rejected**, §2.2) and the grant record {grant_id, fingerprint, grantee_id, currency, amount from the verified product attribution, source, `refund_window` snapshot, `revocation_policy` snapshot (§5.1), state = Presented} with a **deterministic TigerBeetle transfer id derived from the grant_id**; (b) create the pending transfer (§1.1) under that id — creating an already-existing transfer id is an idempotent no-op in the ledger, so a lost response is simply retried; (c) on ledger confirmation, advance the grant to PendingHold. A reconciler (§2.5) resolves any grant stranded between steps.
5. The grant amount and currency come from the source configuration for (storefront, product as returned by step 2's verification) — never from the request.

### 2.4 Publisher-revocation batch intake

- Publishers revoke key batches (e.g. a stolen batch, abuse case 4). Intake format: a signed JSON document `{publisher_id, storefront_id, batch_id, issued_at, fingerprints: [...]}`, where fingerprints are computed with the public §2.1 algorithm so **the publisher never sends raw keys**. Signature: the publisher's registered key for that storefront namespace.
- Intake is idempotent on `(publisher_id, batch_id)`: a replayed batch is recorded and changes nothing twice.
- Effect per fingerprint: `key_state = revoked_by_publisher`; then by grant state — no grant yet → future presentations Rejected (§2.3 step 3); grant in PendingHold → void (§1.4); grant Active → for-cause clawback (§4). The batch document itself is stored as the attestation evidence (§3).

### 2.5 Ledger atomicity: intent-first, transfer-second, confirm-third

Every flow in this spec that pairs a control-plane record with a ledger transfer (grant creation §2.3, post/void §1, clawback §4.2) follows the same protocol, because no database rollback can undo a TigerBeetle transfer whose response was lost:

1. **Intent:** write the control-plane record first, in its pre-ledger state, carrying a deterministic ledger transfer id derived from the record's own id (grant_id, debt_id, clawback id). Retrying the intent write is safe — the id is the identity.
2. **Transfer:** issue the ledger call under that id. Ledger creation with an existing id returns the existing transfer instead of duplicating it, so a lost or timed-out response is retried, not compensated.
3. **Confirm:** advance the control-plane record to its post-ledger state.
4. **Reconcile:** a standing reconciler scans records stuck in a pre-ledger state past a freshness bound, queries the ledger by transfer id, and drives each to the confirm step (transfer exists) or back to a clean retry/abandon decision (it does not, and the paired attestation check is re-run first). No record may sit in a pre-ledger state silently; the scan's backlog is an operator-visible metric.

The clawback/debt pair (§4.2 step 5) uses the same shape with the roles ordered for safety: the debt record (intent, with the clawback transfer id) is written **before** the compensating transfer is issued, so value can never move without the obligation already recorded; the confirm step marks both from the ledger outcome, and a partial outcome is the normal confirm, not an exception.

## 3. External-event attestation records

Refunds, chargebacks, and publisher revocations are not user-signed facts. The currency service records them as **service-signed attestations of external fact**; an attestation is the sole trigger for every for-cause transition in this spec. A wrong attestation is the service operator's signed act, with the same exposure as any other signed object in this program.

### 3.1 Record format

```json
{
  "attestation_id": "uuid",
  "event_type": "refund_executed | chargeback_filed | publisher_revocation",
  "source": "steam | payment_processor | publisher:<publisher_id>",
  "external_ref": "the source system's own identifier for the event",
  "fingerprint": "hex, §2.1 (when the event names a key)",
  "grantee_id": "the ID the event's grant belongs to (when it names an account instead)",
  "occurred_at": "when the external event happened, per the source",
  "received_at": "when the service recorded it",
  "evidence": {"kind": "webhook_payload | batch_document | processor_notice", "sha256": "hash of the stored evidence"},
  "service_signature": "currency service operator key over all fields above"
}
```

Intake rules: webhooks and processor notices are authenticated by the source's own mechanism (webhook signing secret, processor signature) **before** an attestation is minted — the service signature attests "we verified this arrived from the source", it does not launder unauthenticated input. Intake is idempotent on `(source, external_ref)`: duplicates are stored as duplicate receipts and trigger nothing twice.

### 3.2 Transitions each event type may trigger

| Event | Grant in PendingHold | Grant Active | Grant already Revoked/Settled |
|---|---|---|---|
| `refund_executed` | void → Revoked (§1.4) | clawback (§4) | recorded, no-op |
| `chargeback_filed` | void → Revoked (§1.4) | clawback (§4) | recorded, no-op |
| `publisher_revocation` (single or §2.4 batch) | void → Revoked (§1.4) | clawback (§4) | recorded, no-op |

No other transition is attestation-driven. In particular an attestation never moves money by itself: it opens the §4 choreography, whose own transfers are separately recorded.

## 4. Clawback, Shortfall, and debt

### 4.1 What can be clawed back: the outstanding figure

The grant record carries `amount_outstanding`: initially the grant amount; decreased when grant-derived value is **consumed by a delivered service** (an `accept_payment` redemption the service attributes to this grant, FIFO across the grantee's grants of that currency); **not** decreased by transfers out — a transfer moves tokens, it does not consume them, and the liability stays with the grantee (§0). While a grant is Active and younger than `chargeback_horizon` (§1.2), it carries a **contingent liability marker**: services accepting grant-derived tokens in that period are accepting issuer credit risk, which the marker makes visible instead of implicit.

### 4.2 Clawback choreography (Active → ClawbackPending → Revoked | Shortfall)

1. Trigger: a §3 attestation (for-cause), or an effective issuer-discretion revocation (§5.3). Grant moves to **ClawbackPending** and its outstanding figure is frozen at the trigger value.
2. The service computes `clawback_due = amount_outstanding` and `available = grantee's current balance of the currency across their accounts`.
3. **Partial clawback is the normal case, not an exception:** the service posts a compensating transfer of `min(clawback_due, available)` from the grantee to the issuer's clawback account (posted transfers are immutable, #2040 — a clawback is a new transfer, never an edit). Third-party holders are never a clawback source.
4. If the full due amount moved: grant → **Revoked**, `clawed_amount` recorded.
5. If less moved (including zero): grant → **Shortfall**; the remainder becomes a debt record (§4.3) written as durable intent before the transfer under the §2.5 protocol, so "we clawed what existed" and "the rest is owed" can never diverge.

### 4.3 Debt records

```json
{
  "debt_id": "uuid",
  "grantee_id": "...", "grant_id": "...", "currency": "...",
  "amount_original": 0, "amount_outstanding": 0,
  "cause_attestation_id": "§3 record that triggered the clawback, or the §5.3 notice id",
  "created_at": "...",
  "state": "open | settled | written_off",
  "service_signature": "..."
}
```

- **Offset against future value:** any future grant issuance or service payout to the grantee in the same currency is applied to the oldest open debt first (FIFO across debts): the issuance/payout transfer is split — offset portion to the issuer's clawback account with an offset record referencing both the debt and the transfer, remainder (if any) delivered normally. Offsets decrement `amount_outstanding` atomically with the transfer.
- **Suspension effects** while any debt is open: the grantee cannot register new grants (§2.3 refuses with the open debt id named), and grant-derived redemptions for that ID are refused at the `accept_payment` layer. Ordinary holding and transfer of unrelated value is unaffected — suspension is a grant-system consequence, not a general account freeze (there is no freeze primitive, #2040, and this spec does not invent one).
- **Settlement:** `amount_outstanding` reaching zero via offsets (or a direct repayment transfer referencing the debt) moves the debt to `settled` and the grant to **Settled**. Suspension is a function of *all* the grantee's debts, recomputed at this moment: it lifts **only if the grantee has no remaining open debt** — settling one of several debts changes nothing about suspension while a larger one stands.
- **Write-off:** the issuer may write off a debt (the sympathetic-case valve of abuse case 4) by a signed write-off record `{debt_id, amount, reason, issuer_signature}`. The debt moves to `written_off`, the grant to **Settled**, the same all-debts suspension recompute as settlement applies, and the loss is permanently visible as the issuer's signed act — a write-off is never a silent deletion. Debt expiry, if any, is an open parameter (OP-4), not a default.

## 5. `revocation_policy` enforcement

### 5.1 The policy ladder and the snapshot rule

The currency definition (#2041) declares `revocation_policy: none | for-cause | issuer-discretion`, a ladder of increasing issuer power. The service copies the policy onto the grant record at registration (§2.3). **The policy in force at issuance governs the grant for its whole life**: a definition edit can change what future grants promise, it cannot retroactively grant the issuer power over tokens already issued under weaker terms.

### 5.2 Refusal rules (checked at execution, not at request)

| Requested act | `none` | `for-cause` | `issuer-discretion` |
|---|---|---|---|
| Void a PendingHold on a §3 attestation | allowed — voiding a hold that was never spendable is issuance mechanics, not revocation, under every policy | allowed | allowed |
| Clawback of an Active grant with a valid §3 attestation | **refused** — the issuer chose irrevocable; refund risk past the window is theirs | allowed (§4) | allowed (§4) |
| Clawback of an Active grant without an attestation (discretionary) | **refused** | **refused** | allowed only via the §5.3 notice + grace flow |

A refusal is a recorded, signed service response naming the policy value and the grant's snapshot — "the service enforces the declared policy" (abuse case 6) is only credible if refusals are as auditable as executions.

### 5.3 Issuer-discretion revocation: notice + grace flow

1. The issuer submits a **revocation notice object** `{notice_id, scope: grant_id | currency, cause, effective_at, issuer_signature}` to the service's public revocation registry. The registry stamps `registered_at` — the service's own first-publication timestamp — at intake; the issuer's timestamps are informational only, because a signed `published_at` proves what the issuer wrote, not when holders could first see it. Intake requires `effective_at >= registered_at + grace window (OP-5)` and recomputes nothing from issuer-supplied times; a notice asking for an earlier effect is refused at intake. Grace is therefore measured from when the warning actually became public.
2. During grace, affected grants stay Active and fully usable — holders can spend, transfer, or exit; the notice is the warning, and usability during grace is what makes it a warning rather than a seizure with a timestamp.
3. A holder may file a dispute record referencing the notice; a dispute does not block execution (the definition promised the issuer this power) but is permanently attached to the notice — abusive revocations carry their objections with them.
4. At `effective_at`, the §4 choreography runs per grant in scope, with the notice id standing in for the attestation id in the debt record's cause field.
5. Every notice is permanent and attributable. The market/registry layer punishes abusive issuers; the protocol's job is that they cannot deny what they signed.

## 6. Delisting is not revocation

Any service may stop accepting a currency at any time through its `accept_payment` configuration (#2041/#2030). That act touches no balance, no grant state, and no debt, and it requires no policy permission — including by Drew for developing-today's own services. This spec provides no mechanism that turns an acceptance decision into a balance action, deliberately.

## 7. State machine implementation map (no orphan states)

Every state and transition in the #2042 diagram, and the spec mechanism that implements it:

| #2042 state / transition | Implemented by |
|---|---|
| **Presented** | §2.3 registration in flight: key canonicalised + hashed, grant record not yet committed |
| Presented → **Rejected**: fingerprint already granted / key revoked by publisher | §2.2 primary-key conflict; §2.3 step 3 publisher-revocation index check |
| Presented → **PendingHold**: key verified, grant issued as pending hold | §2.3 step 2 (authoritative storefront verification + product attribution), then §2.3 step 4 / §2.5 intent-first creation of the TigerBeetle pending transfer + §1.1 |
| PendingHold → **Active**: refund window lapses, hold posts | §1.3 post flow (scheduler sweep; timeout-void re-issuance fail-safe) |
| PendingHold → **Revoked**: refund / chargeback / publisher revocation in window | §1.4 void flow on a §3 attestation (incl. §2.4 batches) |
| Active → **Spent**: tokens consumed for services (terminal per amount) | §4.1: `amount_outstanding` attributed consumption via `accept_payment` redemptions; the grant is Spent when its outstanding figure reaches zero by consumption. (A grant is an amount, not a coin: partial consumption is a figure change, full consumption is the terminal state.) |
| Active → **ClawbackPending**: refund / chargeback / for-cause revocation notice | §4.2 step 1 (§3 attestation, or §5.3 notice at `effective_at`) — subject to the §5.2 policy check |
| ClawbackPending → **Revoked**: clawback of unspent balance succeeds | §4.2 steps 2–4 (compensating transfer of the full outstanding figure) |
| ClawbackPending → **Shortfall**: balance already spent — partial clawback + debt record | §4.2 step 5 + §4.3 debt record creation via the §2.5 intent-first protocol |
| Shortfall → **Settled**: debt offset against future grants/earnings, or written off by issuer | §4.3 offset rules, settlement, and signed write-off |
| Active → **Expired**: grant terms expire unspent (if the currency definition says grants expire) | Definition-carried grant expiry: at the expiry time the service posts a compensating transfer of `min(amount_outstanding, grantee's available balance)` to the issuer sink account named in the currency definition and zeroes the figure. A shortfall here is **not** a debt and triggers no suspension: transfers out were a permitted use of an Active grant, expiry was a term the grantee accepted, and no restitution obligation arises — the unrecovered remainder is recorded on the grant record as `expiry_shortfall` (the issuer's visible, by-terms loss) and the grant still reaches Expired terminally. Third-party holders are never an expiry source, exactly as in §4. (If the definition declares no expiry, this transition does not exist for that currency.) A grant still in PendingHold at its expiry voids instead — it was never spendable. |
| **Revoked** → terminal, **Spent** → terminal | No outgoing transitions are implemented; attestations arriving against terminal grants are recorded as no-ops (§3.2) |

## 8. Abuse-case trace (#2042 cases 1–6)

| Case | Spec mechanism | Residual risk |
|---|---|---|
| 1. Buy → register → refund → keep tokens | PendingHold spans the whole refund window (§1); a refund voids a hold that was never spendable (§1.4). Profit zero. | None identified, provided no source is configured with a window shorter than its storefront's real refund window — a configuration duty (§1.2), flagged here rather than hidden. |
| 2. Chargeback after activation and spend-down | Clawback of the unspent remainder + Shortfall debt + suspension (§4.2, §4.3); consumed services are honestly unrecoverable (§4.1 terminal Spent). | **Accepted residual:** the issuer eats genuinely consumed services. #2042's framing stands — bounded by grant size and priced into the grant amount; the loss is visible via debt/write-off records, not eliminated. |
| 3. Same key to many IDs / resold after refund | Fingerprint registry: one grant per key, first wins, append-only rows (§2.2); re-presentation flagged against the registering ID. | A key never registered anywhere is indistinguishable from a fresh legitimate key — by design; the registry governs grants, not key sales. |
| 4. Publisher revokes a stolen key batch after activation | Publisher revocation is a for-cause attestation (§2.4, §3) → §4 path incl. Shortfall. | **Policy call, surfaced:** an innocent buyer of a stolen key still receives a debt record (they hold value not validly paid for); the signed write-off (§4.3) is the sympathetic-case valve, exercised publicly by the issuer — OP-6 asks #2043/operator whether a good-faith-purchase presumption should soften this. |
| 5. Grantee transfers tokens away, then refunds, leaving an empty account | Liability is personal to the grantee ID (§0, §4.1): clawback sources only the grantee's accounts, third-party holders untouched, shortfall becomes the grantee's debt (§4.3). | The debt may never be collected if the grantee never returns — same accepted residual class as case 2, bounded the same way. |
| 6. Issuer revokes a whole currency to rug holders | Possible only where the definition declared `issuer-discretion` (§5.1 snapshot rule prevents retroactive power); mandatory signed notice + grace (§5.3); under `for-cause`/`none` the service **refuses** (§5.2, refusals recorded). Delisting remains available to everyone and seizes nothing (§6). | **Accepted residual:** where discretion was honestly declared and accepted by holders at issuance, a rug is possible inside the rules — the defence is the permanent signed notice, the grace exit, and registry/market reaction, not prevention. |

## 9. Open parameters — for #2043 synthesis / operator, not defaulted here

This spec deliberately does **not** choose values for the following. Each is listed with its owner; an implementation that silently defaults any of them fails this spec's acceptance.

- **OP-1** `refund_window` per grant source beyond the Steam-direct reference value (esp. retail-key sources, per publisher). Owner: operator with publisher terms.
- **OP-2** `posting_grace` length (§1.2/§1.3). Owner: operator (ledger operations).
- **OP-3** `chargeback_horizon` length (§4.1). Owner: #2043 / operator, from payment-processor terms.
- **OP-4** Debt expiry, if any, and the write-off approval policy (§4.3). Owner: #2043 / operator.
- **OP-5** Grace-window length for issuer-discretion revocations (§5.3), incl. whether it scales with scope (grant vs whole currency). Owner: #2043.
- **OP-6** Good-faith-purchase presumption for publisher-revoked stolen batches (case 4 softening). Owner: #2043 / operator.
- **OP-7** `revocation_policy` default for developing-today's own premium token. #2042's recommendation on record: `for-cause`. Owner: #2043 / Drew.
- **OP-8** Grant expiry defaults where a currency definition is silent (§7, Expired row). Owner: #2043.

## 10. Implementation routing

This issue delivers the spec only. Implementation is filed as a separate issue (linked from #2254 when filed) covering: the `grant_keys` registry + registration flow (§2), pending-transfer lifecycle + scheduler sweep (§1), attestation intake (§3), clawback/debt engine (§4), and policy enforcement + revocation registry (§5) — sequenced so §2 + §1 land first (they close abuse case 1, the core loop) and no phase enables Active-state grants before §4 exists to claw them back.
