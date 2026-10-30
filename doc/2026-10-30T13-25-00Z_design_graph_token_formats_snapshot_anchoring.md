# Design: graph token object formats + snapshot anchoring

- **Issue:** #2251 (spec follow-on from the #2041 investigation, queue task 3406)
- **Inputs:** #2041 (token issuance in the ID graph — object model, abuse cases 1–7),
  #2040 (TigerBeetle backend: currency = bare `u32` ledger, mint = transfer out of a
  designated issuer account, conversions = application-composed linked transfer pairs,
  graph object ID stamped in `user_data_128`), #2030 (counting views decide what
  counts), #2037 (attestation pattern reused for swap agreements)
- **Status:** design spec only. No permanent feature is built on the unreviewed #2041
  investigation (program rule, epic #2056). Implementation work is filed separately.
- **Revision (2026-10-30, queue task 3954):** all seven Codex P2 findings on
  PR #2741 are addressed in text — the cross-author credit frontier (§2),
  ledger identity normalization (§3), the conversion-aware reconciliation
  join (§3), a single minor-unit rate convention (§1.5/§5), cap accounting
  that counts executed conversions before receipt publication (§5 step 6),
  an explicit executor-clock expiry policy (§1.5/§5 step 2), and a dedicated
  non-monetary `token.receipt` kind (§1.8). A second Codex round on the
  amended text added three more, also addressed: the issuance burn frontier
  (cap checks evaluated over causal ancestry, §2), definitive-only release
  of graph-native cap reservations (§5 step 6), and anchored execution
  states so a service-rejected intent consumes no spending authorization
  (§2). Round 3 added two more: the mint-source account is excluded from
  both balance roots and reconciled by an exact offset equation (§3), and
  recovery distinguishes complete from partial leg sets — a one-sided
  graph-native swap yields the signed claim, never a success receipt
  (§5 step 6). Placement: this doc lives in the
  repo-root `doc/` store and is reachable at `pkgs/id/doc/` because
  `pkgs/id/doc` is a tracked symlink to `../../doc` — the #2251 placement
  requirement is met through that symlink, per repo convention.

## 0. Conventions inherited from `pkgs/id`

Every token object is an ordinary signed event in the author's chain, built with
`chain::sign` (`src/chain.rs`): a p2panda header carrying `ChainExt`, a body committed
to by BLAKE3 hash, canonical CBOR throughout (a non-canonical encoding is refused at
decode, exactly as for every other kind). Nothing in this spec adds a new envelope,
a new signature scheme, or a new identity type.

- The token payload is the event **body** (CBOR map, keys as below). `ChainExt.kind`
  carries the object type; `ChainExt.group` is unused by token objects (`None`);
  `ChainExt.after` carries the author's causal frontier as usual, and token objects
  that must cite a frontier (offers executed, snapshots) cite event IDs in the body
  as specified per type — the header frontier is the author's, the body's is the
  *subject's*, and conflating them is a validation error.
- **IDs.** An object ID is its event ID (BLAKE3 over the encoded header, signature
  included — the existing definition). Where an ID must fit TigerBeetle's
  `user_data_128` (128 bits), the stamped value is the **low 128 bits of the event
  ID**. Collision bound: accidental collision needs ~2^64 stamped objects per ledger;
  a deliberate collision requires a BLAKE3 second-preimage against a fixed target
  prefix, which is not a practical attack. Reconciliation (§3) therefore treats the
  128-bit stamp as a join key, and any join producing two different full event IDs
  for one stamp is a hard reconciliation failure, never a silent pick.
- **Amounts** are unsigned 128-bit integers of minor units at the currency's fixed
  `scale` (decimal places, fixed by the definition; per #2040 a token launched at
  scale 2 cannot later subdivide, so definitions should choose generously).
  Zero-amount objects are invalid except where a type says otherwise (a burn of 0
  is invalid; a snapshot is not an amount object).
- **Rational rates** are `{num: u128, den: u128}` with `den > 0`, stored
  **gcd-reduced** so that equal rates have identical encodings and therefore
  identical object IDs. 0.8123 is `8123/10000` reduced to `8123/10000` (already
  coprime); a float appears nowhere in any token object.

## 1. Wire formats

Kinds are namespaced `token.*`. All bodies are CBOR maps with exactly the keys
listed; unknown keys are refused (the `deny_unknown_fields` discipline of the
existing `Wire` forms).

### 1.1 `token.define` — currency definition

Signed by the issuer; appears on the issuer's chain.

| key | type | notes |
|---|---|---|
| `name` | string | display name ("coins"); NFC, non-empty. Never an identity. |
| `symbol` | string | display ticker; may be empty |
| `scale` | u8 | decimal places, 0–18, fixed forever |
| `supply` | `{"capped": u128}` \| `"open"` | cap is on *counted issued − burned* (§2) |
| `binding` | `"graph"` \| `{"ledger": u32}` | graph-native, or ledger-anchored with the TigerBeetle ledger id from #2040 |
| `memo` | string, optional | issuer's human description |

**Currency ID derivation.** `definition_hash = BLAKE3(canonical CBOR of the body)`.
The currency ID is the pair `(issuer public key, definition_hash)`; its compact
form, used as a map key and in UIs after the petname, is
`BLAKE3(issuer_pubkey ‖ definition_hash)`. Redefining a currency — even with one
field changed — produces a different currency. There is no edit operation.

### 1.2 `token.issue` — issuance

Signed by the issuer, on the issuer's chain.

| key | type | notes |
|---|---|---|
| `currency` | hash | the compact currency ID (§1.1) |
| `amount` | u128 | > 0 |
| `to` | pubkey | first holder |
| `memo` | string, optional | |

There is no other mint path: counted supply is a fold over `token.issue` minus
`token.burn` (§2), so supply is auditable by anyone holding the issuer's chain.

### 1.3 `token.transfer`

Signed by the sender, on the sender's chain.

| key | type | notes |
|---|---|---|
| `currency` | hash | compact currency ID |
| `amount` | u128 | > 0 |
| `to` | pubkey | recipient |
| `seq_ref` | u64 | the sender's chain sequence number this transfer spends from — i.e. the position at which the sender asserts their fold covers it (see §2 ordering) |
| `memo` | string, optional | `accept_payment` predicates match on this (§4) |

### 1.4 `token.burn`

A transfer whose `to` is the issuer is a redemption, not a burn. A burn is its own
kind so supply accounting never has to infer intent:

| key | type | notes |
|---|---|---|
| `currency` | hash | |
| `amount` | u128 | > 0 |
| `seq_ref` | u64 | as in `token.transfer` |

Signed by the holder; reduces counted supply when applied (§2). Burning to a
dedicated burn identity is expressible as a plain transfer and does **not** reduce
counted supply — only `token.burn` does. (This asymmetry is deliberate: it keeps
the supply fold a pure function of kinds, with no special-cased identities.)

### 1.5 `token.offer` — rate offer

Signed by the offerer (issuer or any holder with a balance in the source currency).
Each direction is a separate offer; an offer says nothing about the reverse rate.

| key | type | notes |
|---|---|---|
| `from` | hash | currency given |
| `to` | hash | currency received |
| `rate` | rational | §0; **destination minor units per source minor unit** — the only rate convention in this spec; §5 step 3 applies it directly, with no scale adjustment |
| `per_tx_cap` | u128 | max `from` amount per execution |
| `total_cap` | u128 | max cumulative `from` amount across all executions |
| `expires` | u64 | wall-clock expiry, **Unix epoch seconds (UTC)**, after which executions are invalid. The inherited event envelope carries no timestamp (`chain::sign` supplies no clock value), so expiry is judged against the executor's clock under the §5 step 2 policy — it is an execution-time check, never a fold input |
| `takers` | `"public"` \| list of pubkeys | eligible takers |
| `supersedes` | hash, optional | a previous offer by the same offerer for the same pair; see §2/§5 frontier rule |

An offer is revoked by superseding it (a new offer with `supersedes` set, possibly
with `total_cap` already exhausted) — there is no separate revoke kind, so the
frontier check in §5 is the only revocation semantics an executor must implement.

### 1.6 `token.convert_receipt` — conversion receipt

Emitted by the executor (the currency service for anchored currencies; the taker
for graph-native swaps, countersigned as part of the swap agreement, #2037 pattern).

| key | type | notes |
|---|---|---|
| `offer` | hash | the offer **event ID** — a specific version, never "the current offer" |
| `offerer_frontier` | hash | the offerer's chain head the executor verified at execution time (§5) |
| `from_amount` | u128 | debited from the taker |
| `to_amount` | u128 | credited to the taker, after rounding (§5) |
| `spread_amount` | u128 | the rounding/spread leg, 0 if none; always explicit, never folded into `to_amount` silently |
| `leg_a` | hash | event ID of the `from`-leg transfer (graph) or its stamped `user_data_128` value (anchored) |
| `leg_b` | hash | as above, for the `to` leg |
| `conversion` | hash | the shared conversion ID stamped on both anchored legs (§3) |

### 1.7 `token.snapshot` — issuer snapshot

Signed by the issuer — or, in a central-issuer deployment, by the service ID
(which signer, and the cadence, are open questions routed to #2043, §6).

| key | type | notes |
|---|---|---|
| `currency` | hash | compact currency ID |
| `frontier` | list of (pubkey, u64, hash) | per-author (author, last seq, head event ID) the fold covers — the snapshot's "as-of" |
| `issued` | u128 | counted issued at the frontier |
| `burned` | u128 | counted burned at the frontier |
| `balance_root` | hash | §3 construction |
| `ledger_pos` | u128, optional | anchored only: TigerBeetle position (latest transfer timestamp/id high-water) the ledger side was read at |
| `ledger_accounts_root` | hash, optional | anchored only: hash over the ledger account counters read at `ledger_pos`, computed **after identity normalization** (§3) so its leaves hash the same holder pubkeys as `balance_root` |

### 1.8 `token.receipt` — non-monetary payment acknowledgment

Emitted by an `accept_payment` host (§4) to record that a payment was
acknowledged and an effect taken. It is **not** a movement of funds and must
never be folded as one:

| key | type | notes |
|---|---|---|
| `payment` | hash | event ID of the `token.transfer` being acknowledged |
| `effect` | string | the effect taken, in the app's vocabulary (e.g. a #2037 match ID, a service-grant reference) |
| `memo` | string, optional | |

A receipt has **no `amount`, no `currency`, and no `to`** — the closed schema
refuses those keys, so no folder can mistake a receipt for a transfer.
Receipts are excluded from the §2 fold entirely (no balance effect, no supply
effect), and for anchored currencies a receipt creates **no**
ledger-execution obligation and carries **no** `user_data_128` stamp: it is a
graph-side record only. (Acknowledging a payment as a `token.transfer`
companion — the earlier §4 wording — would have subjected the acknowledgment
to §1.3's mandatory positive amount and §2's balance fold: a second financial
transfer instead of a record of one, plus a ledger obligation for anchored
currencies. This kind exists so that never happens.)

## 2. Fold rules (graph-native balances and supply)

The fold is a deterministic function of the applied event set, using the
`event_store` applied/void resolution that already decides what counts for every
other kind (#2030's counting views are the consumer of these rules).

**Ordering.** Events fold in chain order per author (sequence number, backlink
chain). Cross-author ordering matters exactly where one object's validity
depends on another author's object: a transfer's funding can include credits
issued or sent by other authors, and a conversion depends on the offer. The
fold therefore proceeds in **causal order**: an object is folded only after
every object in its causal ancestry (the transitive closure of its header
`after` frontier) has been folded. Within that constraint, author chains fold
in sequence order and no other global order is needed — matching how the rest
of the graph already folds.

**The credit frontier (which cross-author credits fund a spend).** A
`token.transfer` or `token.burn` by holder H is funded by exactly: H's own
prior applied objects, plus every applied `token.issue` / `token.transfer`
with `to = H` whose event is in the spend's **causal ancestry**. A credit
that is not an ancestor — one H received "later" in wall-clock terms or in
another author's chain order, but which the spend does not cite through its
frontier — does **not** fund the spend, even if a folder happens to hold it.
This is what keeps the fold deterministic across folders: ancestry is fixed
by the signed headers, so every folder holding the ancestry computes the
same funding set. A folder missing part of the ancestry cannot yet decide
the object — it is `Pending` (§4) — and must not guess in either direction.
`seq_ref` (§1.3) positions the spend on H's own chain; the credit-frontier
rule positions its funding.

**Balance of holder H in currency C** = Σ applied `token.issue` with `to = H`
+ Σ applied `token.transfer` with `to = H` − Σ applied outbound
(`token.transfer` + `token.burn` signed by H).

**Transfer/burn validity (applied vs void).** A `token.transfer` or `token.burn`
is **applied** iff, folding in causal order up to and including this object,
the sender's balance — computed over the credit frontier defined above —
never goes negative at this step: the applied inbound in the frontier minus
prior applied outbound covers `amount`. Otherwise it is **void**: retained as
signed evidence (it is exactly the proof in abuse cases 1–2), counted
nowhere. This is the validity/counting split #2041 specified: the object
*asserts*, the fold *decides*. Because validity depends only on the object's
causal ancestry, two folders holding the same ancestry always agree — a
transfer cannot be applied for one honest folder and void for another, and a
folder missing ancestry reports `Pending` instead of deciding.
`token.receipt` objects are not folded at all: they carry no amount and
affect neither balances nor supply (§1.8).

**Fork treatment.** If a sender's chain forks (two objects at one sequence
number), every object on both branches from the fork point onward is **void**
until the fork resolves under the graph's existing chain-resolution rules; the
pre-fork prefix folds normally. A recipient's acceptance policy (§4) decides how
much un-anchored, fork-exposed value to act on — the fold itself takes no risk
position. Overdrafts compose with forks the same way: an object that is applied
on one branch and void on the other is void while the fork is unresolved.

**Supply and the cap.** Counted supply = Σ applied `token.issue` − Σ applied
`token.burn` for the currency. An issuance that would push counted supply above
a `capped` maximum is **void** (not applied) — and the cap check is evaluated
against the issuance's **causal ancestry**, not against whatever a folder
happens to hold: counted supply *for that issuance* = applied issuance minus
applied burns **within its ancestry**, plus its own amount, compared to the
cap. Burns outside the ancestry do not free capacity for it, exactly as
credits outside a spend's frontier do not fund it (§2 credit frontier) — a
concurrent independent burn therefore cannot make an issuance's validity
depend on fold order. Two issuances racing past the cap are both on the
issuer's own chain, so they are causally ordered by it: the earlier counts,
the later is void, deterministically. Supply at any snapshot frontier is the
same per-object rule evaluated over the frontier's event set, so honest
folders with identical events always agree on supply and balances. A counting view (#2030) that serves
balances **must refuse to count** over-cap issuance, overdrawn transfers, and
unresolved-fork objects; a view that counts any of those is non-conformant with
this spec, and `supply(currency)` (§4) must return the counted value, never the
asserted one.

**Anchored currencies.** The fold above still runs — it is the authorization and
audit layer — but the balance of record is TigerBeetle's. A graph object with no
corresponding ledger transfer (§3 join) is an *intent that never executed*: it
does not move the anchored balance, and `balance()` reads the ledger. Divergence
between the intent fold and the ledger is a reconciliation finding (§3), not a
fold rule change.

**Anchored execution states (a rejected intent consumes no authorization).**
For anchored currencies each outbound intent carries an execution state —
`pending` (signed, not yet finally answered by the service), `executed` (its
ledger transfer exists, §3 join), or `rejected` (the service finally refused
it: `ServiceRejected`, `InsufficientFunds`, §4). The anchored authorization
fold counts an outbound object against its holder **only while pending or
executed**: authorization balance = ledger balance − Σ pending outbound.
A `rejected` intent consumes nothing and blocks nothing — the holder may
immediately re-spend the same units, and the new payment's graph object is
applied in the anchored fold when it executes, satisfying §3 check 2. A
pending intent that has no ledger transfer after the service's execution
window transitions to `rejected` by the service's attestation (surfaced at
the next reconciliation, §3); until that transition it conservatively holds
its authorization, so a slow execution can never be double-spent against.
Without this rule a single refused payment would permanently freeze the
refused amount in the graph fold while the ledger still shows it spendable —
the two layers would contradict each other on every later payment.

## 3. Snapshot anchoring and the reconciliation join

**Balance-root construction.** Collect `(holder_pubkey, counted_balance)` for
every holder with a non-zero balance at the snapshot frontier, sort by pubkey
bytes, and compute a BLAKE3 binary Merkle root over leaves
`BLAKE3(pubkey ‖ balance_u128_le ‖ currency_id)`. Including the currency ID in
every leaf means a root can never be replayed across currencies.

**Identity normalization (the two roots must hash the same preimages).**
`ledger_accounts_root` is **not** computed over raw TigerBeetle account IDs:
a graph leaf hashes a holder pubkey while a raw ledger leaf would hash an
account ID — different preimages that can never yield equal roots for equal
balances, so un-normalized root equality (check 3) would fail for correctly
executed payments. Before building the comparable root, every ledger account
is mapped back to the canonical holder identity the graph uses:

- an account opened for a holder maps to that holder's pubkey (the
  account → pubkey assignment is recorded when the account is created and is
  part of the anchored deployment's configuration — reconciliation reads it,
  it does not invent it);
- the designated issuer **mint-source** account (#2040: issuance is a
  transfer out of it) maps to a reserved `mint-source` identity that is
  **excluded from both roots** and reconciled separately (next paragraph) —
  mapping it to the issuer's pubkey would mismatch by construction, because
  the ledger debits the mint source on issuance while the graph fold credits
  the recipient without debiting the issuer. (The issuer's *ordinary holder*
  account, if it holds units as a holder, maps to the issuer's pubkey as
  usual — mint source and holder accounts are distinct accounts.)
- any other ledger-only account (spread/fee collection, service float) must
  be named in the deployment configuration with its own stable identity, and
  is folded under that identity on **both** sides (its graph-side balance is
  normally zero), or check 3 fails by construction.

`ledger_accounts_root` is then the construction above over
`(holder_pubkey, ledger_balance)` pairs after this mapping. An account with
no mapping is a reconciliation failure, never silently dropped — a dropped
account is exactly where value would hide.

**The mint offset (reconciled beside the roots, not inside them).** The
mint-source account's ledger balance is checked by an exact equation
instead of the root: `mint_source_balance == −(executed_issued −
executed_burned)` at `ledger_pos`, where the executed figures are §3's
(executed intents only, per checks 1–2). TigerBeetle mint-source accounts
run negative by design (#2040); the equation is the whole reconciliation
for that account, and any deviation is the same class of finding as a root
mismatch. Symmetrically, the graph side never debits the issuer for an
issuance — both sides therefore represent the mint offset identically:
outside the holder roots, as a signed scalar checked exactly.

**What a snapshot proves.** For graph-native currencies, anyone with the full
object set recomputes issued/burned/root and compares — a lying snapshot is
provable by recomputation (abuse case 6). For anchored currencies the snapshot is
computed *from the ledger* at `ledger_pos` and published as a graph object; the
two roots and the join below are the check.

**The mechanical join.** Every anchored execution stamps its graph object ID
(low 128 bits, §0) in the TigerBeetle transfer's `user_data_128`:

- issuance → the mint transfer out of the issuer account (#2040) carries the
  `token.issue` stamp;
- payment → the transfer carries the `token.transfer` stamp;
- conversion → **both** legs carry the receipt's `conversion` ID (not the
  receipt's own event ID — the receipt is emitted after execution, the legs must
  be joinable before it lands), and the receipt records both leg references.

**Conversion-aware join.** Conversion legs deliberately do not carry their
own object stamps, so the per-object join below has an explicit conversion
case. A `conversion` ID resolves through a **conversion → receipt lookup**:
the receipt whose `conversion` field equals the stamp. The join then
validates the *complete leg set* that receipt names — `leg_a`, `leg_b`, and
the spread leg iff `spread_amount > 0` — with each leg's amount checked
against the receipt's `from_amount` / `to_amount` / `spread_amount`. A leg
set that is incomplete, or whose amounts disagree with the receipt, fails
the join exactly as a mismatched single transfer does. Legs bearing a
`conversion` stamp whose receipt has not landed by the snapshot frontier
are **pending conversions**: enumerated in the reconciliation report,
counted against the offer's cap per §5 step 6, and neither passed nor
failed until the receipt (or a §5 step 6 recovery record) lands.

Reconciliation at a snapshot is then a three-way mechanical check, in order:

1. **Graph → ledger:** every applied anchored object at or before the frontier
   has exactly one ledger transfer with its stamp, of the same amount, between
   the accounts the binding assigns. (Conversion legs are the one exception to
   "exactly one transfer": they are validated as a complete leg set via the
   conversion-aware join above.) Missing or mismatched ⇒ the service failed
   to execute an authorized intent (or executed it wrongly); the ledger is
   truth for balances, the graph is truth for what was authorized, and the
   discrepancy names which side is wrong.
2. **Ledger → graph:** every ledger transfer in the currency at or before
   `ledger_pos` has a stamp that resolves to an applied graph object — or,
   for conversion legs, to a `conversion` ID whose complete leg set validates
   via the conversion-aware join (a pending conversion at the frontier is
   reported as pending, not passed).
   An unstamped or unresolvable transfer is value moved with no authorization
   on record — the anchored form of abuse case 1, and the check that exposes
   service collusion.
3. **Roots:** the fold's `balance_root` over *executed* objects equals
   `ledger_accounts_root`. (The intent fold may legitimately differ by
   unexecuted intents; those are enumerated by check 1, so the equality is
   over the executed subset — the snapshot's `issued`/`burned` are counted
   (executed) figures, with pending intents reported in the service log, not
   the snapshot.)

Any failure is a published finding, not a retry loop: snapshots chain (each
names its frontier), so a failed reconciliation is bounded to the interval
since the last good snapshot.

**Cadence options** (the choice is #2043's, §6): per-N-issuances, time-based,
or on-demand before large acceptances (§4 policy tiers). The format supports
all three unchanged; `frontier` makes any cadence auditable after the fact.

## 4. App-function API surface

One surface for both bindings; an app never branches on `binding`. Signatures
are language-neutral (the Rust implementation lives in `pkgs/id` beside the
other app functions); amounts are minor units.

| function | signature | semantics / errors |
|---|---|---|
| `define_currency` | `(name, symbol, scale, supply, binding) → CurrencyId` | Signs `token.define` on the caller's chain. Errors: `InvalidScale`, `NameNotNFC`. No uniqueness check on name — by design (§1.1). |
| `issue` | `(currency, to, amount, memo?) → ObjectId` | Issuer only: `NotIssuer` if the caller is not the definition's issuer. Anchored: also submits the mint (§3). A void-if-counted issuance (over cap, graph binding) is still signed and returned, with `Counted = false` in the result — callers must be able to *see* the cap refusal, not just fail. |
| `award` | `(currency, to, amount, memo) → ObjectId` | A `token.transfer` from the caller's pool identity; the memo carries the cause (e.g. a #2037 match ID). Identical mechanics to `pay`; the separate name exists so operator code reads as prize-pool-out, and indexers can distinguish awards by kind of caller intent. Errors as `pay`. |
| `pay` | `(currency, to, amount, memo?) → ObjectId` | Signs `token.transfer`. Errors: `UnknownCurrency`, `ZeroAmount`. **Insufficient funds is not an error at signing time** — the transfer is signed and its counted status is a fold result; the result reports `Counted | Pending | Void` as known locally. Anchored: the service executes or refuses, and refusal (`InsufficientFunds`, `ServiceRejected`) *is* returned, because the ledger is truth. This asymmetry is the two bindings' one visible difference, and it is in the result type, not the signature. |
| `balance` | `(currency, of) → u128` | Counted balance: graph fold (graph binding) or ledger read (anchored). Never the asserted balance. |
| `supply` | `(currency) → (issued, burned, counted_supply)` | §2 counted figures; anchored: executed figures per §3. |
| `accept_payment` | `(currency, predicate{min_amount, memo_pattern}) → handler` | Registers the app's acceptance hook. On a matching transfer the host verifies, per binding (below), then fires the app's effect and emits a signed **`token.receipt`** (§1.8) naming the payment object ID and the effect taken — a non-monetary record kind, excluded from the fold and from ledger execution. Refund/revocation of what a payment bought is #2042's subject and is **not** specified here. |
| `convert` | `(offer_id, amount) → ConversionReceipt` | Executes §5. Errors: `OfferNotFound`, `OfferSuperseded`, `OfferExpired`, `CapExceeded`, `NotEligibleTaker`, `InsufficientLiquidity` (anchored, atomic — no partial state), `CounterpartyLegMissing` (graph binding, after the agreement window — the receipt is replaced by the signed claim, §5). |
| `snapshot` | `(currency) → SnapshotObject` | Latest signed snapshot at or before the local frontier; `NoSnapshotYet` if none. Verification of a snapshot is recomputation (§3), never trust in the signer alone. |

**Binding-specific verification inside `accept_payment`** (the steps differ;
the handler contract does not):

- *Graph binding:* verify the signature and envelope; confirm the sender's
  chain is unforked at the transfer's position; fold the sender's chain and
  confirm the transfer is applied (funded) — or apply the app's configured
  risk tier (act on asserted-but-unfolded transfers only below a value
  threshold, per abuse case 2).
- *Anchored binding:* verify the signature; confirm via the service that a
  ledger transfer with this object's stamp exists and is final; the fold
  check is replaced by the §3 join for this single object.

## 5. Conversion execution checklist

An executor (service or taker) performs these steps in order; a failure at any
step aborts before any leg is submitted.

1. **Cite the offer version.** The execution names the offer by event ID
   (§1.6). "The current offer for the pair" is not a valid citation.
2. **Frontier freshness.** Read the offerer's chain frontier *at execution
   time*: the cited offer must be the latest non-superseded offer for its
   (offerer, pair) at that frontier, unexpired, with per-tx and cumulative
   caps not exhausted by already-executed conversions citing it (counted per
   step 6 — receipts, durable reservations, and recovered executed legs,
   never receipts alone). A superseded or
   revoked offer fails here, cleanly, before value moves (abuse case 4).
   **Expiry clock policy:** `expires` (§1.5) is Unix epoch seconds, judged
   against the executor's own clock — for anchored conversions the service's
   clock is authoritative; for graph-native executions the taker's clock
   governs its own execution, with a ±300 s tolerance for counterparty clock
   skew. An executor MUST refuse an offer expired under its own clock, and
   MUST NOT execute one inside the tolerance window on the strength of the
   counterparty's clock alone. Expiry is an execution-time policy check
   only: folders never void a receipt retroactively on clock grounds, since
   no chain object carries the time an execution happened.
3. **Eligibility and amounts.** Taker ∈ `takers`; `amount ≤ per_tx_cap`;
   cumulative executed + amount ≤ `total_cap`. Compute the `to` amount from
   the exact rational in the wire format's units (§1.5: destination minor
   units per source minor unit): `to_amount = floor(from_amount × num / den)`,
   directly in minor units. There is **no scale adjustment** — the two
   definitions' `scale` values are already inside the rate as quoted, and
   adjusting again would transfer a different amount for the same signed
   offer.
4. **Rounding and spread.** Rounding is always **in the liquidity provider's
   favour** (the floor above; the taker receives the floor, never a rounded-up
   figure). Any spread the provider charges is a **separate leg**: a third
   transfer of `spread_amount` in the `to` currency from the conversion flow
   to the provider, named in the receipt (§1.6) — per #2040 §2's linked-pair
   pattern extended honestly: the pair stays the pair, the spread is its own
   auditable object, and a receipt whose legs don't sum is invalid on its
   face.
5. **Execute.** Anchored: submit the linked transfer pair with the shared
   `conversion` ID in `user_data_128` on both legs (and the spread leg, if
   any, stamped with the same ID); TigerBeetle's linked semantics make the
   pair atomic — both land or neither (abuse case 5). Graph-native: **not
   atomic, and the spec does not pretend otherwise** — both parties first
   sign a bilateral swap-agreement object (#2037 attestation pattern) naming
   the offer, amounts, and a completion window; each leg is a transfer
   citing the agreement; if a leg never appears, agreement + completed leg
   constitute a signed claim against the defaulter.
6. **Reserve first; emit the receipt last; count executions, not receipts.**
   Caps are never computed from receipts alone: a crash between step 5 and
   receipt emission would otherwise let a later execution re-spend the same
   allowance (value moved, cap unspent), and a graph-native taker could
   withhold its own receipt after both legs land. The order is:
   - **Before step 5**, the executor durably records a **cap reservation**:
     the `conversion` ID, the cited offer, and `from_amount`. Anchored: the
     service's durable execution log, written before the legs are submitted.
     Graph-native: the signed swap agreement (step 5) is itself the
     reservation — it exists before either leg.
   - Step 2's cumulative-cap check counts **receipts + outstanding
     reservations + executed-but-unreceipted legs**.
   - **Recovery:** before executing against an offer again, an executor
     scans its reservations that have no receipt — anchored, by querying the
     ledger for transfers stamped with the reserved `conversion` IDs;
     graph-native, by folding for the agreement's legs. A **complete** leg
     set found (anchored legs are atomic, so this is the only anchored
     case; graph-native: both legs, plus the spread leg iff
     `spread_amount > 0`) ⇒ the conversion executed: emit the (late)
     receipt and convert the reservation into it. A **partial** graph-native
     leg set (one leg landed, the other never did — the one-sided execution
     step 5 explicitly permits) ⇒ the conversion did **not** complete:
     recovery emits no receipt, the reservation's accounting is retained,
     and on window expiry the agreement + completed leg stand as the signed
     claim against the defaulter (§5 step 5; §4's `CounterpartyLegMissing`)
     — a receipt asserting both amounts were exchanged would be false.
     No legs found ⇒ release the reservation as
     unexecuted **only on a definitive condition**: anchored, the ledger
     query is definitive because the service is the only executor — legs
     cannot appear later without it, so an empty query closes the
     reservation. Graph-native, absence of legs is NOT definitive while the
     signed agreement's completion window is still open (the parties may
     yet submit them): the reservation is released only when the window has
     expired under the step 2 clock policy, or when both parties have
     signed a cancellation of the agreement. Releasing earlier would let a
     second conversion consume the allowance and the delayed legs then
     exceed `total_cap`. A reservation is never silently expired while its
     legs are unaccounted for.
   - **Emit the receipt** (§1.6) naming the offer version, the frontier
     checked in step 2, both leg references, and the spread. A conversion
     counts as happened when it *executed*; the receipt is the public
     record of that fact, and the reservation/recovery machinery above is
     what makes "executed" knowable even when a receipt is late.

## 6. Abuse-case trace (#2041 cases 1–7)

| # | case | spec mechanism | residual risk |
|---|---|---|---|
| 1 | Issuer inflates past an announced cap | §2: over-cap issuance is void in fold order; counting views must refuse it. Anchored: §3 check 2 exposes any ledger movement with no authorized object. | Graph-native: provable, not prevented — accepted, as #2041 stated. Anchored prevention requires service honesty at execution time; the join makes dishonesty evident after the fact. |
| 2 | Offline double-spend | §2: overdrawn transfers void; fork branches void from the fork point; §4 risk tiers bound what a recipient acts on before anchor/sync. | Prevention is policy, not protocol — accepted and named. |
| 3 | Name confusion / squatting | §1.1: identity is (issuer, definition hash); name is display only; every object carries the compact currency ID. | UI discipline (issuer petname + name shown together, balances never merged across issuers) is a client requirement, restated here so it is not lost. |
| 4 | Stale/revoked offer executed | §5 steps 1–2: version citation + frontier freshness at execution time; supersession is the only revocation and the check is mechanical. | An executor that skips step 2 is non-conformant; conformance is testable against the checklist. |
| 5 | Offerer can't fund the rate | Anchored: linked-pair atomicity — insufficient liquidity aborts both legs. Graph: swap agreement + claim. Offers and failures are on record (reputation). | Graph-native non-atomicity is inherent; accepted, per #2041. |
| 6 | Lying snapshot | §3: roots are recomputable (graph) or contradicted by the ledger (anchored); snapshot is an auditable assertion, not a trust root. | Light clients that never spot-check accept signer risk between checks — inherent to the snapshot model Drew specified. |
| 7 | Issuer key compromise | Partially mitigated in-spec: caps bound mint damage even with the key (§2); definitions keep their identity across rotation because identity includes the definition hash, not a key generation. | **Not solved here.** Rotation mechanics are an open question for #2043 (§7) — a rotation object signed by the old key naming a successor is sketched in #2041 but its counting-view semantics (which generation a view pins) are genuinely unsettled. |

## 7. Open questions — routed to the #2043 synthesis, not decided here

Per the acceptance criteria, these are listed for #2043 and **not** decided
silently. #2043 remains gated on #2042 (tokens for real services) as well.

1. **Snapshot signer for the developing-today deployment** — issuer key, or a
   dedicated service ID? (Determines who can be compelled to snapshot, and
   whose compromise forges one.)
2. **Snapshot cadence** for the central-issuer deployment — per-N, time-based,
   or acceptance-triggered (§3 supports all three; the operational choice is
   a deployment decision).
3. **Issuer key-rotation mechanics** (abuse case 7): the rotation object's
   format, and which key generation(s) a counting view pins for definitions
   issued pre-rotation.
4. **Fold/index service**: do graph-native currencies get a shared fold/index
   service, or stay purely client-side? (Affects §4's `Pending` state
   duration in practice, not the API.)
5. **Receipt/handler interplay with #2042**: `accept_payment` receipts (§4)
   will meet refund/revocation objects whenever #2042 lands; the receipt
   format here names the payment and effect but deliberately leaves the
   reversal vocabulary to that design.

## 8. What this spec does not do

- No implementation. Follow-on implementation issues are filed separately;
  this issue delivers the spec only.
- No refund/revocation policy (#2042 owns it), no currencies-v1 synthesis
  decisions (#2043 owns them, gated), no changes to TigerBeetle findings
  (#2040 stands as the ledger-side input).
