# Design: generic invite delegation — encoding, depth, leakage, and group live-set semantics

Issue: [#5751](https://github.com/developing-today/code/issues/5751) (design pass; the model and Drew's 2026-11-04 ruling are stated there and not re-decided here). Cross-links: #2146 (program sessions), #5684 (identity / login / delegation), #835 (scope bits), #5675 (discovery — invites are its private counterpart), #5674 (wire format).

## Decision summary

The invite model of #5751 stands as written. This record settles its four open questions:

1. **Encoding** — two layers, each with a budget. The _paste layer_ is what already ships in `id-directory::invites` (a bare iroh `EndpointTicket`, or the `id:join?group=&ticket=&by=` link) and does not change. The _delegation object_ is the signed invite event; its body is postcard per #5674, budgeted at **≤ 512 bytes**, and gains the terms fields §2–§4 require. The paste layer is budgeted at **≤ 512 characters** so it survives being pasted into any chat.
2. **Delegation depth** — depth is a term of the invite, minted by the authority source and **monotonically decreasing** along the chain. Open-delegation invites default to depth **1** (the redeemer may re-invite; those invites are leaves), world policy may raise the cap to at most **3**. Moderator-grant invites default to depth **0**; a grant may extend depth explicitly, never beyond the grantor's own remaining depth.
3. **Link leakage** — bearer links stay (they are the distribution mechanism), bounded by default: **7-day expiry and a 25-use cap unless the inviter sets otherwise**, per-link redemption auditing, one-operation rotation (revoke + re-mint), and an optional **redeemer pin** in the terms that makes a link non-transferable when set.
4. **Group-fronted worlds** — **live set**. A group member's access follows the group's current fronting set, with #5675's per-world sensitivity filters applying at access time, and the group's signed journal making any past coverage answerable after the fact. A world that needs snapshot semantics does not front through a group; it invites directly.

## Baseline: what already exists

`pkgs/id/crates/id-directory/src/invites/` already implements the paste layer and the verification posture this design builds on:

- `Invitation::Ticket` — a standard iroh `EndpointTicket`, pasted as-is; names no inviter, joins as `RouteStatus::Unverified`.
- `Invitation::Link` — `id:join?group=G&ticket=T&by=K`; `by` (the inviter's account key) is the trust anchor.
- `InviteEvent` / `InviteBody` — the signed delegation object, stored in the inviter's chain and fetched by the joiner (never pasted): today it carries `inviter`, `ticket_digest`, `group`, `role`, `expires_ms`, plus the signature. Verification is offline-capable: a bad signature, or an event that does not match the link or ticket, refuses the join before anything is sent.
- The module's N30 note documents address exposure honestly (who learns whose endpoint addresses from a link); §4 below inherits it unchanged — authorization is access control, not privacy, and this design does not pretend otherwise.

What the baseline does **not** yet carry, and this record specifies: a world target (today the event names a group only), use counts, delegation depth, a redeemer pin, and revocation. Those are the deltas in §5's implementation shape.

## 1. Ticket encoding and the size budget

**Two layers, two budgets.** Conflating them is what makes tickets balloon: everything the delegation _means_ lives in the signed event; everything a human _pastes_ is just a pointer plus a dial address.

- **Paste layer (≤ 512 characters).** The bare ticket, or the `id:join` link (ticket + 128 hex chars of group and inviter key). An `EndpointTicket` carrying a relay URL and no direct addresses is ~150–250 base32 characters; the link form lands ~300–450. To keep the budget with margin, tickets minted for invites SHOULD be **relay-first**: a relay URL plus at most two direct socket addresses. A ticket that would exceed the budget is not minted — the minter drops direct addresses (the relay path still works) rather than emitting a link chat clients will mangle. URL and ID-protocol-URL carriers wrap the identical link form; the delegation object inside does not vary by carrier, per #5751.
- **Object layer (≤ 512 bytes, postcard).** `InviteBody` serialized with postcard, the workspace wire format (#5674; already the encoding of the id crates). Today's body is ~180 bytes; the fields §2–§4 add (target world, uses, depth, redeemer pin, parent digest) add ~120 bytes worst case. The 512-byte budget is a hard parser limit, not a hope: an event body that decodes over budget is refused, so a future field cannot silently grow the object past what chains and relays carry comfortably.

The ticket itself has no expiry (an iroh property the baseline documents); expiry is a _term_, enforced from the event at redemption — which is why an expired invite fails even though its ticket still dials.

## 2. Delegation depth

An invite's authority comes from its source (#5751): open delegation, or a moderator grant. Depth answers "may the redeemer re-invite, and how far does that chain run?"

- Every invite event carries `depth_remaining: u8` and `parent_digest` (the digest of the invite the minter redeemed, or none for a source invite). A re-invite is a **new signed event in the re-inviter's own chain**, naming its parent — so the full chain back to the authority source is auditable from the journals, and redemption journaling (§3) records the chain, not just the last hop.
- **Monotone rule:** a minted invite's `depth_remaining` is strictly less than the minter's own. Nobody — moderator included — can mint depth they do not hold; this is #5751's "an inviter can never grant more than their own authority" made mechanical.
- **Defaults and caps:** a source invite under **open delegation** is minted with `depth_remaining = 1` by default (redeemers can invite; their invites are leaves at 0). World/group policy may set the source depth higher, hard-capped at **3** — beyond three hops the provenance journal stops being something a moderator will actually read, and wide trees are what the use cap and revocation are for. A source invite under a **moderator grant** is minted with `depth_remaining = 0` by default; the grant itself may explicitly extend it (to at most the grantor's remaining depth at grant time).
- A redeemer with `depth_remaining = 0` holds no invite power from that invite. Invite power from other sources (their own moderator grant, open delegation in another world) is unaffected — depth is per-chain, not per-person.
- Invite power remains a scope under #835: grantable, revocable, auditable. Revoking a minter's invite power does not retroactively kill invites they already minted at depth ≥ 0 — those stand on their own signatures until expiry or their own revocation — but it stops new mints. (The alternative, cascading revocation down the chain, was considered and rejected for v1: it makes one revocation silently uninvite a subtree of people who joined in good faith. The journal makes the subtree _visible_; a moderator who wants it gone revokes the individual invites.)

## 3. Link leakage

A bearer link posted publicly is indistinguishable from intended distribution — that is what a bearer link _is_. The design therefore bounds the blast radius and makes use visible, rather than pretending secrecy:

- **Bounded by default.** Link invites are minted with a default **expiry of 7 days** and a default **use cap of 25 redemptions** (both are terms in the event; the inviter may set either, including uncapped uses for a genuinely public link — a deliberate act, not the default). One-time tickets remain the right tool for a single known joiner.
- **Per-link auditing.** Every redemption is journaled with the invite event's digest (the link's identity), the redeemer's IRID, the chain (§2), and the time. The inviter's client lists redemptions per link; a link redeeming from unexpected IRIDs is visible the moment it happens, not in a later log review.
- **Rotation is one operation.** Revoke + re-mint is a single client action that issues a fresh ticket/event pair and revokes the old event. Revocation is checked at redemption against the inviter's chain (the same fetch that retrieves the event), so a revoked link dies at the next redemption attempt. Already-redeemed memberships are separate state, per #5751: rotation never ejects existing members.
- **The stronger option: a redeemer pin.** The terms may carry `redeemer: Option<IRID>`. When set, redemption verifies the redeemer's IRID against the pin (offline, from the signed event) and refuses anyone else — the link becomes non-transferable even if leaked. This is cheap because verification was already offline-capable; it is the recommended form for sensitive groups, with per-recipient links (mint cost ~zero) as the standard UX there. Bearer (unpinned) links remain for public distribution, where transferability is the point.

Nothing stronger is adopted for v1: join secrets and proof-of-possession handshakes were considered and set aside — the Seitan key already exists as the typed-secret mechanism where a group wants one (the baseline notes it never travels in the link format), and layering a second secret into the invite would duplicate it.

## 4. Group-fronted worlds: live set

**Settled: live set.** When a group fronts a set of worlds, a member's access follows the group's _current_ fronting set:

- A world added to the fronting set becomes accessible to existing members when it is added; a world removed stops being accessible through the group. This is the point of fronting — the alternative (snapshot at join) makes groups into per-join fossils and pushes every change back to re-inviting, which is the friction groups exist to remove.
- **Eligibility is not access.** Group membership makes a member _eligible_ for the fronted worlds; #5675's per-world sensitivity filters still apply at access time, and a world whose own policy excludes the member does not open. Live-set propagation never widens a world's own ceiling.
- **Removal symmetry.** Leaving or being removed from the group ends access to worlds the member reached only through that group. Access held through another group or a direct invite is untouched — access is evaluated per path, and any surviving path serves.
- **The journal is the snapshot.** Fronting-set changes are signed group events. "Which worlds did this membership cover at time T?" is answerable from the journal at any later time, so audits get snapshot _answers_ without snapshot _semantics_ freezing access.
- A world that genuinely needs join-time snapshot semantics does not front through a group; it issues its own invites (§1–§3 apply unchanged). That is a deliberate trade, stated here so nobody builds snapshot flags into the group path later by accident.

## 5. Follow-on implementation shape

Against the `id-directory::invites` baseline:

1. Extend `InviteBody` (postcard, versioned): `target` (group, or world + its fronting group), `uses_cap: Option<u32>` + redemption counting, `depth_remaining: u8`, `parent_digest: Option<[u8; 32]>`, `redeemer: Option<[u8; 32]>`. Enforce the §1 budgets at parse (event ≤ 512 bytes) and at mint (paste form ≤ 512 chars, relay-first tickets).
2. Redemption journaling with link identity + chain (the journal record names the event digest, redeemer IRID, parent chain, timestamp), and the inviter-side per-link redemption listing.
3. Revocation: a revocation event in the inviter's chain, checked on the redemption fetch path; rotation as the single revoke + re-mint client operation.
4. Group fronting-set events + access evaluation per §4 (live set, per-world filters, per-path removal), with the journal query that answers coverage-at-time-T.
5. Defaults wiring: link minting defaults (7 days / 25 uses / depth per source) in the client, overridable per mint.

## Sources

- [#5751](https://github.com/developing-today/code/issues/5751) — the model and the four questions (this record's parent issue)
- [#5674](https://github.com/developing-today/code/issues/5674) — wire format: postcard schema, non-UDP carriers
- [#5684](https://github.com/developing-today/code/issues/5684) — identity, login & delegation (chain rules)
- [#835](https://github.com/developing-today/code/issues/835) — scope bits + session-frame wiring (invite power as a scope)
- [#5675](https://github.com/developing-today/code/issues/5675) — discovery; per-world sensitivity filters
- Current implementation: [`invites/`](../../pkgs/id/crates/id-directory/src/invites/) in `id-directory` (paste layer, signed invite event, N30 exposure note)
