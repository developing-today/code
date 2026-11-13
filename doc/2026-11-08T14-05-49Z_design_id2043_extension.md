# Design: ID tokens v1 — extension for Drew's 2026-11-08 rulings

- Date: 2026-11-08
- Status: **design proposal** — extends the settled design; no implementation code is included in this PR, and nothing here changes a ruling. The rulings themselves are already recorded on the issues (links below); this document designs how the three modifications fit the model and names what still needs Drew's clarification before implementation.
- Issues: [#2043 — ID currencies v1](https://github.com/developing-today/code/issues/2043) (rulings recorded in [comment 6721988851](https://github.com/developing-today/code/issues/2043#issuecomment-6721988851)) and [#2296 — grant refund window and pending spendability](https://github.com/developing-today/code/issues/2296) (rulings recorded in [comment 6721993178](https://github.com/developing-today/code/issues/2296#issuecomment-6721993178)), parent epic #2056.
- Rulings note: Drew, 2026-11-08 09:13 CST — **"double entry"**: the §1 three-class model is a double-entry structure (one ledger, three token classes as account classes, every movement a balanced journal entry). §1 was rewritten on this ruling; it supersedes the earlier draft's balance-classes recommendation, and clarification U7 is answered by it (see §1.7 and §4).
- Rulings note: Drew, 2026-11-08 09:26 CST — **tokens, not currencies**: the things this design calls "currencies" are **tokens** — representations of grants and capabilities offered for a server or a world; they hold no value by definition (his analogies: gold in World of Warcraft, RP in League of Legends, Robux in Roblox). "Currency" is TigerBeetle's word and survives only where TigerBeetle's own terms are meant. See §0.
- Rulings note: Drew, 2026-11-08 09:27 CST — **future extensions approved as deferred work, on one condition**: nothing in v1 may be written in a way that blocks maintaining and transitioning the system and its features; deferral is fine so long as each deferred item is at least a little thought through architecturally. See §6.
- Base design: `doc/2026-11-01_design_id2043.md` (PR #3886). Where this extension and the base design disagree, the base design stands until Drew accepts this extension; the disagreements are listed explicitly in §5.

## 0. What a token is — terminology and nature

Drew's clarification, 2026-11-08 09:26 CST (quoted verbatim on #2043):

> <and exact use isn't currency necessarily they are merely tokens, currency is just the tigerbeetle term and often used for representation grants and capabilities offered fora server or world. they hold no value, by definition, same as gold in wow or rp in lol 9r mayberoblox

- A **token** is a representation of grants and capabilities offered for a server or a world — in-world entitlements. It is not money, not stored value, and not redeemable for anything outside the system that issued it, and it **holds no value by definition**. Drew's analogies: gold in World of Warcraft, RP in League of Legends, Robux in Roblox — closed, in-world units whose meaning is exhausted by what the world itself grants for them.
- The word **"currency"** enters this design only because it is **TigerBeetle's term**: TigerBeetle denominates each of its ledgers in what it calls a currency. Where our own prose speaks, the word is **token**. The mapping: one TigerBeetle `ledger` = one token system (one grant programme, §1.3); TigerBeetle's per-ledger denomination ("currency") = that system's token. Where the base design or TigerBeetle says "currency," read "token."
- The base design (`doc/2026-11-01_design_id2043.md`) uses "currency" throughout, including the programme's name on #2043 ("ID currencies v1"). The base design is **not** rewritten by this extension; under this clarification, its "currency" reads as "token" everywhere. This extension's prose uses "token" except where it quotes the base design, Drew's rulings, or TigerBeetle's own vocabulary.
- Names that are **identifiers, not prose**, are unchanged: the `currency.*` graph event types (`currency.pending_option`, `currency.rate_authorisation`, …) and the currency-service component name stay as they are. Renaming identifiers is a migration, not a terminology pass, and §6's transition rule applies to it like anything else.
- The three classes Drew named are **token classes**: **Units**, **Pending**, and **Revoked** keep their account-class names (§1.3) and are framed everywhere as classes of one token — three kinds of entitlement balance, never three monies.

### What the clarification changes, section by section

- **§1 / the DE invariants: unchanged.** Double-entry conservation (DE-1/DE-2) is about the integrity of grant accounting — every entitlement is somewhere, was issued against the endowment, and moved only by a balanced entry — not about value. Nothing about a token holding no value weakens or alters any DE invariant.
- **§2 / supply and cap: reframed, mechanics unchanged.** Issuance is _entitlement issuance_, not monetary issuance: the supply account holds unissued entitlements, and the finite cap is a scarcity and integrity control on how many grant/capability tokens a programme may create — the control that keeps a world's grants meaningful inside that world — not a monetary policy.
- **§3 / trades: reframed, mechanics unchanged.** A trade transfers _entitlements_ — grants and capabilities — between holders; nothing with value changes hands by virtue of the token moving. The "consideration" leg (§3.1, §3.4) names what the taker gives in exchange and never prices the token: either units of the same token, settled in-system in the same balanced batch, or an off-ledger consideration recorded by hash, which the platform does not price, value, or adjudicate.
- **§6 / future extensions: wording follows the nature.** Where earlier discussion said "user-created currencies," read **user-created token systems**: a user runs a server or a world and defines its grant/capability tokens. Nothing else about §6's gaps or seams depends on the tokens being currency-like.

## The rulings this extension designs

Drew ruled on all six policy questions from the base design's §8 on 2026-11-08. Three answers confirm the recommendations as stated (Q3 recovery key, Q4 emergency revocation, Q6 open-source boundary) and need no design work. Three modify the design and are the subject of this document:

- **Q1 (name):** the neutral label "units" ships and branding is deferred, as recommended — **and** the model has **three token classes: units, pending units, and revoked units** (his wording: "3 currencies"; §0) (§1). A follow-on ruling the same morning (09:13 CST) settles the structure: **double entry** (§1).
- **Q2 (supply):** a finite cap for all v1, as recommended — **except transfers from the account for supplying currency** (§2). His sentence ends "named like tiger beetle would be"; that phrase is unresolved (§4, U1) and is not interpreted anywhere in this document.
- **Q5 (grant terms, #2296):** the base terms are accepted (14-day Steam-direct hold, pending never spendable, per-processor chargeback horizon, debt never silently expires, for-cause-only revocation for first-party premium) — **except pending units may be traded when both parties accept/sign**, under a chained option-contract mechanism (§3). The final sentence of his ruling, proposing atomic batch escrow of the whole chain, ends with a question mark in his text and is recorded on the issues as an **open proposal, not a ruling**; §3.7 designs it as an option and recommends on it.

Drew's full ruling text is quoted verbatim on both issues (links above) and is the authority for everything below. Where his text does not settle a point, this document says UNRESOLVED and §4 lists the exact clarification needed; no unresolved point is silently designed around.

## 1. Three token classes on one double-entry ledger

### 1.1 What the rulings change — and what "double entry" supersedes

The base design has one ledger-anchored token per signed definition (it says "currency"; §0). "Pending" is a **state** of a token balance: a purchase grant is a pending TigerBeetle hold, unspendable until the refund window closes, then it posts into the holder's ordinary balance. "Revoked" is an **event/policy**: issuer revocation, gateway suspension, per-definition revocation policy. Neither is a token class.

Drew's Q1 ruling names **three currencies — units, pending units, and revoked units** — in §0's terms, three **token classes** of one token. The first draft of this extension designed that ruling two ways (three distinct token definitions vs. three balance classes on one ledger), recommended the balance-classes reading, and put the fork to Drew as clarification U7. Drew's follow-on ruling on 2026-11-08 — **"double entry"** — supersedes that fork and that recommendation. The model is now fixed:

- **One** signed definition, **one** TigerBeetle ledger per grant programme (the base design's identity rule is unchanged).
- The three are **three classes of accounts** on that ledger — every holder's position is kept in a units account, a pending account, and a revoked account, presented and ruled as the three token classes Drew named in every wallet view, statement, and API response. They are never summed into one number and are never fungible with each other.
- **Every movement of tokens is a balanced double-entry journal entry**: a debit and an equal credit, posted as one atomic operation. Nothing is created or destroyed inside the books; tokens change class, holder, or both, only through a named journal entry type (§1.4), and the only boundary entries are issuance out of the supply account and the one-time genesis endowment that funds it (§1.3).

This is stronger than the superseded balance-classes draft in exactly the way Drew's ruling intends: classes alone said _where_ tokens sit; double entry also says _how they may move_ — never by editing a balance, never by a one-sided adjustment, only by a balanced pair whose other side names where the tokens came from.

### 1.2 The double-entry rule

- A **journal entry** is a set of TigerBeetle transfers submitted as one linked batch that **balances**: total debits = total credits. Simple movements are a single debit/credit pair; multi-leg events (a trade plus its units consideration, §3.4) are one batch whose legs balance in aggregate and post or fail together (base Invariant 6 pattern).
- **Convention.** Entries are written `Dr <account>` / `Cr <account>` in TigerBeetle's own sense: the debited account is the transfer's source (`debit_account_id`), the credited account is its destination (`credit_account_id`). A holder account's balance in its token class is `credits_posted − debits_posted` (plus the pending fields, §1.6); the supply account's balance is read the same way.
- **No one-sided operations exist.** There is no mint primitive, no balance edit, no admin adjustment. Issuance is an entry (Cr supply → Dr holder class, §1.4). Correction of a posted entry is a new **compensating entry** referencing the original — the base design's rule for posted conversions, generalised to every entry type. The journal is append-only.
- **Every entry carries its authorisation**: the graph event that authorises it, joined by `user_data_128` as usual (base Invariant 7). An entry with no authorising event is not submittable.

### 1.3 Chart of accounts (per programme ledger)

| Account                        | Held by               | Meaning                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------ | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Genesis endowment** (contra) | The ledger itself     | Balances the one-time endowment that funds the supply account at definition creation. It is the only account permitted a negative standing balance (debits exceed credits by exactly the endowed amount); that negative figure _is_ the endowed issuance of the programme, and it moves only under a new signed definition event (e.g. a cap amendment). |
| **Supply**                     | Currency service only | Unissued tokens. Its balance is the unissued remainder of the endowed issuance. See §2 — exactly one per token, named in the signed definition.                                                                                                                                                                                                          |
| **Units** (one per holder)     | Each holder           | Settled tokens: the **units** class. Spendable and transferable per the base design.                                                                                                                                                                                                                                                                     |
| **Pending** (one per holder)   | Each holder           | **Pending units**: grant tokens inside their refund/chargeback window, plus positions received under §3 trades. Never spendable (§1.5, DE-5).                                                                                                                                                                                                            |
| **Revoked** (one per holder)   | Each holder           | **Revoked units**: tokens cancelled or revoked out of the live classes, kept on the _holder's_ statement so exactly what was taken, and why, stays visible to them. Terminal (§1.5, DE-6).                                                                                                                                                               |
| **Escrow** (one per chain)     | Gateway               | Exists only under the §3.8 Variant E escrow design; in the sequential design (Variant S, recommended for v1) no escrow accounts are created. Holds pending tokens escrowed for an atomic batch.                                                                                                                                                          |

A user's balance in each token class is simply the balance of their account of that class — three balances, three accounts, reported separately in every snapshot (per-class roots, §1.5 DE-2). Scale is shared across all classes (the definition's scale), so every 1:1 life-cycle entry is exact integer equality with no rounding path.

### 1.4 Journal entries by event type

| Event                                             | Debit                                       | Credit                            | Notes                                                                                                                                                                                                                                |
| ------------------------------------------------- | ------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Genesis endowment** (definition creation, once) | Genesis endowment                           | Supply                            | Amount = the programme's authorised issuance (§2). The single boundary entry that brings tokens into the books; thereafter the books are closed (§1.5, DE-2).                                                                        |
| **Issuance — grant**                              | Pending (grantee)                           | Supply                            | The grant enters as pending units inside its window (§3.0). Counts against the cap net of returns (§2.4 — unchanged; Drew did not override §2).                                                                                      |
| **Issuance — direct**                             | Units (recipient)                           | Supply                            | Non-grant issuance (e.g. a purchase settled immediately), where the definition allows it.                                                                                                                                            |
| **Vesting** (window closes, no adverse event)     | Units (holder)                              | Pending (holder)                  | Same holder, class change only. Executed by the gateway at window close.                                                                                                                                                             |
| **Cancel option exercise** (§3.3)                 | Revoked (originator)                        | Pending (originator)              | 1:1, originator-initiated, inside the 14-days-from-Init window. An exit-from-the-chain device, not early vesting: the originator receives revoked units, never units.                                                                |
| **Revocation — in-window** (adverse event)        | Revoked (holder)                            | Pending (holder)                  | Authorised by the issuer's revocation policy event.                                                                                                                                                                                  |
| **Revocation — post-activation**                  | Revoked (holder)                            | Units (holder)                    | Only up to the holder's units balance (DE-4: no negative holdings). Any shortfall becomes explicit grantee debt — a graph/control-plane record (base Invariant 9), **not** a ledger entry: debt never appears as a negative balance. |
| **Trade of pending units** (§3.4)                 | Pending (taker)                             | Pending (grantor)                 | Holder changes, class does not; only under a countersigned chained option contract. If the consideration is units, a second leg posts in the same balanced batch: Dr Units (grantor) / Cr Units (taker).                             |
| **Payment / redemption**                          | Units (recipient or issuer service account) | Units (payer)                     | The only spend path, and it can draw only on Units accounts (DE-5).                                                                                                                                                                  |
| **Burn / return**                                 | Supply                                      | Units (holder)                    | Under a signed burn event; restores cap headroom net of returns (§2.3). Final.                                                                                                                                                       |
| **Escrow in / out** (Variant E only, §3.8)        | Escrow (chain) / Pending (holder)           | Pending (holder) / Escrow (chain) | Submission escrows; batch settlement or unwind posts the reverse and onward legs as one balanced batch.                                                                                                                              |

### 1.5 Invariants, restated in double-entry terms

- **DE-1 — Trial balance.** Every journal batch balances (Σ debits = Σ credits), and therefore the ledger balances cumulatively at every instant: Σ over all accounts of (debits posted − credits posted) = 0, the genesis contra account included. TigerBeetle enforces the per-transfer half structurally (a transfer _is_ a debit and a credit); the gateway enforces the batch half — a multi-leg event whose legs do not balance in aggregate is rejected before submission.
- **DE-2 — Conservation.** Between genesis and any later signed endowment amendment, the closed total is constant: `Supply + Σ Units + Σ Pending + Σ Revoked + Σ Escrow = endowed issuance`, and `Genesis endowment = −endowed issuance`. Life-cycle entries (vest, cancel, revoke) move tokens between classes without changing any class-crossing total; §2.3's reconciliation equation is this invariant read at a snapshot. A mismatch is a durable finding, never silently dropped (base Invariant 7). Snapshots report per-class roots; a snapshot reporting only the combined total does not satisfy base Invariant 11 under this extension.
- **DE-3 — Cap as a supply-account constraint.** The finite cap (§2.4 wording, unchanged) is enforced as a balance constraint on one account: the Supply account's balance may never go negative, issuance entries are valid only while it covers them, and cumulative net issuance (issuance entries minus returns) never exceeds the cap stated in the signed definition.
- **DE-4 — No negative holdings.** No holder account of any class may go negative. Enforced by TigerBeetle account flags (§1.6) _and_ gateway validation; a revocation or payment larger than the balance posts only up to the balance, with the remainder handled off-ledger as debt (base Invariant 9) — never as a negative entry.
- **DE-5 — Pending never spendable, as an account-type restriction.** Debits out of a Pending account are permitted only to the destinations the journal names: Units of the _same holder_ (vesting), Revoked of the _same holder_ (cancel/revocation), Pending of another holder (a §3 trade under a countersigned option contract), or Escrow (§3.8). No payment, redemption, or conversion path may name a Pending account as its source. This is an allowlist on (source class, destination class, event type) enforced by the gateway — "pending never spendable" (§3.6) is a property of the account type, not a per-request check. Revoked accounts are stricter still (DE-6).
- **DE-6 — Revoked is terminal.** No journal entry debits a Revoked account except one: a compensating relief entry under a new, separately authorised operation referencing the original revocation (the good-faith relief path, base §8 Q5: signed write-off / case-by-case relief). Revoked tokens are never spendable, never tradable (including under §3), and never re-enters circulation directly — this keeps "revoked" from becoming a laundering hop back into units.

### 1.6 TigerBeetle mapping — what is native, what is not

**Maps natively:**

- **Accounts and balanced transfers.** TigerBeetle's account model _is_ double-entry shaped: every transfer names a debit account and a credit account for equal amounts on one `ledger`, and accounts carry `debits_pending / debits_posted / credits_pending / credits_posted`. The chart of accounts (§1.3) is a set of TigerBeetle accounts on the programme's single ledger; the three token classes are three accounts per holder, distinguished by account metadata the gateway assigns at creation. DE-1's per-transfer half and DE-4 are enforceable with TigerBeetle's own account flags (`debits_must_not_exceed_credits` on holder accounts and on Supply, so neither a holder nor the supply account can be overdrawn; `history` retained for audit).
- **Two-phase transfers for the vesting leg.** TigerBeetle transfers are inherently two-phase (pending → posted/voided, with timeouts). The vesting entry can be pre-created at grant time as a TigerBeetle _pending transfer_ Pending → Units with the window as its timeout: it posts at window close, and an in-window adverse event voids it and posts the revocation entry instead. Two different senses of "pending" must not be conflated in code or docs: **pending units** are a _token class_ (the Pending account class, this section), while a TigerBeetle _pending transfer_ is a _transfer state_ — the design uses the second as machinery for the first's life cycle, and the grant hold of the base design (#2042) already works this way.

**Needs the ledger-anchored layer from the base design (not native to TigerBeetle):**

- **Authorisation and identity.** TigerBeetle has no authentication, no signatures, no identity (#2040). Which journal entries may exist at all — the §1.4 entry-type table, the §1.5 DE-5/DE-6 (source, destination, event-type) allowlist, the option-contract rules of §3, cap semantics beyond Supply's balance floor — is gateway policy: the graph event authorises, the gateway is the only TigerBeetle client and writer, and `user_data_128` joins each transfer to its authorisation, exactly as the base design's ledger-anchored binding prescribes.
- **The genesis endowment.** TigerBeetle accounts start at zero and transfers must balance, so the endowment is a real transfer out of the Genesis endowment contra account (§1.3) — an ordinary account whose flags deliberately permit its one negative standing balance. Nothing in TigerBeetle creates it; the signed definition event does.
- **Cross-token conversion.** Life-cycle entries never leave the ledger, so #2040's "no cross-ledger transfer" is not in play inside a programme. Conversions _between token systems (programmes)_ still use the base design's machinery (signed `currency.rate_authorisation`, pinned quote, linked batch, §2063) unchanged.
- The unresolved phrase "named like tiger beetle would be" (U1, §4) remains **UNRESOLVED verbatim**; nothing in this mapping interprets it or relies on it.

### 1.7 What this supersedes, and U7's disposition

- **Superseded:** the first draft's §1 in full — the two-readings fork (three definitions vs. balance classes), its §1.5 recommendation of the balance-classes reading, and its §1.4 "rules common to both readings" (whose substance survives, restated as DE-5/DE-6 and the shared-scale rule in §1.3). Also superseded: every cross-reference in §§2–3 to the two readings (updated to point at this section).
- **U7 is answered and removed.** U7 asked whether Drew intended three independent signed definitions or three balance classes on one ledger. "Double entry" dissolves the fork rather than picking a side: it fixes the structure at the journal level — one ledger, three token classes as account classes, every movement a balanced entry — which is the balance-classes _account structure_ with the definitions question made moot for v1, because no life-cycle operation needs a second definition or a second ledger under either reading once entries are balanced. The one residue is noted, not re-asked: if a class is ever to become an independently definable token (different issuer, scale, or cap as policy), it would be split into its own double-entry ledger at that time, and §§1.2–1.5 apply per ledger unchanged. §4 records the removal.

## 2. The supply account and the cap exception (Q2)

### 2.1 What Drew ruled

Finite cap for all v1 — "except transfers from the account for supplying currency." The base design's Invariant 2 reads: _"Every currency has a finite, explicit issuance cap in v1; cap checks are deterministic under causal order and forks."_ The design has no supply account and no exception. This section defines both, and §2.4 gives the replacement invariant wording.

### 2.2 The supplying account, defined

- Each ledger-anchored token (one token system per programme ledger under §1's double-entry model) has exactly one **supply account**: a gateway-controlled TigerBeetle account, created with the token's registry entry, named in the signed definition by its registry account ID, and controlled by no key other than the currency service's.
- The supply account is the **only** account a token's units can be issued into existence through. Issuance is a transfer out of the supply account; the supply account is funded only by the definition process itself (its opening balance is the authorised issuance, §2.3). No other account may go negative, and no other account's transfers create tokens.
- The supply account is not a holder account: it cannot receive ordinary transfers, cannot be a trade counterparty under §3, cannot be debited by clawback, and its balance is published in every supply audit (base phase 4 already produces supply-audit/reconciliation reports; the supply account balance becomes a required line).

### 2.3 Mint/burn accounting, and whether supply transfers count against the cap

Drew's text does not say whether transfers out of the supply account count against the finite cap — the ruling comment on #2043 records this as unresolved, and it is carried here as U2 (§4). This section designs the accounting so that either answer can be adopted by changing one sentence of the invariant (§2.4), and recommends the reading that keeps the cap meaningful:

- **Authorised issuance (the cap)** is a number in the signed definition: the maximum total amount that may ever leave the supply account, net of returns, over the token's life. Recommended semantics: **transfers out of the supply account count against the cap; transfers back into it (burns, voided grants returned by the issuer's own operations, expired unissued allocations the gateway returns) restore headroom.** Under this reading the exception in Drew's sentence is about _who may move tokens without a holder-style authorisation_ — the supply account's transfers are issuance events authorised by the definition itself, not exempt from counting. The alternative reading — supply transfers do not count at all — makes the cap unenforceable (any amount could be issued by routing it through the supply account, which is the only issuance path anyway), so this document does not recommend it; it is recorded as the literal alternative in §2.4 for Drew's U2 answer.
- **Mint** is not a ledger primitive here: "minting" N units = transferring N from the supply account to a holder account (or, at grant issuance, to a holder's pending account/class). The graph issuance event, signed by the issuer, authorises it; the gateway validates definition, cap headroom, and registry mapping before submitting, exactly as base §3 already requires for anchored issuance.
- **Burn** is a transfer into the supply account from a holder account under a signed burn event (the §1.4 burn entry); the burn is final, reduces circulating supply, and — under the recommended semantics — restores cap headroom only when it returns to the supply account. Debts and write-offs never touch the supply account: a signed write-off (base Invariant 9) is a graph/control-plane record, not a ledger movement.
- **Reconciliation:** at every snapshot, `supply account balance + Σ holder balances (all classes) = endowed issuance` (invariant DE-2, §1.5), and `cumulative net issuance ≤ cap`. A mismatch is a durable finding, never silently dropped (base Invariant 7).
- **Nature (§0):** none of this is monetary issuance. The supply account holds unissued _entitlements_; the cap is a scarcity and integrity control on grant creation, and "supply" never names money awaiting circulation.

### 2.4 Replacement wording for Invariant 2

> **2. Definition and supply:** scale/binding are immutable for an identity. Every token has a finite, explicit issuance cap in v1, stated in its signed definition. Each token has exactly one supply account, named in the definition and controlled solely by the currency service; tokens enter circulation only by transfer out of the supply account under a signed issuance event, and such transfers count against the cap net of returns to the supply account. Cap checks are deterministic under causal order and forks. No other account may create tokens, and the supply account's balance and cumulative net issuance are published in every supply audit.

If Drew's U2 answer is the literal alternative (supply transfers exempt from counting), the fourth sentence is replaced by: _"transfers out of the supply account are not counted against the cap; the cap binds all other issuance paths"_ — and the design then needs a separate control on the supply account's opening balance, since that balance becomes the de facto cap. This document flags that consequence rather than choosing it.

### 2.5 The unresolved phrase

Drew's Q2 sentence ends: "named like tiger beetle would be". Recorded verbatim; **UNRESOLVED** (U1, §4). It is not interpreted in this section or anywhere in this document — in particular, nothing here assumes it names the supply account, the cap, or the exception.

## 3. Trading pending units — the #2296 amendment (Q5)

### 3.0 Shape of the mechanism

Base terms stand as accepted: pending units vest after the source's window (14 days Steam-direct), are never spendable, and carry the per-processor chargeback horizon and the rest. Drew's amendment adds one capability: **a pending position may be traded — transferred to another party — when both parties accept/sign, and only inside a chain of signed option contracts that starts at the initial grant.** Everything in this section is ledger-anchored only; graph-native tokens have no pending grants (base design §4) and so no pending trading.

Vocabulary used below, all new graph event types in the currency vocabulary (closed schemas, per base §1):

- `currency.pending_option` — the **option contract**: a signed event created at, or chained from, the initial grant, authorising one trade (or one cancel exercise, §3.3) of a specified pending position.
- **Chain** — the sequence grant → option₁ → trade₁ → option₂ → trade₂ → …, where each option names the previous trade/grant event as its parent. A pending position is tradable only while it carries an unbroken chain back to its initial grant event ("only trade able as a chained item from initial grant by us").
- **Init** — the anchor event/time the cancel window runs from. Drew did not define it (U3, §4); this design uses _the initial grant event_ as the working reading everywhere a start point is needed, and marks each such use.

### 3.1 The option contract

A `currency.pending_option` event carries:

- the pending position it covers: grant event ID, current holder (the option grantor), amount (exact integer, scale of the definition), and the position's chain parent (grant event or previous trade event);
- the counterparty (the option taker) by identity — an option is not a bearer instrument; only the named counterparty may countersign;
- the consideration terms, as references: trades under this mechanism exchange pending units for consideration **outside** the pending class — either units (the settled token class) paid through an ordinary anchored transfer/conversion, or an off-ledger consideration recorded by hash. (On "consideration" under §0: a token holds no value, so consideration never prices the token — it names what the taker gives in exchange: units of the same token settled in-system, or something wholly outside the system, recorded by hash only and neither priced nor adjudicated by the platform.) The option event names which, and if units, the linked payment transfer ID once executed (§3.4). The mechanism does not create a pending-for-pending barter path in v1;
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
- **Exercise:** the originator signs an exercise referencing the cancel option; the gateway executes the 1:1 pending → revoked movement (§1.4: the balanced journal entry pending → revoked) for the specified amount, up to the position's remaining pending amount. Partial exercise is allowed; the unexercised remainder keeps its chain and its own still-live cancel option until the deadline.
- **Deadline:** exercise is valid **not after 14 days from Init** (working reading: the initial grant event's timestamp). After the deadline the cancel option lapses — it cannot be revived by stale gateway state (base Invariant 4).
- **The notBefore attachment:** exercising the cancel option "also attaches the notBefore trade from pending units to units." Working design (the phrase itself is UNRESOLVED, U4): the exercise event embeds a vesting term stating that any portion of the position **not** cancelled continues to vest by the ordinary pending → units transition at its `not_before` (= window close), i.e. exercising a partial cancel re-affirms and pins the vesting schedule for the remainder rather than leaving it implicit. The cancel exercise receipt therefore records three things atomically: amount revoked, amount remaining pending, and the remainder's vesting `not_before`. If Drew means something else by the attachment, only this subsection changes.
- The cancelled amount lands in revoked units (§1.4: terminal). The originator receives revoked units, not units — the cancel option is an exit-from-the-trade-chain device, not an early-vesting device: it unwinds the originator's exposure (and, per §3.5, the downstream chain's expectations are settled in revoked units under the escrow variant, §3.7) without making pending tokens spendable or settled early.

### 3.4 Execution of a trade

A trade under a countersigned option executes as one gateway operation:

1. Validate per §3.2.
2. Move the pending amount from grantor to taker (the §1.4 trade entry: a same-ledger transfer between pending accounts — no conversion is involved in a trade, the token does not change).
3. If the consideration is units, execute the linked units payment in the same operation batch (linked transfers, base Invariant 6 pattern) so the pending leg and the payment leg post or fail together. If the consideration is off-ledger, the trade posts on the countersignature plus the grantor's signed attestation of the consideration terms' hash; the platform does not adjudicate off-ledger payment, and the option's terms must say so in the client UI.
4. Emit the trade event as the new chain head for the position, naming the option event as its authorisation (`user_data_128` join, base Invariant 7).

The taker receives **pending units**, with the original window and chargeback exposure intact: trading does not reset, shorten, or extend the vesting clock, and does not launder the position's provenance — the chain _is_ the provenance. Post-activation clawback rules (base Invariant 9) still attach per the grant terms; who bears a shortfall after a trade — grantor, current holder, or per the option's terms — must be an explicit option field, defaulting to the current holder at the time the adverse event lands, with the option contract free to allocate it otherwise. (This allocation question is a design consequence of tradability that Drew's text does not address; it is flagged here as a design decision taken by this document, not attributed to Drew.)

### 3.5 Chained redemption and ordering

Drew: "each users contracted other users can redeem against that users account using their signed chained contract, they can [resubmit] signed countersigned contracts to us and we will run as soon as all the chains up to them are done." ("oresubmit signs" in his text is read as "resubmit signed", a transcription guess recorded on #2296, not a ruling.)

- **Redemption** = presenting a countersigned option contract to the gateway to run the trade it authorises, including at vesting time presenting the chain to claim the vested units into the current holder's account.
- **Ordering rule:** a chain link (option exercise / redemption) runs only when **every link before it in its chain has completed** — the grant is final, each prior trade has posted, each prior option is consumed. The gateway evaluates a submitted contract against the chain state: if any ancestor link is incomplete, the submission is **queued, not denied** — Drew's "we will run as soon as all the chains up to them are done": the gateway holds the countersigned contract and executes it automatically when its last incomplete ancestor completes, in chain order, without the parties resubmitting. (Resubmission is also accepted and is idempotent — same contract, same effect, base Invariant 6.)
- A link whose ancestor is **denied or voided** (e.g. the underlying grant is voided by an in-window adverse event) can never complete; queued descendants are then denied with a signed denial naming the failed ancestor, and each party's client surfaces it. Value never moves on a broken chain.
- Chain state is a control-plane projection over the graph events, rebuildable like every other projection (base §2); the graph chain, not the projection, is the audit record.

### 3.6 "Pending never spendable" is preserved — and the #2296 rewording

Tradability does not touch spendability, and the distinction is mechanical, not rhetorical:

- **Spendable** means acceptable by a service as payment — a `token.receipt`-producing payment, a conversion, or any debit that settles a purchase. Pending units have no such path: no service endpoint accepts them, no rate authorisation may name the pending token class as a source, and the gateway rejects any payment intent drawn on a pending balance.
- **Tradable** means exactly one thing: transfer to a named counterparty under a countersigned chained option contract (§3.1–§3.4), receiving pending units still inside their window. The recipient gains no spending power they did not have; the position's restrictions travel with it.

#2296's Decision 2 currently reads (issue body, verbatim): _"Recommendation: **not spendable, at all.** Allowing purchases against pending balance reintroduces the clawback problem through the front door."_ — and its acceptance criterion asks for pending tokens "spendable or not, explicitly confirmed." The exact replacement wording this extension proposes for Decision 2, for Drew's acceptance on #2296:

> **Decision 2 — spendability and tradability of pending units.** Pending units are **not spendable, at all**: no service may accept them as payment, no conversion may draw on them, and no purchase may be made against a pending balance; allowing purchases against pending balance reintroduces the clawback problem through the front door. Pending units are **tradable** only under the chained option-contract mechanism of the #2043 extension design: both parties accept/sign, the option contract is a required input to any pending-unit trade (trades without one are denied), the position's window and restrictions travel with it unchanged, and a trade confers no spending power. Tradable is not spendable.

### 3.7 The 28-day auto-sign behaviour

Drew: "at 28 days notAfter signature is enabled, we automatically sign and put the transfer at 14 days."

What is settled by the text: there is a day-28 moment; at it, a `notAfter` signature becomes enabled; the platform ("we") automatically signs; and a transfer is placed "at 14 days." What is **not** settled (U5, §4): 28 days counted from what (working reading: from Init, the initial grant), and "put the transfer at 14 days" measured from what and effective when (from the auto-sign? from Init? a 14-day settlement delay on the auto-signed transfer?).

Designed with those two readings marked: at **Init + 28 days**, any option contract in the chain that was created with the auto-sign term and remains unexercised has its `not_after` signature **enabled** — the platform's countersignature, pre-authorised by the contract's own auto-sign term (both parties accepted this term when they signed; the platform is executing their instruction, not originating one) — and the transfer it authorises is placed with a **14-day settlement point** (working reading: effective 14 days after the auto-sign, i.e. the transfer posts at Init + 42 days under the two working readings combined). Safety rails that hold under any reading: auto-sign applies only to contracts carrying the explicit auto-sign term; the platform's signature is by the currency-service identity and is labelled as an auto-sign execution in the event, never as a party's signature; an auto-signed transfer is still subject to §3.5 ordering and §3.2 validation at execution; either party can cancel the auto-sign term by mutual signed amendment before the day-28 moment. No other part of §3 depends on the unresolved readings.

### 3.8 Option: atomic batch escrow (Drew's proposal, not a ruling)

Drew's proposal, in his words' substance: clients require all such contracts to be submitted to us by the pending-unit owner; the other party countersigns only transactions escrowed to us, with all chains up to the original also escrowed, so participants know they will either get units or rejected units as we process the entire batch atomically.

Designed as **Variant E (escrowed batch)** alongside the sequential-chain design of §3.1–§3.7 (**Variant S**):

- Under E, a trade does not execute when its option is countersigned. Instead, every contract in a chain — the initial grant's escrow consent, each option, each countersignature — is **submitted to the platform and held in escrow**: the pending amounts are moved into a per-chain escrow account (same ledger; a gateway-controlled account per chain) as each link is submitted.
- When the whole chain up to a redemption point is escrowed and countersigned, the platform processes **the entire batch atomically**: one linked transfer batch that settles every link in order — each party receives either the units (vested/settled tokens at the batch's settlement point) or **rejected units** (Drew's term in the proposal; whether "rejected units" and "revoked units" are the same thing is U6, §4) if their link fails validation at processing time. No link settles unless every link settles; a failed batch unwinds the escrow back to the pre-batch holders, with signed records for every step.
- Under E the §3.3 cancel option and §3.7 auto-sign become batch events: a cancel joins the batch as a link; the auto-sign term authorises the platform to countersign an unexercised contract _into the escrow_ at day 28.

Trade-offs:

|                     | Variant S (sequential chain)                                                                                                 | Variant E (atomic batch escrow)                                                                                                                                                |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Settlement          | Each link settles when its ancestors complete; early links are final before later links are even signed                      | Nothing settles until the whole chain is escrowed; then all-or-nothing                                                                                                         |
| Counterparty risk   | A later link can fail after earlier links settled — parties bear chain risk link by link (mitigated: queued execution, §3.5) | Participants know ex ante they get units or rejected units — Drew's stated reason for the proposal                                                                             |
| Liquidity / latency | Positions keep moving; vesting is never delayed by an incomplete downstream chain                                            | Tokens sit in escrow until the chain completes; a stalled downstream link freezes the whole chain's tokens                                                                     |
| Complexity          | Reuses ordinary transfers + queued execution                                                                                 | New escrow account machinery, batch composition across many parties, unwind paths, escrow-time chargeback interaction (an adverse event landing mid-escrow must pick a winner) |
| Failure surface     | Partial completion states exist and must be explainable                                                                      | Atomicity removes partial states but concentrates failure: one bad link rejects the batch for everyone                                                                         |

**Recommendation: adopt Variant S (sequential chain) for v1, and record Variant E as the designed v2 option.** In one line: the sequential chain delivers Drew's ruled mechanism — chained contracts, ordered execution, contract-required — with machinery the base design already has, while escrow's all-or-nothing guarantee mainly insures against a multi-party chain depth v1 grant programmes are unlikely to reach before the simpler design is proven. If Drew's answer to the proposal is "no, escrow is the point," §3.8 is specified enough to build from, and the switch does not change §§1–2 or the option-contract schema — escrow consumes the same contracts.

## 4. UNRESOLVED — clarifications needed from Drew

Lifted from the ruling comments on #2043 and #2296. Each item quotes Drew's words verbatim, then states the specific clarification needed. Nothing in §§1–3 interprets these phrases; where a section needed a working reading to proceed, it is marked as such there and cross-referenced here.

- **U1 (§2).** _"named like tiger beetle would be"_ — What does this phrase name, and how? (The supply account? The cap exception? The currency?) Until answered, no name in this design relies on it.
- **U2 (§2).** _"finite except transfers from the account for supplying currency"_ — Do transfers out of the supplying account count against the finite cap, or are they exempt from counting? §2.3 recommends "they count, net of returns"; the invariant wording in §2.4 has a one-sentence alternative if the answer is "exempt."
- **U3 (§3.0, §3.3).** _"not after (14 days from Init)"_ — What is **Init**: the initial grant event, the option contract's creation, the trade, or something else? And is the "originator" who holds the cancel option the initial grantee (this design's working reading) or the issuer?
- **U4 (§3.3).** _"which also attaches the notBefore trade from pending units to units"_ — What exactly is attached, and to what? §3.3's working design (the exercise pins the remainder's vesting `not_before`) is a guess at the shape, not a reading of the words.
- **U5 (§3.7).** _"at 28 days notAfter signature is enabled, we automatically sign and put the transfer at 14 days"_ — 28 days counted from what? And "put the transfer at 14 days": 14 days measured from what, effective when?
- **U6 (§3.3, §3.8).** _"rejected units"_ (escrow proposal) vs _"revoked units"_ (cancel option) — Are rejected units and revoked units the same currency/class, or different things? §1 designs revoked units; nothing designs rejected units until this is answered.

Two transcription readings, recorded on the issues and used in §3, are also Drew's to correct if wrong: "oresubmit signs" read as "resubmit signed" and "ckuntersigns" read as "countersigns."

- **U7 — REMOVED (answered by Drew, 2026-11-08).** U7 asked whether the three token classes were three independent signed definitions or three balance classes on one ledger. Drew's follow-on ruling — "double entry" — dissolves the fork: §1 now fixes one double-entry ledger with the three token classes as account classes and every movement a balanced journal entry (§1.7). The only residue (splitting a class into its own definition later) is recorded in §1.7, not re-asked.

## 5. What changes in the base design if this extension is accepted

- Invariant 2 is replaced by the §2.4 wording (supply account; cap accounting).
- Invariant 9 gains the §3 tradability carve-out: pending tokens may change holder under §3 without becoming spendable; clawback allocation after a trade follows the option contract's explicit field (§3.4).
- Invariant 11 gains per-class snapshot roots (§1.5, DE-2).
- §1's three token classes are implemented as double-entry account classes on one ledger (Drew's 2026-11-08 "double entry" ruling): the §1.4 journal-entry table and the §1.5 DE-1–DE-6 invariants join the phase 1 canonical-contracts work and the phase 4 anchored-transfer work.
- Phase 6 (real-service grant lifecycle) gains the §3 mechanism as a sub-phase, shipped disabled until the §4 clarifications land — the base terms' config defaults (the ones the #2065 build issues read, per #2296) must encode "tradable only under chained option contracts" alongside the accepted base terms.
- New graph event types: `currency.pending_option` and its exercise/denial records (§3.1). These belong to phase 1's canonical-contracts work if the extension is accepted before phase 1 closes.
- Terminology (Drew, 2026-11-08 09:26 CST): the base design's "currency" reads as "token" under §0; the base design itself is not rewritten.
- §6 records the deferred future extensions and the seam each one is left, under Drew's transition condition (2026-11-08 09:27 CST); nothing in §6 changes v1.

## 6. Future extensions — deferred, with seams

Drew approved this section on 2026-11-08 (09:27 CST) on one condition (recorded in the header rulings notes): it is fine to defer most things not yet covered, **so long as nothing is written in a way that blocks maintaining and transitioning the system and its features, and each deferred item is at least a little thought through architecturally**. This section is that thinking. For each gap: what is deferred, and the seam the current design already leaves — or must now explicitly leave — so the future change is a transition, not a rewrite.

**Architectural rule (Drew's condition, binding on v1):** every v1 mechanism must have a stated transition path to its foreseeable successor. A design point with no transition path is a defect in v1, not a decision deferred to later. Where the current text _does_ hard-wire something, this section names it as a known narrowing with its intended transition path (§§6.2, 6.5).

### 6.1 User-created token systems need anchored provisioning

- **Deferred:** who may register an anchored token with the gateway; who provisions its TigerBeetle ledger and supply account; who sets and monitors its cap; who pays for gateway capacity. (Graph-native tokens need none of this — a signed definition is already permissionless under the base design's identity model.)
- **Seam:** token identity is (issuer public key, definition hash), never a registry row — so admission can be added later as a gateway-side provisioning layer without touching token definitions or any existing token. Provisioning is a gateway-side concern; it does not reach into definitions. The known tension is §2.2's "supply account controlled solely by the currency service": the transition path is per-token supply accounts whose control is delegated to the token's issuer under gateway policy (the service retaining audit and suspension), which changes who holds which key — the chart of accounts (§1.3) and the journal (§1.4) do not change.

### 6.2 The binding needs a backend dimension

- **Deferred:** expressing _which_ anchored backend a token binds to. Today "ledger-anchored" means TigerBeetle, full stop, and §1 hard-assumes **one TigerBeetle ledger per programme** — a known narrowing. Drew's direction is the reverse of the current text: TigerBeetle becomes **a second ledger type**, used for the central backend, alongside the ledger type(s) already in use.
- **Seam:** the journal is the portable contract. Journal entries (§1.4) and the DE invariants (§1.5) are stated in backend-neutral accounting terms; §1.6 is explicitly a _mapping_ onto TigerBeetle — one of possibly several. A second ledger type is added by writing a second mapping and giving the definition's binding field a backend dimension; token identity (issuer key + definition hash) does not include the backend, so no existing token changes identity when the dimension appears. The one TigerBeetle-specific dependency is named: the vesting leg leans on TigerBeetle's native two-phase pending transfers with timeouts (§1.6). Another backend needs an equivalent hold primitive or a gateway-driven vesting scheduler; the journal entry it executes (Pending → Units at window close) is identical either way.

### 6.3 Cross-backend atomicity

- **Deferred:** a trade or conversion whose two legs execute on different ledger types, settling atomically. Today conversions assume one ledger's linked batch (base design §3; #2040: no cross-ledger transfer), and a §3 trade's legs post in one batch on one ledger (§3.4).
- **Seam:** §3's mechanics operate on signed graph events — options, trades, and chain state as a projection over events (§3.5) — not on ledger calls. The contract layer is backend-independent; only execution is per-ledger. The transition path is a gateway coordinator running a two-phase commit over per-backend legs, and §3.5's queued-not-denied rule already supplies the "hold until ancestors complete" semantics a coordinator needs. What v1 must not do: bake single-ledger batch IDs into the event schemas as if they were the trade's identity. The trade event is the identity; batch IDs are execution detail.

### 6.4 Ledger-type migration mechanics

- **Deferred:** moving a live token — balances, pending positions, open chains — from one backend to another. (§1.7 already contemplates the special case of splitting a class into its own ledger; migration is the general case, and it is also the path by which the central backend would adopt TigerBeetle under §6.2.)
- **Seam:** because every movement is a balanced journal entry and snapshots carry per-class roots (DE-2), migration is definable as: freeze at a snapshot root; bring the target backend up to the same per-class totals through its own genesis endowment (the §1.3 machinery, reused); cut over the registry mapping; resume. Chain state rebuilds from graph events on either side of the cut (§3.5). The v1 rule that keeps this possible: no mechanism may depend on TigerBeetle-internal IDs (transfer IDs, timestamps) as durable references — the `user_data_128` joins to graph events are the durable reference, and they are already backend-neutral.

### 6.5 Per-token policy surface

- **Deferred:** the §3 windows (14-day hold, 14-day cancel option, 28-day auto-sign) are global constants taken from Drew's rulings. User-created token systems will need them — and tradability itself, clawback-allocation defaults (§3.4), and auto-sign availability — as per-token choices set in the definition.
- **Seam:** definitions are signed, versioned events, and definition fields are additive: a field absent from a v1 definition reads as the v1 global constant, so promoting a constant to a definition field changes no existing token's meaning and needs no migration. The v1 implementation rule that keeps this seam real: every policy constant lives in exactly one named place, read through the definition — never inlined at call sites — so the promotion is a field addition, not a scavenger hunt.

### 6.6 "Rejected units" (U6) blocks escrow Variant E

- **Deferred:** Variant E (§3.8) stays an option on paper until U6 is answered — its failure output names "rejected units," a class §1 does not define.
- **Seam:** if rejected ≠ revoked, the addition is one more account class with its own DE-5/DE-6-style allowlist and its journal entries (escrow → rejected on batch failure; any relief path out of it) — the chart of accounts (§1.3) and the invariant set (§1.5) extend by addition, and no existing class changes meaning. §3.8's batch semantics are already written against escrow accounts and linked batches only, so defining the failure class completes Variant E without reworking it.

## References

- Rulings: [#2043 comment 6721988851](https://github.com/developing-today/code/issues/2043#issuecomment-6721988851); [#2296 comment 6721993178](https://github.com/developing-today/code/issues/2296#issuecomment-6721993178). Drew's ruling text is quoted verbatim in both.
- Base design: `doc/2026-11-01_design_id2043.md` (PR #3886) — §§1–8, Invariants 1–12, phases 1–7.
- #2042 grant lifecycle (`doc/2026-10-30T11-55-00Z_design_grant_lifecycle.md`); #2065 refundable-grants design (`doc/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers/`); #2040 TigerBeetle mechanics (no cross-ledger transfer); #2063 conversion engine (linked batches, inventory accounts).
