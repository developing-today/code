# Provider discovery & negotiation protocols — decision record

Design record for issue #5899 ("id: design: provider discovery &
negotiation protocols (from #5686)"). Written per
`doc/DOCUMENTATION_PROTOCOL.md` (dated record in `doc/<UTC>_design_<name>/`).
Origin issue: #5899. Parent: #5686 (one capability-provider model for
ESP32/MQTT, terminal, Bevy, smartphone, and web). Evidence base: #5931
(capability-provider field survey — option-space inventory, no verdicts).
Sibling designs this record must not re-decide: #5898 (declaration
artefacts), #5900 (grants), #5904 (offline / degraded behaviour), #5905
(security model). Status: decision record — chosen shape, rejected
alternatives, and implementation invariants are settled here; no
implementation is authorised by this record alone.

---

## 1. Problem

#5686 fixes the provider model — typed, described, versioned capabilities
offered into the ceiling model (#789), used only under least-privilege
grants — and leaves three questions to this record:

1. **Discovery visibility / minimisation.** How do consumers find
   providers and their capabilities (within a world, within a server,
   across bridges / federation), and what may a party with no grant
   learn? #5686's own test: discovery must not leak a device inventory —
   what does an unauthorised browser learn: existence? type? owner?
2. **Negotiation shape.** Before use, provider and consumer agree
   capability version, encoding, rate, quality, and session lifetime.
   Negotiation failure is a normal outcome; partial agreement must be
   representable. Is negotiation pre-grant metadata exchange, or does it
   itself require a grant?
3. **Stale-provider handling.** Providers vanish (device off, tab
   closed, game quit). What do TTLs, heartbeats, and tombstones mean,
   and what may a consumer's cached view assume?

Verified at origin/main `2b3ca04ac8` (2026-11-08): the code has a
*static* provider catalog only — `ProviderManifest`
(`pkgs/id/crates/id-core/src/world_caps.rs:200-217`: name, contract
version, kind, argument schema, bounds, required scope, default-ceiling
flag) and a `ProviderRegistry` over statically registered manifests
(same file, `:504-552`: lookup / catalog views). There is no runtime
registration, no discovery surface, no negotiation, and no staleness
semantics. This record designs those three, against the static catalog
as the degenerate case (a built-in provider is a declaration with an
infinite TTL and world scope — §5).

## 2. Decisions in one paragraph

Discovery is a **filtered view over signed declaration records**
(#5898's artefacts), served per scope (world, server, federation) —
never a separate registry protocol and never a broadcast. Visibility is
**tiered and minimised** (§3): parties outside the scope learn nothing;
scope members without a grant see a minimised card (capability class,
provider-kind class, liveness — behind an unlinkable rotating handle,
never an identity or an inventory); only a negotiation counterpart or
grant holder sees a full manifest, and only for the capabilities in
play. Negotiation is **pre-grant, authority-free metadata exchange**
(§4) that can only *narrow* the declared terms, is bounded in rounds,
and concludes in a **terms record**; a grant (#5900) may be issued only
against a concluded terms record and never wider than it. Staleness is
a **lease-like lifecycle on the declaration** (§5): heartbeat-renewed
TTL by provider class, `live → suspect → stale → tombstoned`, where
stale removes a provider from discovery and voids prior terms records
but does **not** revoke grants — it makes them unusable until the
provider re-declares under a new declaration epoch.

## 3. Discovery: scopes, tiers, and the minimised card

### 3.1 One substrate: the declaration view

A provider becomes discoverable only by filing a signed declaration
(#5898 owns the artefact format) into a scope. Discovery in that scope
is a query against the declarations filed there, filtered by §3.2.
There is no second discovery database and no announcement channel:
anything not in a signed declaration is not discoverable, and anything
discoverable is traceable to exactly one declaration record. This
record requires the declaration artefact to carry, beyond #5898's own
fields: `scope` (world id / server id / federation intent), `ttl_class`
(§5), and an `epoch` (monotonic per provider identity, §5.3). If #5898
settles the artefact without those fields, they ride in the discovery
envelope the provider files alongside the artefact — the coupling is
explicit here rather than silently re-decided there.

Scopes compose by containment, not by copying: a world-scope
declaration is visible to the server scope that hosts the world **only
in minimised-card form and only if the declaration's scope permits
promotion**; federation (bridge) propagation likewise carries only
minimised cards, only for declarations marked federation-visible. The
full manifest never leaves its home scope through discovery.

### 3.2 Visibility tiers

| Tier | Who | What a discovery query returns |
| --- | --- | --- |
| A — outsider | No membership in the scope (e.g. an unauthorised browser) | **Nothing.** Queries return the same empty result as for a scope with no providers; existence is not leaked. |
| B — member, no grant | A member of the world / server scope with no grant from this provider | **Minimised card only:** capability name and class, provider-kind class (constrained-device / host / web / engine / phone), liveness state (§5), and a rotating discovery handle (§3.3). Never: provider identity, owner, device inventory detail, exact rate/size limits (class bands only), declaration history. |
| C — counterpart | A party with a concluded terms record or a live grant for a capability | The **full manifest for those capabilities only** (exact versions, schemas, bounds), plus the provider's stable identity as bound in the terms/grant record. |

Rules that make the tiers real rather than advisory:

- **Queries are answered from the filtered view, never filtered after
  the fact.** A Tier-B consumer issuing a query for a capability class
  learns which minimised cards match; it cannot enumerate by provider,
  by owner, or by handle history, and result counts are per-card, not
  per-device (one device offering five capabilities is five cards —
  the inventory is the set of joins a consumer *cannot* make, because
  cards carry no shared identifier).
- **Rate and shape limits on discovery itself.** Per-consumer query
  rate limits and a maximum page size apply in every tier, so the
  minimised view cannot be strip-mined into an inventory by volume.
  Discovery queries and their result sizes are auditable in scope.
- **No existence oracle.** Tier-A answers, and Tier-B answers for
  capabilities outside the member's world, are indistinguishable from
  "no such provider": same shape, same empty result, no error
  differential.

### 3.3 Discovery handles

A minimised card names its provider by a **discovery handle**: a
pseudonym derived from (provider identity, scope, rotation window),
rotated on a window of the same order as the declaration TTL (§5) and
never reused across scopes. A handle is sufficient to open a
negotiation (§4) — the discovery layer resolves handle → provider
internally — and worthless for anything else: it is not an identity, it
grants nothing, and handles from two scopes or two windows cannot be
joined to track a device. This is the concrete form of #5686's
minimisation requirement: the honest answer to "what does an
unauthorised browser learn" is *nothing* (Tier A), and to "what does a
fellow world member learn" is *that a capability class is available
from something of a kind class, currently live* — not whose, not what
else it offers, not where it is.

## 4. Negotiation: pre-grant, narrowing-only, terms-recorded

### 4.1 Position: negotiation is metadata, not authority

Negotiation requires **Tier-B visibility and nothing more**: it is
pre-grant by design, because its whole purpose is to find out whether
a grant is worth requesting. This follows the principle #5931 records
from the writer-set design (#5545): *ingest is a validity gate, never
an authority gate* — symmetrically here, negotiation is a *terms* gate,
never an authority gate. A concluded negotiation confers no right to
invoke anything; it produces a terms record (§4.3) that a grant
request (#5900) can cite. Negotiation traffic is authenticated as to
*scope membership* (you may only negotiate for cards you can see),
rate-limited, and logged in scope — negotiation is discoverable
metadata exchange, and treating it as free and unlogged would reopen
the inventory leak through the side door (probe-by-negotiation).

### 4.2 Shape: offer / counter-offer with monotonic narrowing

One negotiation is a bounded exchange over a single capability (a
multi-capability session negotiates per capability and bundles the
terms records):

1. **Request (consumer → provider):** capability name, acceptable
   version range with a hard minimum, acceptable encodings, desired
   rate and quality, desired session lifetime.
2. **Offer (provider → consumer):** the provider's terms within both
   its signed manifest and its own ceiling — a point selection, not a
   range: one version, one encoding, one rate, one quality, one
   lifetime — or a typed refusal (§4.4).
3. **Counter-offer (either side, at most 3 rounds total):** may only
   *narrow* — lower rate, coarser quality, shorter lifetime, a version
   inside the intersection already established, an encoding both sides
   already listed. A counter-offer that widens any dimension beyond
   the counterparty's last stated bound, the consumer's original
   request, or the provider's manifest is protocol-invalid and ends
   the negotiation as a failure (`widening-counter-offer`), not as a
   new offer.

Version selection is intersection-and-highest: the selected version is
the highest version in the intersection of the consumer's range and
the manifest's declared versions; an empty intersection is a typed
failure (`version-gap`), never a silent downgrade below the consumer's
stated minimum.

**Partial agreement is a first-class outcome.** The result of a
bundled negotiation is the set of capabilities that concluded;
capabilities that failed are listed with their failure reasons. A
consumer may proceed to a grant request on the concluded subset
without renegotiating the whole bundle.

### 4.3 The terms record

A concluded negotiation writes a **terms record**: {consumer identity,
provider identity, capability, selected version, encoding, rate,
quality, session lifetime, declaration epoch (§5.3), expiry}. Terms
records are journaled in scope (auditable, replayable) and expire —
by default with the shorter of the agreed session lifetime or the
declaration's remaining TTL. Two bindings make the record load-bearing
rather than decorative:

- A grant (#5900) may be issued **only against a live terms record**,
  and the grant's scope must be a subset of the terms (capability,
  version, rate, lifetime) — grants are where authority enters, and
  this record refuses to re-decide #5900 beyond that interface.
- A terms record is **bound to a declaration epoch**: if the provider
  re-declares (epoch advances, §5.3), outstanding terms records under
  the old epoch are void and must be renegotiated. Nobody invokes
  against terms negotiated with a manifest that no longer exists.

### 4.4 Failure reasons (typed, closed set)

`version-gap`, `rate-floor` (provider cannot meet the consumer's
minimum rate — or consumer cannot accept the provider's), 
`quality-floor`, `encoding-gap`, `policy` (ceiling / scope rules
forbid the pairing), `widening-counter-offer`, `rounds-exhausted`,
`provider-vanished` (§5 transition mid-negotiation). A failure ends
that capability's negotiation; it is recorded with its reason, so
"negotiation keeps failing" is diagnosable from the journal rather
than folklore.

### 4.5 Transport notes (shape is one, carriers differ)

The exchange above is transport-agnostic record exchange, per #5686's
one-model test. Over an MQTT bridge (ESP32 class), the records ride
bridge topics and the round bound plus small record size keep it
within constrained bandwidth — the bridge translates carriage, never
semantics, and holds no negotiation state of its own. In-process
(engine plugin), the same records are direct calls. If a carrier
cannot represent a record field, that is a carrier limitation to
surface as `policy` failure, not a dialect of the protocol.

## 5. Stale providers: TTL lifecycle and cache rules

### 5.1 Lifecycle states

Every declaration carries a TTL class and is kept live by heartbeats
(provider → scope discovery view). Defaults, tunable per class without
protocol change:

| Provider class | Heartbeat | TTL (stale after) | Rationale |
| --- | --- | --- | --- |
| Session-bound (web tab, engine session, phone foreground) | 15 s | 45 s | The provider dies with the session; discovery must notice within a minute. |
| Host (terminal, local services) | 30 s | 120 s | Processes are longer-lived; two missed minutes is a real signal, not a hiccup. |
| Constrained device via bridge (ESP32/MQTT) | bridge-attested, 60 s | 300 s | The *bridge* heartbeats liveness for sleeping devices; the TTL bounds bridge attestation, not device wakefulness — a device may sleep inside its TTL, what expires is the bridge's claim that it will answer. |

States:

- **live** — within TTL. Discoverable per tier; negotiable.
- **suspect** — past a soft deadline at half the TTL, or one missed
  heartbeat: still discoverable, but cards are flagged `suspect`, and
  **no new negotiations open** against a suspect provider (in-flight
  negotiations may conclude). Suspect is the flap damper: a provider
  oscillating at its TTL edge does not churn negotiation state.
- **stale** — past TTL: removed from discovery results in all tiers.
  Consumers holding the provider in an active session receive a
  `provider-vanished` notice (§5.2); terms records under its
  declaration epoch are void (§4.3).
- **tombstoned** — on explicit withdrawal, or 24 h after stale: the
  declaration is replaced by a tombstone record (provider identity,
  epoch, withdrawal/stale time, reason class). Tombstones are retained
  for audit and are never discoverable. A tombstone is what makes
  "was this provider ever here, and what happened to it" answerable
  without resurrecting the card.

### 5.2 What consumers and sessions may assume

- **Cached views expire with the record.** A consumer may cache a
  minimised card or manifest view only until the earlier of the
  record's TTL or its own cache policy, and must present expired
  cache entries as *unknown*, never as live. There is no
  serve-stale in discovery: a capability that might be gone is
  negotiated against only after a fresh view confirms it.
- **In-flight invocations** at the stale transition complete or fail
  on their own terms — staleness does not abort running work — but no
  new invocation starts under a grant whose provider is stale
  (§5.3). Sessions drain; they do not migrate, because there is
  nothing in this design that could make a second provider
  interchangeable with the first (that would be a declaration-level
  claim, #5898, and it is not made by default).
- **Stale is not revoked.** Staleness is a statement about
  *reachability*, not *authority*: grants (#5900) survive a stale
  period unchanged, and revocation remains #5900's separate,
  deliberate act. Conflating the two would make every tab close a
  revocation event and every return a re-approval — the UX failure
  #5686's grant section warns against by separating sensitivity
  classes.

### 5.3 Return and declaration epochs

A returning provider re-declares: same identity, **epoch = previous
epoch + 1**, fresh TTL. The new declaration supersedes the tombstone
(if any) in discovery, and the epoch advance is precisely what voids
old terms records (§4.3) and re-arms negotiation. Grants cite the
terms record they were issued against; a grant issued under epoch *n*
is invocable under epoch *n+1* only if its own terms (capability,
version, limits) are unchanged in the new declaration — checked
mechanically against the two manifests at first invocation after
return, and recorded. Anything else requires a fresh negotiation.
Static built-in providers (today's `ProviderRegistry` catalog) are
the degenerate case: compile-time declarations, epoch 0, no TTL —
they cannot go stale because they cannot leave.

## 6. Rejected alternatives

| Alternative | Why rejected |
| --- | --- |
| **Open broadcast discovery** (announce-to-all, mDNS-style) | Leaks the device inventory #5686 explicitly forbids; no scope or tier concept exists to minimise against. |
| **A central global registry service** for all scopes | Creates a cross-scope correlation point (the inventory, again, one query away), a single point of failure for a world-local operation, and a second database to keep consistent with the declarations. The journal-resident declaration view (§3.1) needs none of it. |
| **MCP `tools/list` as the discovery protocol** | Per #5931 §6.1, MCP is a declaration + invocation surface for a session; the spec is explicitly silent on policy and visibility, and its authorisation story is token-centric. MCP is a legitimate *mouth* — a provider may be projected as an MCP server, and the projection of a minimised card into `tools/list` is a fine Tier-B rendering — but the discovery *layer* (scopes, tiers, staleness) is ID's, not MCP's. |
| **CapTP-style live-reference discovery** (#5931 §6.3) | Authority-as-possession works for live, connection-scoped sessions; discovery records here must survive offline, sync, and audit in a journal. CapTP discipline may inform live session handling; it cannot be the discovery ledger. |
| **Negotiation folded into grant issuance** (#5900) | Conflates agreeing terms with conferring authority; makes partial agreement and typed failure awkward exactly where #5686 demands them, and would require a grant (an authority act, possibly human-approved) before two parties can even learn whether their versions intersect. |
| **Serve-stale discovery caches** | A consumer acting on a provider that vanished an hour ago fails later, worse, and with the failure misattributed to the invocation layer. Expired = unknown (§5.2). |
| **Staleness as revocation** | §5.2: reachability ≠ authority; merging them makes routine vanish/return cycles into re-approval churn and destroys the audit distinction between "left" and "was removed". |

## 7. Invariants implementation must hold

1. **Discovery grants nothing.** A discovery record, a handle, and a
   concluded negotiation are claims and metadata; authority enters
   only through a grant (#5900). Every layer that touches a discovery
   artefact treats it as a validity gate, never an authority gate.
2. **Minimisation is monotone and structural.** A party can never
   learn more than its tier allows — including by enumeration, by
   handle correlation across scopes/windows, by negotiation probing,
   or by error differentials (§3.2). If a proposed feature needs a
   join across cards, the feature is wrong, not the tier.
3. **No cross-scope leakage.** Full manifests never leave their home
   scope through discovery; promoted/federated forms are minimised
   cards, and only where the declaration permits promotion.
4. **Negotiation only narrows.** Every concluded term is within the
   provider's signed manifest, the provider's ceiling, and the
   consumer's original request (§4.2). Widening is a protocol failure,
   not an offer.
5. **Terms are epoch-bound and expiring.** No grant against an expired
   or epoch-voided terms record; no invocation under terms negotiated
   against a superseded manifest (§4.3, §5.3).
6. **Stale means undiscoverable, suspect means unnegotiable, expired
   cache means unknown.** These three are checked at the discovery
   layer, not left to consumer politeness (§5).
7. **Staleness never revokes, revocation never fakes staleness.** The
   two state machines (this record's lifecycle, #5900's grants) touch
   only at the documented invocability check (§5.3).
8. **Every transition is journaled in scope.** Declaration filed,
   suspect/stale/tombstone transitions, negotiation conclusions and
   failures with typed reasons — the audit trail is the journal, at
   the visibility of the scope it happened in.

## 8. Explicit couplings (not re-decided here)

- **#5898 (declaration):** this record consumes the signed declaration
  artefact and requires `scope`, `ttl_class`, and `epoch` to be
  available to the discovery layer (§3.1). Artefact format, signing,
  and withdrawal mechanics are #5898's.
- **#5900 (grants):** consumes the terms record as its only
  negotiation input (§4.3). Grant issuance flows, approval UX by
  sensitivity class, and revocation are #5900's.
- **#5904 (offline / degraded):** what a provider or consumer queues
  *while disconnected* — including whether discovery views sync for
  later use — is #5904's. This record settles only what freshness a
  view must have when it is *used* (§5.2).
- **#5905 (security model):** attestation strength for declarations
  per provider class, and the threat model for handle derivation, are
  #5905's. This record assumes signed declarations; it does not set
  how strongly a class must prove it can do what it declares (#5686's
  declaration-honesty question, which stays with #5898/#5905).

## 9. Implementation follow-ons (named next actions)

Filed as design-now / implement-when-demanded, in dependency order;
each is a separate future packet, none is authorised by this record:

1. **Discovery view over declarations** — the filtered-view query
   layer (§3), tiers, handles, and discovery audit. Depends on #5898.
2. **Negotiation + terms records** — the §4 exchange, typed failures,
   and the journal record. Depends on follow-on 1 and #5898.
3. **Staleness lifecycle** — heartbeat/TTL state machine, tombstones,
   consumer cache rules, and the §5.3 return check. Depends on
   follow-on 1; the grant-invocability interface is agreed with #5900.
