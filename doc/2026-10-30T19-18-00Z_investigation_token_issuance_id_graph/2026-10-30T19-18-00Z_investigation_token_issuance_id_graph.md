# Token issuance in the ID graph — investigation: per-ID issuance, issuer snapshots, authorized conversion rates

**Date:** 2026-10-30 · **Question owner:** Drew's "ID currencies / tokens" directive
(LIST 11b, 2026-10-28), program epic
[#2056](https://github.com/developing-today/code/issues/2056)
**Scope:** How any ID issues tokens under its own name in ID's own terms —
the issuance/transfer objects in the ID graph, how issuer snapshots anchor
graph state to the TigerBeetle ledger, and what a transaction-accepting
function looks like for apps and games. Ledger mechanics and the gateway are
[#2062](https://github.com/developing-today/code/issues/2062)'s; the
RateAuthorisation object design is
[#2290](https://github.com/developing-today/code/issues/2290)'s; the
currencies v1 synthesis that consumes this document is
[#2043](https://github.com/developing-today/code/issues/2043)'s.
**Method:** This is the wave-2 write-up of the investigation whose findings
were first recorded on
[#2041](https://github.com/developing-today/code/issues/2041) by queue task
3352 (2026-10-29). That pass verified the model against the codebase and
left the model on the issue; the document itself was never written. This
document formalises those findings and reconciles them with the designs
that landed after them: the currency control plane (#2062, closed, doc
`2026-10-29T12-42-21Z_design_currency_control_plane`), the centralised
exception (#2066, doc
`2026-10-29T12-20-00Z_design_currency_centralised_exception`), the
conversion engine ([#2063](https://github.com/developing-today/code/issues/2063)),
and the grant lifecycle spec (doc
`2026-10-30T11-55-00Z_design_grant_lifecycle.md`, from
[#2042](https://github.com/developing-today/code/issues/2042)). Where the
wave-1 model and a later design disagree, the later design wins and the
reconciliation is stated explicitly (§1). No proof of concept is built
here, and no permanent feature may be built on this investigation without
a review round (program rule, epic #2056).

---

## TL;DR verdict

**The model Drew sketched is coherent: every ID can issue tokens under its
own name, and the whole system needs exactly one object vocabulary, used
in two bindings.**

- **Token identity is never the name.** A currency is identified by its
  issuer and its signed definition object — anyone can issue a token
  called "coins"; names are display/petname-layer (§1).
- **Issuance is a signed graph object, not a hidden mint.** "This ID made
  100 tokens of itself and gave them to that ID" is an `Issuance` object
  on the issuer's chain; total issued is a fold over the issuer's issuance
  objects, so supply is publicly auditable (§2, §4).
- **Two bindings, chosen once per currency.** *Graph-native* tokens live
  entirely as graph objects (balances are folds; they work offline, and
  offline double-spend is detectable but not preventable). *Ledger-anchored*
  tokens use the graph objects as the authorization/audit layer and
  TigerBeetle as balance truth, cross-linked by `user_data_128` (§3).
- **A snapshot is a signed, checkable compression of state at a
  frontier** — total issued, total burned, a balance root, and (when
  anchored) the ledger position. It replaces global consensus with a
  named signer whose assertions are cheap to audit; a lying snapshot is
  provable, mechanically for anchored currencies and by recomputation for
  graph-native ones (§5). This is Drew's "snapshots from an official
  issuer instead of a full blockchain," made precise.
- **Conversions surface here as per-pair, per-direction rate offers.**
  ID 123 accepting ABC at 0.8123 is one signed offer; the reverse is a
  separate offer or none. Execution is #2063's linked transfer pair; the
  RateAuthorisation field schema is #2290's. This document owns only how
  offers and receipts appear in the graph and in the app API (§6).
- **Apps see one function surface either way** — `define_currency`,
  `issue`, `award`, `pay`, `accept_payment`, `convert`, `snapshot` — with
  the binding-specific verification spelled out behind each call (§7).
  Worked traces for both acceptance examples are in §8 and §9.

---

## 1. Currency identity, reconciled with the control plane

Wave-1 said: currency ID = (issuer ID, currency-definition hash). The
control plane design (#2062 §2.3–2.4) then introduced two more
identifiers: a registry-minted `currency_id` and the TigerBeetle `ledger`
u32. These are not three competing identities; they are three layers, and
this document fixes which layer each consumer uses:

| Layer | Identifier | Who mints it | Used by |
| --- | --- | --- | --- |
| Graph (semantic) | (issuer ID, definition-object ID) | The issuer, by signing the definition | Graph objects, wallets, this design |
| Registry (operational) | `currency_id` | The registry at registration (#2062 §2.3) | Gateway requests, app APIs |
| Ledger (mechanical) | `ledger` u32 | The registry allocator (#2062 §2.1) | TigerBeetle accounts/transfers only |

Bindings between the layers are themselves recorded: the registry row
holds `graph_object_id` pointing at the signed definition (#2062 §2.4 —
the graph object is canonical, the registry is a derived index and is
rebuildable by replaying the graph), and every ledger write stamps the
graph object ID in `user_data_128` (#2062 §7.1). Consequences:

- Graph objects in this document reference a currency by
  (issuer, definition) and never carry a `ledger` u32; the gateway
  resolves `currency_id → ledger_id` at submission time.
- Two issuers may both define a token with symbol `ABC` without
  ambiguity: the (issuer, definition) pair differs, and #2062 §2.3 makes
  the symbol unique only per issuer. UIs must display issuer petname +
  name together and never merge balances across issuers (§10, case 3).
- Scale is fixed in the definition object and is effectively immutable
  once balances exist (#2062 TL;DR); a token launched at scale 2 cannot
  later subdivide, so definitions should choose scale generously.

## 2. The graph objects

All seven objects are signed events in the graph, on per-author chains
like everything else in `pkgs/id` (the signed-event envelope is
`pkgs/id/src/chain.rs`; wire formats are the spec follow-on
[#2251](https://github.com/developing-today/code/issues/2251)'s job, not
this document's).

| # | Object | Signer | Content (semantic fields) | Counting rule |
| --- | --- | --- | --- | --- |
| 1 | Currency definition | Issuer | Name/symbol, scale, supply policy (`capped{max}` \| `open`), binding (`graph-native` \| `ledger-anchored`) | Canonical record of the currency; registry indexes it (#2062 §2.4) |
| 2 | Issuance | Issuer | Currency, amount, recipient, memo | Counts iff issuer's chain is unforked at that position and cumulative issuance ≤ cap |
| 3 | Transfer | Sender | Currency, amount, recipient, sender's chain position | Counts iff sender's chain is unforked and funded at that position (§3) |
| 4 | Burn | Holder | A transfer to the issuer or a dedicated burn identity | Reduces counted supply |
| 5 | Rate offer | Offerer (issuer or any liquidity holder) | From-currency, to-currency, exact rational rate, per-tx and total caps, expiry, eligible takers (`public` \| named set) | Live iff not superseded/revoked at the offerer's current chain frontier |
| 6 | Conversion receipt | Executing service / both parties | Offer (specific version/frontier), both leg references, conversion ID | Emitted on execution (§6); the audit record of a conversion |
| 7 | Issuer snapshot | Issuer — or, in a central-issuer deployment, the currency service ID | Currency, as-of frontier, total issued, total burned, balance root, ledger position when anchored | An auditable assertion, never a trust root (§5) |

Two rules cut across the table:

- **Rates are exact rationals, never floats.** 0.8123 is 8123/10000.
  (Field-level representation, including scale handling, is #2290's.)
- **Validity vs counting is the program-wide split** (#2030): a signed
  object is an assertion by its signer; whether it *counts* depends on
  chain unforkedness, funding, caps, and frontier freshness at the moment
  a view or a service evaluates it. Nothing in this document changes
  that split.

## 3. Two bindings, one vocabulary

The currency definition's `binding` field chooses, once, at definition
time. An app does not choose per transaction.

**Graph-native** (games, casual/community currencies). Issuance and
transfers are graph objects only; a holder's balance is a fold over
their inbound/outbound transfer objects. Fully offline and peer-to-peer.
The hard limit is stated plainly: **offline double-spend cannot be
prevented, only detected** — a sender who signs two transfers of the
same funds has forked their chain or overdrawn their fold, both provable
from the objects. Acceptance policy (§7) decides how much risk a
recipient takes before requiring an anchor.

**Ledger-anchored** (developing-today's central-issuer deployment,
anything redeemable for real services). The graph objects are the
authorization and audit layer; TigerBeetle is the balance truth,
exactly as #2040 specified and #2062 built the control plane for: the
gateway verifies the signed graph object, checks permission against the
registry (signature is *who*, registry is *what* — #2062 §4.2), then
submits the ledger transfer with the graph object ID stamped in
`user_data_128`, so the two records are permanently cross-linked and
reconciliation is a join, not an investigation (#2062 §7).

The centralised exception (#2066) bounds what "central issuer" means:
exactly one component — the currency service (TigerBeetle cluster +
control plane + gateway) — is centralised; identity, graph, content and
everything else stay P2P or serverless, and if the currency service is
down, currency operations stop and nothing else does. Its warning is
adopted here as a rule for this design's outputs: **TigerBeetle-backed
tokens are online currency; the graph-claims tier is not currency in
that sense, and the two tiers must never be presented to users as the
same thing.**

## 4. Issuance in ID terms

Per-ID issuance, end to end:

1. The issuer signs a **currency definition** (§2, object 1). For an
   anchored currency the gateway routes the signed definition through
   registration: registry record, allocated `ledger` u32, and the
   per-currency system accounts (issuer, liquidity, fee, burn) from the
   account factory (#2062 §2–3).
2. To issue, the issuer signs an **Issuance** object naming currency,
   amount, and recipient. There is no other mint path and no hidden
   mint: total issued is a fold over the issuer's issuance objects.
3. For an anchored currency, the gateway additionally checks the signer
   is in the currency's minter set and that cumulative issuance stays
   within the registered supply policy, then submits the ledger
   transfer out of the issuer account (minting *is* a transfer out of
   the designated issuer account — #2040; there is no separate mint
   verb, #2066 §3) with the Issuance object's ID in `user_data_128`.
4. For a graph-native currency, steps 2 alone is the whole act; counting
   views refuse over-cap issuances (§10, case 1).

Supply policy guidance: `capped` is the default recommendation for
anything presented as scarce or valuable, because the cap is also the
damage bound under issuer key compromise (§10, case 7). `open` fits
score-like tokens whose value is purely reputational. Games should
prefer the **prize-pool pattern** (§7): mint the season's supply once to
the game's own pool identity, then award by transfer — supply stays
fixed and auditable, and `award` never needs mint rights.

## 5. Issuer snapshots and anchoring

A snapshot (object 7) is a signed, checkable compression of a currency's
state at a frontier: `{currency, as-of frontier, total issued, total
burned, balance root, ledger position when anchored}`. The balance root
is a hash over holder balances at that frontier (construction is
#2251's spec point 3).

**Anchored currencies.** The currency service computes the snapshot from
TigerBeetle — account counters/history at a ledger position (#2040
§6.3; the control plane's reconciliation and supply audit, #2062 §7.3,
is the scheduled checker) — and publishes it as a graph object signed by
the service ID. The graph fold of issuance/transfer *intents* and the
ledger's executed transfers must agree up to the stamped `user_data_128`
links. A snapshot that disagrees with either side is provably false, and
which side is wrong is mechanically determinable: **the ledger wins for
balances; the graph wins for who authorized what.**

**Graph-native currencies.** Anyone holding the full object set can
recompute the balance root; the issuer's snapshot is a convenience for
light clients, and a lying snapshot is provable by recomputation.

**Central issuer vs issuer snapshots vs a full blockchain** — the three
ways to answer "what is the state of this currency?":

| Model | Who asserts state | What a light client trusts | Cost / limit |
| --- | --- | --- | --- |
| Full blockchain | Global consensus | The consensus rules | The heavy machinery Drew's directive explicitly avoids for a deployment like developing-today's |
| Central issuer (service-signed snapshots) | The currency service ID, over ledger truth | One signed snapshot + spot checks against the ledger join | Centralised — but contained to the currency service (#2066); assertions are auditable after the fact |
| Issuer snapshots (per-ID currencies) | Each issuer, over their own chain fold | The issuer's signed snapshot + recomputation spot checks | No central party; a snapshot is only as good as the issuer's chain availability |

What snapshots buy in both non-blockchain models: a light client
(phone, web, a game server) can trust balances and total supply from one
signed object plus spot checks, instead of folding the whole graph or
holding a TigerBeetle connection. The gateway's balance responses are
already specified as signed snapshots in the centralised-exception
design (#2066 §3, boundary rule B3), so the anchored path and this
object are the same mechanism seen from two sides.

**Not decided here:** snapshot cadence, and who signs snapshots for
each class in the developing-today deployment, are #2043 synthesis
decisions (§11) — #2251's acceptance criteria already reserve them.

## 6. How authorized conversion rates surface here

Boundary first: the conversion *engine* — application-composed,
atomically linked same-ledger transfer pairs through a liquidity
provider's two accounts — is #2063's design. The *RateAuthorisation
object* — schema, signing chain to the liquidity accounts, revocation
set, quote binding — is #2290's. What belongs to *this* investigation is
how rates appear in the graph model and the app surface:

- A **rate offer** (object 5) is the graph-side statement of an
  authorized rate: per ordered pair, per direction. ID 123's offer to
  accept ABC at 8123/10000 says nothing about the reverse direction;
  ABC's issuer publishes their own reverse offer, or doesn't. This is
  Drew's example carried through unchanged.
- An offerer is the issuer *or any liquidity holder*; offers are how a
  holder authorizes conversions against their liquidity without being
  an issuer.
- **Execution cites a specific offer version.** The executor checks the
  offer is live at the offerer's current chain frontier (not
  superseded/revoked, caps and expiry hold), computes both leg amounts
  from the rational rate — rounding in the liquidity provider's favour,
  with any spread as a separate auditable leg (#2040 §2) — submits the
  linked pair with one conversion ID in `user_data_128` on both legs,
  and emits the **conversion receipt** (object 6). Either both legs land
  or neither does (anchored path).
- **Graph-native swaps are honestly non-atomic:** two transfers bound
  by a bilateral swap-agreement object both parties sign first (the
  #2037 attestation pattern). If one leg never appears, the agreement
  plus the completed leg are a signed claim against the defaulter —
  enforceable by registries/reputation, not by the protocol. Any design
  promising atomic offline swaps of independent chains is lying; this
  one doesn't.
- Where #2290's RateAuthorisation and this offer differ in audience:
  the RateAuthorisation is the control-plane record the *gateway*
  enforces at submission; the rate offer is the *graph* statement a
  wallet or app discovers and cites. The synthesis (#2043) should treat
  them as two projections of one authorization, with #2290 owning the
  field schema — flagged in §11 so the two do not drift into rival
  formats.

## 7. The app-facing transaction functions

One API across both bindings; the binding changes the verification
behind the call, never the call shape. (Signatures and error semantics
are specced by #2251 point 4; this is the semantic contract.)

| Function | Semantics | Anchored verification | Graph-native verification |
| --- | --- | --- | --- |
| `define_currency(name, scale, supply_policy, binding)` → currency ID | Signs the definition object (§2.1) | + gateway registration, ledger allocation, system accounts (#2062) | Definition object only |
| `issue(currency, to, amount, memo)` | Issuer only; signs an Issuance object | Gateway checks minter set + cap, submits issuer-account transfer, stamps `user_data_128` | Counting views enforce the cap |
| `award(currency, to, amount, memo{cause ref})` | Transfer from the operator's pool identity; the memo links the payout to its cause (e.g. a #2037 match ID) | Ledger transfer from the pool account | Fold transfer from the pool identity |
| `pay(currency, to, amount, memo)` | Ordinary holder transfer | Signed intent → gateway → ledger | Signed Transfer object |
| `balance(currency, of)` / `supply(currency)` | Read | Ledger read via gateway, or latest signed snapshot | Fold, or latest snapshot + spot check |
| `accept_payment(currency, predicate)` → handler | The transaction-accepting function (below) | Gateway/service notifies on matching posted (or pending) transfer | App folds/watches for matching Transfer objects |
| `convert(offer, amount)` → conversion receipt | §6 execution | Linked pair via gateway | Bilateral swap agreement |
| `snapshot(currency)` → snapshot object | Latest signed snapshot (§5) | Service-signed, from ledger position | Issuer-signed, from chain frontier |

**The transaction-accepting function, precisely.** An app registers
`{currency, min amount, memo pattern}`. On a matching transfer the
app/service verifies, in order: signature valid; sender's chain
unforked; sender funded (ledger balance, or fold up to the app's
risk tier); currency identity is the full (issuer, definition) pair,
not the name. Only then does it fire the app's effect — grant the
service, ship the item, enable the bandwidth tier — and emit a signed
**receipt** object tying the effect to the transfer. Two refinements
from the designs that landed after wave-1:

- **Real-service currencies use the hold pattern.** Where delivery can
  be revoked or refunded (Steam-key grants and the like), acceptance
  runs on a TigerBeetle pending transfer with the grant lifecycle's
  window parameters, fingerprint registry, and clawback/shortfall rules
  (doc `2026-10-30T11-55-00Z_design_grant_lifecycle.md`; #2042 owns that
  lifecycle). `accept_payment` is the hook that handler hangs off; the
  grant spec owns what happens inside the window. General per-ID
  issuance (this document) and purchase grants (that spec) are
  different flows and neither substitutes for the other.
- **Idempotency is part of the shape.** Every mutating call carries a
  client-generated idempotency ID persisted before submission (#2066
  §3); a retry reuses it, an `exists` result is success, and an
  `exists_with_different_*` result is a hard error surfaced to the
  user — never retried into a second operation.

**Risk tiers for graph-native acceptance** (the policy §3 promised):
small amounts may be accepted on signature + unforked chain alone
(cash-risk acceptance); larger amounts require a snapshot anchor at or
after the sender's position, or an anchored currency. The tier
thresholds are app policy, not protocol.

## 8. Worked example A — an ID issues 100 self-tokens to another ID

Alice (ID `A`) creates a token and gives 100 of it to Bob (ID `B`).
Traced graph-to-ledger for the anchored binding; the graph-native
variant differs only where noted.

1. **Definition.** `A` calls `define_currency("alice-credits", scale 2,
   capped{10000}, ledger-anchored)` and signs the definition object
   `D` on her chain. Currency identity: (issuer `A`, definition `D`).
2. **Registration (anchored only).** The gateway verifies `A`'s
   signature, the registry allocates a `ledger` u32 and a `currency_id`,
   records `graph_object_id = D`, and the account factory derives the
   system accounts (issuer, liquidity, fee, burn) and `A`'s and `B`'s
   holder accounts deterministically (#2062 §3).
3. **Issuance.** `A` calls `issue(currency, to = B, 100.00, memo:
   "welcome")` and signs Issuance object `I` on her chain. (Amount is
   10000 minor units at scale 2; the graph object carries the rational
   amount, the ledger carries minor units.)
4. **Gateway checks (anchored only).** Signature verifies (`A` is who
   she claims); registry says `A` is in the minter set for this
   currency and cumulative issuance after this mint is 100.00 ≤ cap
   10000 (signature is *who*, registry is *what*).
5. **Ledger write (anchored only).** The gateway submits a transfer of
   10000 minor units from the currency's issuer account to `B`'s
   account on the allocated ledger, stamping `user_data_128 = I`.
6. **Receipt and visibility.** `B`'s wallet resolves the currency by
   (issuer `A`, definition `D`) — displayed as "alice-credits (Alice)",
   never merged with anyone else's "credits". `balance(currency, B)`
   returns 100.00 from the ledger; `supply(currency)` returns 100.00,
   and anyone can cross-check: fold of `A`'s issuance objects (100.00)
   = issuer-account debits on the ledger (the #2062 §7.3 supply audit
   join, via `I`'s stamp).
7. **Snapshot.** The next service snapshot for the currency records
   frontier ⊇ {`D`, `I`}, issued 100.00, burned 0, balance root over
   {`B`: 100.00}, and the ledger position of step 5's transfer.

Graph-native variant: steps 2, 4, 5 drop out; step 6's balance is a
fold; step 7's snapshot is signed by `A` and checkable by recomputation.

## 9. Worked example B — a game awards coins to a winner and a donor

A game operates under its own identity `G`, with a season currency
defined once, `capped`, anchored.

1. **Season setup (prize-pool pattern).** Before the season, `G` calls
   `issue(season-coin, to = G-pool, 1,000,000)` once — the whole
   season supply, minted to the game's pool account in a single
   auditable Issuance object. No further minting is needed or wanted.
2. **Match ends.** The match result exists as a signed object (the
   #2037 attestation pattern). The game server calls
   `award(season-coin, to = winner W, 250, memo{match: <match ID>})`.
3. **Graph-to-ledger trace.** The award is a Transfer object signed by
   the pool identity citing the match ID → the gateway verifies the
   pool's signature and funding → the ledger transfer moves 250 from
   the pool account to `W`'s account with `user_data_128` = the
   Transfer object's ID. `W`'s client sees the award, its amount, and
   its cause (the match) from one join.
4. **Donor award.** A donor `Dn` who tipped the prize pool earlier is
   awarded a donor badge token by the same path: `award(badge-token,
   to = Dn, 1, memo{donation: <donation ref>})` from the badge pool —
   "coins to winners and donors" is one function with provenance, not
   two features.
5. **Winner spends at an accepting app.** `W` pays 100 season-coins to
   a skin shop whose `accept_payment(season-coin, {min: 100})` handler
   verifies (anchored: ledger balance + the transfer's stamp), fires
   its effect (the skin is granted), and emits its signed receipt.
6. **What an auditor can prove afterwards.** Season supply never
   exceeded the cap (one Issuance object); every award traces to a
   match or donation memo; every ledger movement joins to exactly one
   signed graph object. Nothing in the trace required trusting the
   game server's say-so after the fact.

## 10. Abuse and failure cases

The wave-1 cases, updated with the mechanisms the later designs added.
"Provable ≠ prevented" is stated wherever it is the honest answer.

1. **Issuer secretly inflates past an announced cap.** Impossible to
   hide: issuance objects *are* the supply; over-cap issuance is
   visible to every folder. Anchored: the gateway refuses mints past
   the registered cap, so inflation requires the service's collusion,
   which the `user_data_128` cross-links would expose. Graph-native:
   not preventable, always provable; counting views refuse over-cap
   issuances. ⚠️
2. **Offline double-spend (graph-native).** The sender forks or
   overdraws; both transfers are signed evidence. Recipient policy
   tiers by amount (§7): small = cash-risk acceptance; large =
   require a snapshot anchor or ledger binding before delivering
   value. Detection is total; prevention is policy, named as such. ✅
3. **Name confusion / squatting.** "coins" from a scammer vs "coins"
   from a game: different currency IDs (issuer + definition);
   wallets/UIs display issuer petname + name together and never merge
   balances across issuers. ✅ at the protocol layer; UI discipline is
   a requirement, not a nicety.
4. **Stale or revoked rate offer executed against the offerer.**
   Execution cites a specific offer version and the executor checks
   the offerer's chain frontier at execution time; a
   revoked/superseded offer fails cleanly (the atomic pair never
   starts). The gateway-side revocation set is #2290's mechanism. ✅
5. **Offerer advertises a rate they can't fund.** Conversions fail
   atomically when the liquidity account lacks balance; failed
   attempts cost the offerer reputation (offers and failures are both
   on record) but never create half-converted state. ✅
6. **Lying snapshot.** Anchored: contradicted by the ledger,
   mechanically. Graph-native: contradicted by recomputation. A
   snapshot is an auditable assertion, not a trust root. ✅
7. **Issuer key compromise.** The catastrophic case for any issuer
   system: the attacker mints/converts as the issuer until the
   compromise is noticed. Mitigations available in this model: supply
   caps bound mint damage even with the key; issuer key rotation as a
   signed object (old key names successor; definitions issued
   pre-rotation keep their identity); registries/views pinning which
   issuer key generation they count. The full treatment is filed as a
   follow-up design issue from this investigation (§12) rather than
   left as a synthesis afterthought. ⚠️

## 11. Not decided here — hand-off notes for the #2043 synthesis

#2043 (currencies v1) consumes this document. Inputs status at the time
of writing: #2040 (ledger) done; #2062 (control plane) done; #2066
(boundary) done; #2063 (conversion engine) and #2290 (RateAuthorisation)
open; #2042 (tokens for real services) open, with its grant lifecycle
spec landed 2026-10-30. The synthesis stays gated on its named inputs;
this document does not pre-empt it. Explicitly handed off:

- **Snapshot cadence and signer per deployment class** (§5) — a
  synthesis/operator decision, reserved by #2251's acceptance criteria
  as well.
- **Rate offer vs RateAuthorisation projection** (§6) — confirm the
  two are projections of one authorization, #2290 owning the field
  schema, before any implementation treats either as canonical.
- **Issuer key rotation mechanics** (§10 case 7) — design follow-up
  filed from this investigation (§12); the synthesis adopts or amends
  its outcome.
- **Graph-native fold/index service** — whether graph-native currencies
  get a shared fold/index service or stay purely client-side remains
  open; this document's model works either way, and the choice is a
  cost/UX trade-off for the synthesis.
- **Acceptance risk-tier defaults** (§7) — protocol leaves thresholds
  to apps; the synthesis may publish recommended defaults for
  first-party apps.

## 12. Follow-up routing

Already filed before this document (not duplicated):

- [#2251](https://github.com/developing-today/code/issues/2251) — the
  design spec that makes §2's wire formats, §3's fold rules, §5's
  snapshot format/anchoring join, and §7's API signatures concrete in
  `pkgs/id`. This document is its parent investigation, now landed.
- [#2063](https://github.com/developing-today/code/issues/2063) /
  [#2290](https://github.com/developing-today/code/issues/2290) —
  conversion engine and RateAuthorisation object (§6 boundaries).
- #2062's follow-ups F1–F5 (registry, account factory, gateway
  envelope, reconciliation/supply-audit job, end-to-end PoC) and the
  gated currency PoC #2298 — the anchored path's build list, including
  the §5/§8 supply-audit join (F4).

Filed from this investigation (buildable pieces not covered above):

- **Issuer key rotation and compromise recovery for currency
  issuers** — [#2804](https://github.com/developing-today/code/issues/2804):
  design the signed rotation object, counting-view pinning of key
  generations, and the anchored path's registry/gateway handling
  (§10 case 7).
- **`pkgs/id` currency client API** —
  [#2803](https://github.com/developing-today/code/issues/2803):
  implement the §7 function surface against #2251's specced
  signatures, gated on that spec; a client library only, not the
  currency service.

## 13. Sources

- Issue #2041 and its wave-1 investigation result (queue task 3352,
  2026-10-29) — the model this document formalises.
- TigerBeetle investigation, #2040 —
  `doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/`
  (mint = issuer-account transfer §1, conversions as linked pairs §2,
  snapshot inputs §6.3).
- Currency control plane design, #2062 —
  `doc/2026-10-29T12-42-21Z_design_currency_control_plane/`
  (registry/identity §2, account factory §3, gateway write path §4,
  consistency and supply audit §7).
- Currency as the centralised exception, #2066 —
  `doc/2026-10-29T12-20-00Z_design_currency_centralised_exception/`
  (the exception §1, gateway API shape incl. signed balance snapshots
  §3).
- Grant lifecycle spec (from #2042) —
  `doc/2026-10-30T11-55-00Z_design_grant_lifecycle.md`
  (pending holds, fingerprint registry, clawback/shortfall — the
  real-services acceptance path in §7).
- Program issues: #2056 (epic), #2043 (currencies v1 synthesis), #2063
  (conversion engine), #2290 (RateAuthorisation), #2251 (object-format
  spec), #2030 (counting views), #2037 (attestation pattern).
