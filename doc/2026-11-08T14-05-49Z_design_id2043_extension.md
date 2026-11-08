# Design: ID currencies v1 — extension for Drew's 2026-11-08 rulings

- Date: 2026-11-08
- Status: **design proposal** — extends the settled design; no implementation code is included in this PR, and nothing here changes a ruling. The rulings themselves are already recorded on the issues (links below); this document designs how the three modifications fit the model and names what still needs Drew's clarification before implementation.
- Issues: [#2043 — ID currencies v1](https://github.com/developing-today/code/issues/2043) (rulings recorded in [comment 6721988851](https://github.com/developing-today/code/issues/2043#issuecomment-6721988851)) and [#2296 — grant refund window and pending spendability](https://github.com/developing-today/code/issues/2296) (rulings recorded in [comment 6721993178](https://github.com/developing-today/code/issues/2296#issuecomment-6721993178)), parent epic #2056.
- Base design: `doc/2026-11-01_design_id2043.md` (PR #3886). Where this extension and the base design disagree, the base design stands until Drew accepts this extension; the disagreements are listed explicitly in §5.

## The rulings this extension designs

Drew ruled on all six policy questions from the base design's §8 on 2026-11-08. Three answers confirm the recommendations as stated (Q3 recovery key, Q4 emergency revocation, Q6 open-source boundary) and need no design work. Three modify the design and are the subject of this document:

- **Q1 (name):** the neutral label "units" ships and branding is deferred, as recommended — **and** the model has **three currencies: units, pending units, and revoked units** (§1).
- **Q2 (supply):** a finite cap for all v1, as recommended — **except transfers from the account for supplying currency** (§2). His sentence ends "named like tiger beetle would be"; that phrase is unresolved (§4, U1) and is not interpreted anywhere in this document.
- **Q5 (grant terms, #2296):** the base terms are accepted (14-day Steam-direct hold, pending never spendable, per-processor chargeback horizon, debt never silently expires, for-cause-only revocation for first-party premium) — **except pending units may be traded when both parties accept/sign**, under a chained option-contract mechanism (§3). The final sentence of his ruling, proposing atomic batch escrow of the whole chain, ends with a question mark in his text and is recorded on the issues as an **open proposal, not a ruling**; §3.7 designs it as an option and recommends on it.

Drew's full ruling text is quoted verbatim on both issues (links above) and is the authority for everything below. Where his text does not settle a point, this document says UNRESOLVED and §4 lists the exact clarification needed; no unresolved point is silently designed around.

## 1. Three currencies: units, pending units, revoked units

### 1.1 What the ruling changes

The base design has one ledger-anchored currency per signed definition. "Pending" is a **state** of value in that currency: a purchase grant is a pending TigerBeetle hold, unspendable until the refund window closes, then it posts into the holder's ordinary balance. "Revoked" is an **event/policy**: issuer revocation, gateway suspension, per-definition revocation policy. Neither is a currency, and the base design's identity rule — a currency is `(issuer public key, definition event/hash)` — gives the system exactly one currency per grant-issuing definition.

Drew's ruling names three currencies. Taken as the ruling (which is how this document treats it), the grant value exists in three distinct currencies over its life:

| Currency | What it is | Who holds it | What can be done with it |
| --- | --- | --- | --- |
| **pending units** | Grant value inside its refund/chargeback window | The grantee (and, under §3, a chain of consenting trade counterparties) | Hold; trade under a chained option contract (§3); vest into units at window close. Never spendable (§3.6). |
| **units** | Settled value | Any holder | Everything the base design allows: spend, transfer, convert under a rate authorisation. |
| **revoked units** | Value that has been revoked or cancelled out of the live currencies | The issuer's revocation account; the originator after exercising the §3 cancel option | Evidence and accounting only. Never spendable, never re-enters circulation except by a compensating operation (§1.4). |

Two readings of the ruling are possible. Both are designed below; §1.5 recommends one.

### 1.2 Reading A — three distinct currency definitions

Each of the three is its own signed currency definition under the base design's identity rule: its own `(issuer, definition)` pair, its own scale, its own supply policy, its own TigerBeetle ledger. Life-cycle movements between them are **conversions** in the base design's sense: a signed, directional `currency.rate_authorisation`, a pinned quote, and one atomically linked TigerBeetle batch — because, per #2040, TigerBeetle has no cross-ledger transfer.

Mechanics under Reading A:

- **Grant issuance** mints (or transfers from the supply account, §2) pending units to the grantee. The pending currency's cap is the grant programme's authorised issuance.
- **Vesting** is a 1:1 conversion pending units → units, authorised by a standing issuer-signed rate authorisation created at definition time (rate exactly 1, zero fee, eligible caller = the gateway's vesting scheduler, `not_before` = window close per grant). The conversion burns/escrows the pending leg and posts the units leg in one linked batch, exactly like any other conversion, and produces a conversion receipt.
- **Revocation / void** is a 1:1 conversion pending units → revoked units (in-window adverse event) or units → revoked units (post-activation revocation under the definition's revocation policy), again via standing authorisation and linked batch.
- **The §3 cancel option** is native to this reading: "trade pending units 1:1 for revoked units" is literally one of these conversions, initiated by the originator.
- **Accounts:** per holder, one account per currency (three TigerBeetle accounts per holder per programme, on three ledgers), plus issuer accounts: supply (§2), revocation sink, and conversion-inventory accounts on each ledger pair the standing authorisations touch (#2063 requires explicitly bound inventory accounts; ordinary shared liquidity accounts are not silently conversion inventory).
- **Invariants:** each currency keeps base Invariant 2 (finite cap) as amended by §2; cross-currency conservation becomes a new invariant — the sum across the three currencies plus burned amounts is constant except at authorised issuance and burn points, and every change in one currency's supply is matched by an equal opposite change in another's under a single conversion ID (base Invariants 6–7 extend naturally, since conversions already work this way).

Costs of Reading A: every life-cycle transition pays full conversion machinery (quote pinning, cap reservation, linked batch, receipt) even though the rate is fixed at 1 and both sides are the same issuer; there are three caps to size and audit instead of one; wallets and services must display and reconcile three balances per programme; the graph-native binding has no natural analogue (a graph-native currency has no pending/revoked siblings unless it also triples, and folds would need cross-definition ancestry rules the base design does not have).

### 1.3 Reading B — one currency definition, three balance classes (implementation technique)

One signed definition, one TigerBeetle ledger — and inside that ledger, each holder's position is kept in **three segregated balance classes**, realised as three TigerBeetle accounts per holder on the same ledger: a `pending` account, an `available` account (units), and a `revoked` account. The classes are named, displayed, and reported as the three currencies Drew named — units, pending units, revoked units — in every wallet view, statement, and API response. They are never summed into one number, and no code path may treat them as fungible with each other.

Mechanics under Reading B:

- **Grant issuance** credits the grantee's `pending` account.
- **Vesting** is an ordinary same-ledger transfer `pending` → `available`, executed by the gateway at window close. No conversion machinery: same ledger, so a plain transfer (or a two-phase pending transfer in TigerBeetle's own sense, which is how the base design already models the hold) settles it.
- **Revocation / void** is a transfer `pending` → `revoked` or `available` → `revoked`, into the holder's own `revoked` account (not the issuer's), so the holder's statement shows exactly what was taken and why; the issuer's revocation policy event is the authorisation, joined by `user_data_128` as usual.
- **The §3 cancel option** is a transfer of the originator's pending units to their own `revoked` account at 1:1 — value-preserving, identity-preserving, and trivially auditable.
- **Accounts:** three accounts per holder on one ledger; issuer supply and inventory accounts as in §2. Account count is the same as Reading A; ledger count is one instead of three.
- **Invariants:** one cap (§2) governs total supply across the three classes; a class-conservation invariant states that transfers between classes neither create nor destroy value, and the class totals reconcile to the currency total at every snapshot (base Invariant 11 gains a per-class root).

What Reading B does **not** do: it does not collapse the three currencies back into "states of one balance." The classes are separate accounts with separate balances, separate transfer rules (pending can never pay a service; revoked can never move except by compensating operation, §1.4), and separate presentation. A holder's pending units are not "their units, temporarily locked" in any code path: no spend, conversion, or transfer rule reads more than one class, except the three named life-cycle transitions (vest, revoke/void, cancel), each of which is its own signed, authorised operation.

Where Reading B genuinely differs from the ruling's letter: the base design's identity rule makes a currency a signed definition, and under B the three currencies share one definition and one ledger rather than having three of each. If Drew intends three independently definable currencies — different issuers able to issue one class, different scales, separate caps as a policy matter rather than an accounting one — only Reading A satisfies him. Nothing in his ruling text states that intent; the text names three currencies in the context of one grant programme's life cycle. This is a real fork in the design, and it is put to Drew as clarification U7 (§4) — but implementation should not wait for it if §1.5's recommendation is accepted provisionally, because B's account structure is a strict subset of A's: moving B → A later means splitting ledgers and re-pinning definitions, while the account-level history carries over unchanged.

### 1.4 Rules common to both readings

- **Revoked units are terminal.** Value in the revoked currency/class is never spendable, never tradable (including under §3), and never converts back directly. The only exit is a compensating operation of the kind the base design already prescribes for posted transfers: a new, separately authorised compensating issuance/conversion referencing the original revocation, used for the good-faith relief path (base design §8 Q5: signed write-off / case-by-case relief). This keeps "revoked" from becoming a laundering hop back into units.
- **Pending units are never spendable** in either reading (§3.6 gives the exact wording change #2296 needs).
- **Scale is shared.** All three currencies/classes use the definition's scale; a 1:1 transition is exact integer equality, no rounding path exists.
- **Snapshots and reconciliation** report per-currency (A) or per-class (B) roots; a snapshot that reports only the combined total does not satisfy base Invariant 11 under this extension.

### 1.5 Recommendation

**Adopt Reading B — three balance classes on one ledger, presented and ruled as three currencies — as the implementation technique for the three-currency ruling.** Why, in one line: it gives Drew exactly the three named, separately-ruled currencies at every surface a user or auditor touches, while keeping vesting and revocation as same-ledger transfers instead of paying full cross-ledger conversion machinery for fixed 1:1 life-cycle moves. If Drew answers U7 saying he intends three independent definitions, Reading A is fully designed above and the switch is a known, bounded migration.

## 2. The supply account and the cap exception (Q2)

### 2.1 What Drew ruled

Finite cap for all v1 — "except transfers from the account for supplying currency." The base design's Invariant 2 reads: *"Every currency has a finite, explicit issuance cap in v1; cap checks are deterministic under causal order and forks."* The design has no supply account and no exception. This section defines both, and §2.4 gives the replacement invariant wording.

### 2.2 The supplying account, defined

- Each ledger-anchored currency (under §1 Reading B: each programme ledger; under Reading A: each of the three ledgers) has exactly one **supply account**: a gateway-controlled TigerBeetle account, created with the currency's registry entry, named in the signed definition by its registry account ID, and controlled by no key other than the currency service's.
- The supply account is the **only** account a currency's value can be issued into existence through. Issuance is a transfer out of the supply account; the supply account is funded only by the definition process itself (its opening balance is the authorised issuance, §2.3). No other account may go negative, and no other account's transfers create value.
- The supply account is not a holder account: it cannot receive ordinary transfers, cannot be a trade counterparty under §3, cannot be debited by clawback, and its balance is published in every supply audit (base phase 4 already produces supply-audit/reconciliation reports; the supply account balance becomes a required line).

### 2.3 Mint/burn accounting, and whether supply transfers count against the cap

Drew's text does not say whether transfers out of the supply account count against the finite cap — the ruling comment on #2043 records this as unresolved, and it is carried here as U2 (§4). This section designs the accounting so that either answer can be adopted by changing one sentence of the invariant (§2.4), and recommends the reading that keeps the cap meaningful:

- **Authorised issuance (the cap)** is a number in the signed definition: the maximum total value that may ever leave the supply account, net of returns, over the currency's life. Recommended semantics: **transfers out of the supply account count against the cap; transfers back into it (burns, voided grants returned by the issuer's own operations, expired unissued allocations the gateway returns) restore headroom.** Under this reading the exception in Drew's sentence is about *who may move value without a holder-style authorisation* — the supply account's transfers are issuance events authorised by the definition itself, not exempt from counting. The alternative reading — supply transfers do not count at all — makes the cap unenforceable (any amount could be issued by routing it through the supply account, which is the only issuance path anyway), so this document does not recommend it; it is recorded as the literal alternative in §2.4 for Drew's U2 answer.
- **Mint** is not a ledger primitive here: "minting" N units = transferring N from the supply account to a holder account (or, at grant issuance, to a holder's pending account/class). The graph issuance event, signed by the issuer, authorises it; the gateway validates definition, cap headroom, and registry mapping before submitting, exactly as base §3 already requires for anchored issuance.
- **Burn** is a transfer into the supply account from a holder account under a signed burn event, or into a dedicated burn account if Reading A is adopted and cross-ledger symmetry is preferred; either way the burn is final, reduces circulating supply, and — under the recommended semantics — restores cap headroom only when it returns to the supply account. Debts and write-offs never touch the supply account: a signed write-off (base Invariant 9) is a graph/control-plane record, not a ledger movement.
- **Reconciliation:** at every snapshot, `supply account balance + Σ holder balances (all classes) + burned-outside-supply (Reading A burn account, if any) = authorised issuance`, and `cumulative net issuance ≤ cap`. A mismatch is a durable finding, never silently dropped (base Invariant 7).

### 2.4 Replacement wording for Invariant 2

> **2. Definition and supply:** scale/binding are immutable for an identity. Every currency has a finite, explicit issuance cap in v1, stated in its signed definition. Each currency has exactly one supply account, named in the definition and controlled solely by the currency service; value enters circulation only by transfer out of the supply account under a signed issuance event, and such transfers count against the cap net of returns to the supply account. Cap checks are deterministic under causal order and forks. No other account may create value, and the supply account's balance and cumulative net issuance are published in every supply audit.

If Drew's U2 answer is the literal alternative (supply transfers exempt from counting), the fourth sentence is replaced by: *"transfers out of the supply account are not counted against the cap; the cap binds all other issuance paths"* — and the design then needs a separate control on the supply account's opening balance, since that balance becomes the de facto cap. This document flags that consequence rather than choosing it.

### 2.5 The unresolved phrase

Drew's Q2 sentence ends: "named like tiger beetle would be". Recorded verbatim; **UNRESOLVED** (U1, §4). It is not interpreted in this section or anywhere in this document — in particular, nothing here assumes it names the supply account, the cap, or the exception.

## 3. Trading pending units — the #2296 amendment (Q5)

### 3.0 Shape of the mechanism

Base terms stand as accepted: pending units vest after the source's window (14 days Steam-direct), are never spendable, and carry the per-processor chargeback horizon and the rest. Drew's amendment adds one capability: **a pending position may be traded — transferred to another party — when both parties accept/sign, and only inside a chain of signed option contracts that starts at the initial grant.** Everything in this section is ledger-anchored only; graph-native currencies have no pending grants (base design §4) and so no pending trading.

Vocabulary used below, all new graph event types in the currency vocabulary (closed schemas, per base §1):

- `currency.pending_option` — the **option contract**: a signed event created at, or chained from, the initial grant, authorising one trade (or one cancel exercise, §3.3) of a specified pending position.
- **Chain** — the sequence grant → option₁ → trade₁ → option₂ → trade₂ → …, where each option names the previous trade/grant event as its parent. A pending position is tradable only while it carries an unbroken chain back to its initial grant event ("only trade able as a chained item from initial grant by us").
- **Init** — the anchor event/time the cancel window runs from. Drew did not define it (U3, §4); this design uses *the initial grant event* as the working reading everywhere a start point is needed, and marks each such use.

### 3.1 The option contract

A `currency.pending_option` event carries:

- the pending position it covers: grant event ID, current holder (the option grantor), amount (exact integer, scale of the definition), and the position's chain parent (grant event or previous trade event);
- the counterparty (the option taker) by identity — an option is not a bearer instrument; only the named counterparty may countersign;
- the consideration terms, as references: trades under this mechanism exchange pending units for value **outside** the pending currency — either units (settled currency) paid through an ordinary anchored transfer/conversion, or an off-ledger consideration recorded by hash. The option event names which, and if units, the linked payment transfer ID once executed (§3.4). The mechanism does not create a pending-for-pending barter path in v1;
- `not_before` / `not_after` bounds on exercise (the base design's rate authorisations already use this vocabulary; the option contract reuses it deliberately);
- the **notBefore vesting attachment** (§3.3): the term Drew describes as attaching "the notBefore trade from pending units to units";
- signatures: the grantor signs at creation; the contract is exercisable only once the taker has countersigned (accept/sign — both parties, per Drew's ruling). An unsigned or singly-signed option authorises nothing.

Options are non-transferable and single-use: exercising an option consumes it; a further trade by the new holder requires a new option chained from the trade event (§3.5).

### 3.2 Contract-as-input: no contract, no trade

Drew: "any pending units without a provided option contract will be denied by us to trade in, our clients will require it as an input for any pending unit trade."

- The gateway's pending-trade endpoint takes the option contract event ID as a **required input**, alongside the trade intent. There is no contract-less path in the API, not merely a policy check: the request schema has no valid shape without it.
- The official clients (the ones "our clients" refers to) require the same input client-side, so a user cannot even compose a contract-less trade request in a first-party client.
- Gateway validation, in order: option exists and is in the graph; chain from the option reaches the initial grant event with no gaps (§3.5); both signatures valid and current-generation (base Invariant 4/10 rules apply); amount ≤ the position's remaining pending amount; `not_before`/`not_after` satisfied at exercise time; position still pending (window not closed, not voided, not already traded under a competing option — the position's chain head is unique, so a second live option on the same chain head is rejected at creation, not at exercise). Any failure denies the trade with a signed denial record; a denial moves no value.

### 3.3 The originator cancel option

Drew: "transfer cancel option that originator can trade their pending units 1:1 for revoked units not after (14 days from Init) which also attaches the notBefore trade from pending units to units."

Designed mechanics:

- At the initial grant, the grant event itself carries (or is accompanied by) a standing cancel option in favour of the **originator** (the grantee of the initial grant — the reading used here; if "originator" means the issuer, the mechanics are identical with the holder of the option changed, and the distinction is noted for Drew in U3's neighbourhood, §4).
- **Exercise:** the originator signs an exercise referencing the cancel option; the gateway executes the 1:1 pending → revoked movement (§1: a conversion under Reading A, a class transfer under Reading B) for the specified amount, up to the position's remaining pending amount. Partial exercise is allowed; the unexercised remainder keeps its chain and its own still-live cancel option until the deadline.
- **Deadline:** exercise is valid **not after 14 days from Init** (working reading: the initial grant event's timestamp). After the deadline the cancel option lapses — it cannot be revived by stale gateway state (base Invariant 4).
- **The notBefore attachment:** exercising the cancel option "also attaches the notBefore trade from pending units to units." Working design (the phrase itself is UNRESOLVED, U4): the exercise event embeds a vesting term stating that any portion of the position **not** cancelled continues to vest by the ordinary pending → units transition at its `not_before` (= window close), i.e. exercising a partial cancel re-affirms and pins the vesting schedule for the remainder rather than leaving it implicit. The cancel exercise receipt therefore records three things atomically: amount revoked, amount remaining pending, and the remainder's vesting `not_before`. If Drew means something else by the attachment, only this subsection changes.
- The cancelled amount lands in revoked units (§1.4: terminal). The originator receives revoked units, not units — the cancel option is an exit-from-the-trade-chain device, not an early-vesting device: it unwinds the originator's exposure (and, per §3.5, the downstream chain's expectations are settled in revoked units under the escrow variant, §3.7) without making pending value spendable or settled early.

### 3.4 Execution of a trade

A trade under a countersigned option executes as one gateway operation:

1. Validate per §3.2.
2. Move the pending amount from grantor to taker (same-ledger transfer between pending accounts/classes; under Reading A, a transfer within the pending currency's ledger — no conversion is involved in a trade, the currency does not change).
3. If the consideration is units, execute the linked units payment in the same operation batch (linked transfers, base Invariant 6 pattern) so the pending leg and the payment leg post or fail together. If the consideration is off-ledger, the trade posts on the countersignature plus the grantor's signed attestation of the consideration terms' hash; the platform does not adjudicate off-ledger payment, and the option's terms must say so in the client UI.
4. Emit the trade event as the new chain head for the position, naming the option event as its authorisation (`user_data_128` join, base Invariant 7).

The taker receives **pending units**, with the original window and chargeback exposure intact: trading does not reset, shorten, or extend the vesting clock, and does not launder the position's provenance — the chain *is* the provenance. Post-activation clawback rules (base Invariant 9) still attach per the grant terms; who bears a shortfall after a trade — grantor, current holder, or per the option's terms — must be an explicit option field, defaulting to the current holder at the time the adverse event lands, with the option contract free to allocate it otherwise. (This allocation question is a design consequence of tradability that Drew's text does not address; it is flagged here as a design decision taken by this document, not attributed to Drew.)

### 3.5 Chained redemption and ordering

Drew: "each users contracted other users can redeem against that users account using their signed chained contract, they can [resubmit] signed countersigned contracts to us and we will run as soon as all the chains up to them are done." ("oresubmit signs" in his text is read as "resubmit signed", a transcription guess recorded on #2296, not a ruling.)

- **Redemption** = presenting a countersigned option contract to the gateway to run the trade it authorises, including at vesting time presenting the chain to claim the vested units into the current holder's account.
- **Ordering rule:** a chain link (option exercise / redemption) runs only when **every link before it in its chain has completed** — the grant is final, each prior trade has posted, each prior option is consumed. The gateway evaluates a submitted contract against the chain state: if any ancestor link is incomplete, the submission is **queued, not denied** — Drew's "we will run as soon as all the chains up to them are done": the gateway holds the countersigned contract and executes it automatically when its last incomplete ancestor completes, in chain order, without the parties resubmitting. (Resubmission is also accepted and is idempotent — same contract, same effect, base Invariant 6.)
- A link whose ancestor is **denied or voided** (e.g. the underlying grant is voided by an in-window adverse event) can never complete; queued descendants are then denied with a signed denial naming the failed ancestor, and each party's client surfaces it. Value never moves on a broken chain.
- Chain state is a control-plane projection over the graph events, rebuildable like every other projection (base §2); the graph chain, not the projection, is the audit record.

### 3.6 "Pending never spendable" is preserved — and the #2296 rewording

Tradability does not touch spendability, and the distinction is mechanical, not rhetorical:

- **Spendable** means acceptable by a service as payment — a `token.receipt`-producing payment, a conversion, or any debit that settles a purchase. Pending units have no such path: no service endpoint accepts them, no rate authorisation may name the pending currency/class as a source, and the gateway rejects any payment intent drawn on a pending balance.
- **Tradable** means exactly one thing: transfer to a named counterparty under a countersigned chained option contract (§3.1–§3.4), receiving pending units still inside their window. The recipient gains no spending power they did not have; the position's restrictions travel with it.

#2296's Decision 2 currently reads (issue body, verbatim): *"Recommendation: **not spendable, at all.** Allowing purchases against pending balance reintroduces the clawback problem through the front door."* — and its acceptance criterion asks for pending tokens "spendable or not, explicitly confirmed." The exact replacement wording this extension proposes for Decision 2, for Drew's acceptance on #2296:

> **Decision 2 — spendability and tradability of pending units.** Pending units are **not spendable, at all**: no service may accept them as payment, no conversion may draw on them, and no purchase may be made against a pending balance; allowing purchases against pending balance reintroduces the clawback problem through the front door. Pending units are **tradable** only under the chained option-contract mechanism of the #2043 extension design: both parties accept/sign, the option contract is a required input to any pending-unit trade (trades without one are denied), the position's window and restrictions travel with it unchanged, and a trade confers no spending power. Tradable is not spendable.

### 3.7 The 28-day auto-sign behaviour

Drew: "at 28 days notAfter signature is enabled, we automatically sign and put the transfer at 14 days."

What is settled by the text: there is a day-28 moment; at it, a `notAfter` signature becomes enabled; the platform ("we") automatically signs; and a transfer is placed "at 14 days." What is **not** settled (U5, §4): 28 days counted from what (working reading: from Init, the initial grant), and "put the transfer at 14 days" measured from what and effective when (from the auto-sign? from Init? a 14-day settlement delay on the auto-signed transfer?).

Designed with those two readings marked: at **Init + 28 days**, any option contract in the chain that was created with the auto-sign term and remains unexercised has its `not_after` signature **enabled** — the platform's countersignature, pre-authorised by the contract's own auto-sign term (both parties accepted this term when they signed; the platform is executing their instruction, not originating one) — and the transfer it authorises is placed with a **14-day settlement point** (working reading: effective 14 days after the auto-sign, i.e. the transfer posts at Init + 42 days under the two working readings combined). Safety rails that hold under any reading: auto-sign applies only to contracts carrying the explicit auto-sign term; the platform's signature is by the currency-service identity and is labelled as an auto-sign execution in the event, never as a party's signature; an auto-signed transfer is still subject to §3.5 ordering and §3.2 validation at execution; either party can cancel the auto-sign term by mutual signed amendment before the day-28 moment. No other part of §3 depends on the unresolved readings.

### 3.8 Option: atomic batch escrow (Drew's proposal, not a ruling)

Drew's proposal, in his words' substance: clients require all such contracts to be submitted to us by the pending-unit owner; the other party countersigns only transactions escrowed to us, with all chains up to the original also escrowed, so participants know they will either get units or rejected units as we process the entire batch atomically.

Designed as **Variant E (escrowed batch)** alongside the sequential-chain design of §3.1–§3.7 (**Variant S**):

- Under E, a trade does not execute when its option is countersigned. Instead, every contract in a chain — the initial grant's escrow consent, each option, each countersignature — is **submitted to the platform and held in escrow**: the pending amounts are moved into a per-chain escrow account (same ledger; a gateway-controlled account per chain) as each link is submitted.
- When the whole chain up to a redemption point is escrowed and countersigned, the platform processes **the entire batch atomically**: one linked transfer batch that settles every link in order — each party receives either the units (vested/settled value at the batch's settlement point) or **rejected units** (Drew's term in the proposal; whether "rejected units" and "revoked units" are the same thing is U6, §4) if their link fails validation at processing time. No link settles unless every link settles; a failed batch unwinds the escrow back to the pre-batch holders, with signed records for every step.
- Under E the §3.3 cancel option and §3.7 auto-sign become batch events: a cancel joins the batch as a link; the auto-sign term authorises the platform to countersign an unexercised contract *into the escrow* at day 28.

Trade-offs:

| | Variant S (sequential chain) | Variant E (atomic batch escrow) |
| --- | --- | --- |
| Settlement | Each link settles when its ancestors complete; early links are final before later links are even signed | Nothing settles until the whole chain is escrowed; then all-or-nothing |
| Counterparty risk | A later link can fail after earlier links settled — parties bear chain risk link by link (mitigated: queued execution, §3.5) | Participants know ex ante they get units or rejected units — Drew's stated reason for the proposal |
| Liquidity / latency | Positions keep moving; vesting is never delayed by an incomplete downstream chain | Value sits in escrow until the chain completes; a stalled downstream link freezes the whole chain's value |
| Complexity | Reuses ordinary transfers + queued execution | New escrow account machinery, batch composition across many parties, unwind paths, escrow-time chargeback interaction (an adverse event landing mid-escrow must pick a winner) |
| Failure surface | Partial completion states exist and must be explainable | Atomicity removes partial states but concentrates failure: one bad link rejects the batch for everyone |

**Recommendation: adopt Variant S (sequential chain) for v1, and record Variant E as the designed v2 option.** In one line: the sequential chain delivers Drew's ruled mechanism — chained contracts, ordered execution, contract-required — with machinery the base design already has, while escrow's all-or-nothing guarantee mainly insures against a multi-party chain depth v1 grant programmes are unlikely to reach before the simpler design is proven. If Drew's answer to the proposal is "no, escrow is the point," §3.8 is specified enough to build from, and the switch does not change §§1–2 or the option-contract schema — escrow consumes the same contracts.

## 4. UNRESOLVED — clarifications needed from Drew

Lifted from the ruling comments on #2043 and #2296. Each item quotes Drew's words verbatim, then states the specific clarification needed. Nothing in §§1–3 interprets these phrases; where a section needed a working reading to proceed, it is marked as such there and cross-referenced here.

- **U1 (§2).** *"named like tiger beetle would be"* — What does this phrase name, and how? (The supply account? The cap exception? The currency?) Until answered, no name in this design relies on it.
- **U2 (§2).** *"finite except transfers from the account for supplying currency"* — Do transfers out of the supplying account count against the finite cap, or are they exempt from counting? §2.3 recommends "they count, net of returns"; the invariant wording in §2.4 has a one-sentence alternative if the answer is "exempt."
- **U3 (§3.0, §3.3).** *"not after (14 days from Init)"* — What is **Init**: the initial grant event, the option contract's creation, the trade, or something else? And is the "originator" who holds the cancel option the initial grantee (this design's working reading) or the issuer?
- **U4 (§3.3).** *"which also attaches the notBefore trade from pending units to units"* — What exactly is attached, and to what? §3.3's working design (the exercise pins the remainder's vesting `not_before`) is a guess at the shape, not a reading of the words.
- **U5 (§3.7).** *"at 28 days notAfter signature is enabled, we automatically sign and put the transfer at 14 days"* — 28 days counted from what? And "put the transfer at 14 days": 14 days measured from what, effective when?
- **U6 (§3.3, §3.8).** *"rejected units"* (escrow proposal) vs *"revoked units"* (cancel option) — Are rejected units and revoked units the same currency/class, or different things? §1 designs revoked units; nothing designs rejected units until this is answered.
- **U7 (§1, raised by this design, not from Drew's text).** Three currencies: three independent signed definitions (Reading A), or three balance classes presented and ruled as three currencies on one ledger (Reading B, recommended in §1.5)? Drew's text names the three currencies without stating which structure he intends.

Two transcription readings, recorded on the issues and used in §3, are also Drew's to correct if wrong: "oresubmit signs" read as "resubmit signed" and "ckuntersigns" read as "countersigns."

## 5. What changes in the base design if this extension is accepted

- Invariant 2 is replaced by the §2.4 wording (supply account; cap accounting).
- Invariant 9 gains the §3 tradability carve-out: pending value may change holder under §3 without becoming spendable; clawback allocation after a trade follows the option contract's explicit field (§3.4).
- Invariant 11 gains per-currency/per-class snapshot roots (§1.4).
- Phase 6 (real-service grant lifecycle) gains the §3 mechanism as a sub-phase, shipped disabled until the §4 clarifications land — the base terms' config defaults (the ones the #2065 build issues read, per #2296) must encode "tradable only under chained option contracts" alongside the accepted base terms.
- New graph event types: `currency.pending_option` and its exercise/denial records (§3.1). These belong to phase 1's canonical-contracts work if the extension is accepted before phase 1 closes.

## References

- Rulings: [#2043 comment 6721988851](https://github.com/developing-today/code/issues/2043#issuecomment-6721988851); [#2296 comment 6721993178](https://github.com/developing-today/code/issues/2296#issuecomment-6721993178). Drew's ruling text is quoted verbatim in both.
- Base design: `doc/2026-11-01_design_id2043.md` (PR #3886) — §§1–8, Invariants 1–12, phases 1–7.
- #2042 grant lifecycle (`doc/2026-10-30T11-55-00Z_design_grant_lifecycle.md`); #2065 refundable-grants design (`doc/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers/`); #2040 TigerBeetle mechanics (no cross-ledger transfer); #2063 conversion engine (linked batches, inventory accounts).
