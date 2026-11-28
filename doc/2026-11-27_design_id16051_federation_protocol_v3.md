# Discovery, registry & federation — v3 design: the federation protocol (#16051)

Design record for issue #16051 ("id(phase-14): Discovery, registry &
federation — v3 — design: federation protocol"). Core topic: #6277
(Discovery, registry & federation). Predecessors: the v1 design #6287
(record `doc/2026-11-16_design_id6287_discovery_registry_federation_v1.md`,
merged in PR #10703), the v2 outline #6288, the v2 design #10712
(record `doc/2026-11-27_design_id10712_public_registry_endpoint_v2.md`,
merged in PR #15943), and the v3 outline #6289 (whose refined scope in
#16051 this record decides point by point). Sibling steps: #6283
(hosted worlds v1) and #1068 (portals by mutual grant, CLOSED) — this
record composes with both and re-decides neither.

**Entry condition — verified 2026-11-27 (lane any-n).** The condition
had two halves, both met: (1) the v2 public-registry design is written
and merged (PR #15943); its §12 resolves the #6288 questions and §13
separates implementation follow-ups; and (2) the v1
signed-list/mirror/client/resolver foundations it stands on have
landed — #10707 (registry list format + reference publisher + mirror
import, `pkgs/id/crates/id-sync/src/registry.rs`), #10709 (client
discovery settings, `discovery_client.rs` via PRs #14671 and #15694),
with the supporting resolver #10708 closed (`discovery_resolver.rs`)
and the join path #15694 (`discovery_join.rs`). #1067
(hosted-operation scheduling) is OPEN and ops-only, not a protocol
prerequisite.

Status: v3 design. The #6289/#16051 scope's open questions are decided
in §8 with defaults; nothing here authorizes implementation beyond the
breakdown in §9.

## 1. Governing principles (#1240) — how each one holds for federation

- **Local-first always.** Federation adds reach, never a dependency:
  the zero-infrastructure path is still the invite link (v1 §6). A
  world with no announcement, no registry subscription, and no relay
  is fully usable by anyone holding its link. Every federation
  mechanism in this record — mirror import, cross-server join, relay
  path — is additive discovery/transport on top of that floor, and a
  client that uses none of them loses nothing but convenience.
- **Hostable by anyone at any scale.** A federating operator needs no
  permission, no key of ours, and no central index: a registry is a
  signed file (`registry.rs`), a relay is a stock iroh relay named by
  URL string, a pkarr sink list is a URL list
  (`announcement_relay.rs` `RelaySinks`). Federation state is files
  and URL lists an operator can copy, not accounts on our service.
- **P2P behavior identical to hosted behavior.** Same lists, same
  `arbitrate` fresher-seq-wins rule (`discovery_resolver.rs`), same
  `client_resolver` subscription filter and `route_for` relay gate
  (`discovery_join.rs`). The hosted instance differs only in which
  registries it subscribes to by default (our curated list) and that
  it operates a default relay — both are settings, not protocol
  branches.
- **Our hosted instance is the public backstop, never a
  requirement.** Our curated registry and our relay are defaults a
  fresh self-host does not inherit (`ClientDiscoveryConfig::fresh_self_host`
  follows none). Federation must work with the backstop offline: two
  self-hosts with each other's registry keys and one shared relay (or
  none, where direct connectivity exists) federate fully.

## 2. Disposition of the #6289 outline (accept / decide)

All four outline bullets are accepted as the shape of v3; the items
the outline deferred are decided in §8:

- **Registry federation and mirror propagation** — accepted: the
  mirror is the federation primitive (v1 §5, `Registry::import`),
  and this record fixes exactly what crosses a mirror boundary (§3).
- **Cross-server join and portal composition** — accepted with the
  boundary kept: the join handshake resolves an endpoint and proves
  authorization at both ends; portal grants stay the only
  authorization (§4). Registry listing never grants, never bypasses.
- **Relay/proxy threat model and protocol** — accepted: the
  relay-required client behavior (`route_for`, `RELAY_REQUIRED_DISCLOSURE`)
  is reused unchanged; this record specifies federation/inter-server
  relay semantics, not another client setting (§5).
- **Synthesis** — decided in §7: the smallest interoperable protocol
  is three rules (mirror, join, relay) with explicit parity/backstop
  boundaries, and the implementation breakdown in §9.

## 3. Mirror propagation: what crosses the boundary

A mirror fetch is a fetch of a signed list followed by a local
`import` under the mirror's own policy — never a live query, never a
transitive crawl. The boundary rule, stated once:

> **A mirror copies announcements and tombstones; it never copies
> trust.** The fetched list's announcements remain valid across the
> boundary because they are self-signed by world keys
> (`SignedRegistryList::verify` re-checks each one against its own
> `world_id`). The fetched list's signature, policy, `seq`, and
> `published_at` do not cross: the mirror applies its own policy,
> assigns its own `included_at`/`registry_seq`, signs its own list,
> and is attributed by its own key.

Concretely, per fetched list from registry key K:

1. **Verify first.** `SignedRegistryList::verify_from` against K,
   then each entry announcement against its own `world_id`, expiry
   at `now`, and `registry_listable` (listed-only; `check_shape`
   refuses unlisted at inclusion and `arbitrate` re-checks at read
   time). A list that fails verification is dropped whole — there is
   no partial import of an unverifiable list.
2. **Entries cross verbatim or not at all.** An entry that verifies
   is copied byte-for-byte (announcement + world signature); the
   mirror never edits, rewords, or re-signs an announcement. The
   mirror's `include` records its own `included_at` and its own
   `registry_seq`. An entry the mirror's policy rejects is simply
   absent from the mirror — silence, not a tombstone (tombstones
   speak only for worlds the mirror itself once listed).
3. **Tombstones cross as notices, never as vetoes.** A fetched
   tombstone `{ world_id, removed_at, reason, registry_seq }` is its
   author's signed opinion. The mirror may (a) ignore it, (b) display
   it alongside a continued listing (the v2 §6 endpoint behavior), or
   (c) adopt a delisting under its own key with its own reason and
   its own `registry_seq` — but (c) is a new tombstone by the mirror,
   attributed to the mirror, not a forwarding of K's. A tombstone
   from K never voids a live announcement another subscribed list
   (or pkarr, or the invite link) serves: `arbitrate` treats it as
   `Outcome::Delisted` for K's layer only.
4. **Sequence does not cross.** `registry_seq` and list `seq` are
   per-registry monotone counters. The mirror's counters advance only
   on the mirror's own include/delist/publish events. A reader
   comparing two registries compares each registry's `seq` against
   that same registry's previous fetch — never K's `seq` against the
   mirror's. Freshness across hops is per-hop `published_at`
   (§3.1), not a transitive chain.
5. **Policy never crosses.** The fetched `policy` string (≤
   `MAX_POLICY_CHARS`) is display context for the operator deciding
   whether to keep subscribing — it is not imported, not merged, and
   not served as the mirror's policy. Origin is declared by
   convention (v2 §8): the mirror's policy names the registries it
   mirrors ("mirrors K under policy P, kept/dropped …"), and every
   served view attributes each row by signing key (v2 §6).

### 3.1 Subscription, freshness, loops, fan-out

- **Operator-directed subscription is the default and the norm.**
  A mirror subscribes to registries its operator names, in the order
  the operator added them (`ClientDiscoveryConfig.subscribed_registries`
  order is the query order). There is no default subscription for a
  fresh self-host, and the hosted instance's curated default is a
  setting, not a protocol requirement.
- **Bounded automatic refresh is allowed; automatic discovery is
  not.** Re-fetching subscribed lists on a timer (heartbeat
  republish cadence, v1 §4: half the record's effective lifetime) is
  mechanical and needs no justification. Adding a subscription the
  operator did not name — following a registry because a subscribed
  registry names it — is forbidden by default: it turns one
  operator's trust choice into transitive trust. Any future
  bounded-discovery proposal must justify itself in its own design
  step with explicit hop and fan-out bounds; this record does not
  grant it.
- **Loop bound: one fetch, one import, no chains.** A mirror imports
  from lists it fetched. It never imports from a list-of-lists, never
  follows a "mirrors" pointer inside a fetched policy, and never
  serves a fetch of its own list as an instruction to fetch further.
  A → B → A converges because each import is a local policy decision
  over verified announcements: A re-importing B's list that contains
  A's own announcements re-verifies them (they are still
  self-signed) and re-applies A's policy. No loop detection protocol
  is needed because there is no transitive traversal to loop.
- **Fan-out bound: the reader queries what it subscribes to.**
  A resolver asks the invite link, then pkarr, then its subscribed
  registries (`client_resolver` drops every fetched list whose key is
  not subscribed). A world listed once and mirrored widely is "found
  everywhere" because many operators chose to list it — not because
  one query fans out. There is no federated query, no crawl, no
  aggregation feed (v2 §3 holds unchanged).

## 4. Cross-server join and portal composition

The minimum join handshake across independently operated servers has
three steps, in order, and each step fails closed:

1. **Resolve.** The joiner's client resolves the world's freshest
   valid announcement through `plan_join`: invite link, then pkarr,
   then subscribed registries, fresher-`seq`-wins (`arbitrate`). The
   announcement yields the host's node id and relay hint
   (`Subject::Endpoint`). A registry listing is discovery only: it
   answers "where is this world" and never "may I enter".
2. **Prove authorization at both ends.** The joiner's server checks
   the joiner is entitled to attempt the join (its own access
   controls); the host's server checks the joiner is entitled to
   enter (its own access controls, including the world's visibility
   and any portal grant). Either check may refuse, and a refusal
   states which end refused without leaking the other end's policy.
3. **Open the portal channel by mutual grant.** Portal messaging by
   mutual grant exists (#1068, CLOSED): the channel opens only when
   both servers hold a grant for it. The join handshake composes with
   that channel — it does not replace it, re-decide it, or carry a
   second authorization inside the discovery objects.

Invariants (all already enforced in code; restated here as protocol):

- **Listing never grants.** `registry.rs` carries no access or grant
  fields by construction; `discovery_join.rs` takes no grant input
  and produces no grant output. A world an operator can see in a
  registry but holds no grant for is undiscoverable-but-unjoinable:
  resolution succeeds, authorization refuses.
- **Portals never leak through discovery.** Announcements carry
  endpoint + relay hint + coarse occupancy + tags — never member
  lists, never grant state, never portal contents. A portal grant
  check happens over the server-to-server channel, not in any object
  a registry mirrors.
- **Visibility scopes hold across servers.** Private (no
  announcement) resolves only via the invite link at both ends;
  unlisted resolves via pkarr-by-key but never via any registry
  (refused at inclusion and at read time); listed resolves via all
  three layers. A server that lists an unlisted world anyway is
  refused by every conforming reader — non-conformance is visible as
  a verification failure, not a silent widening.

## 5. Relay threat model and protocol (federation path)

This section reuses the landed relay-required client behavior
(`route_for`, `RELAY_REQUIRED_DISCLOSURE`, `JoinRoute::RelayOnly` vs
`DirectAllowed`) and specifies what changes when the two ends are
independently operated servers rather than one client and one host.

### 5.1 What the relay learns

A relay on the join/portal path learns, at most: that two node ids
exchanged packets through it, when, and how many bytes. It learns
nothing else by construction of the objects it carries:

- Announcements mirrored through registries carry relay-hint URL
  strings, never direct addresses or member IPs (v1 §7).
- Portal payloads are authorized at both ends (§4); the relay is not
  a party to the grant check and holds no grant state.
- The relay never sees world membership lists, registry policies, or
  tombstone reasons beyond what is already public in the mirrored
  lists it could fetch as any reader.

An operator choosing a relay trusts it with timing/size metadata and
nothing else. That trust is per-connection and per-world: using relay
R for world W says nothing about R's suitability for world V.

### 5.2 Who may operate a relay

Anyone. A relay is a stock iroh relay named by URL; operating one
requires no permission, no registry key, and no grant from either
server. The three legitimacy classes, in order of assumption:

1. **The host's relay** (named in the world's announcement).
   Preferred when present: the host chose it, and the joiner's
   relay-required path needs exactly one relay both ends can reach.
2. **The joiner's relay** (the joiner's operator configuration).
   Used when the announcement names none the joiner can reach, by
   explicit operator choice at the joiner's end — never silently
   substituted mid-join.
3. **A third-party relay** (either end's fallback list, or our
   hosted default as backstop). Used only by explicit configuration;
   the handshake names which relay was used so both ends' logs agree.

No relay is privileged by the protocol. Our hosted relay is the
public backstop (reachable default), never a requirement: two
self-hosts with direct connectivity join with no relay at all
(`DirectAllowed` with `relay: None`), and two self-hosts behind NATs
with one shared relay URL join through it with no account anywhere.

### 5.3 When no suitable relay exists

The handshake fails closed with a named reason, and the failure is
attributed to the missing path — never papered over by falling back
to direct:

- Relay-required joiner + world publishes no reachable relay →
  `route_for` bails ("relay-required is on and this world publishes
  no relay…"). The client surfaces the disclosure string, not a
  retry-over-direct.
- Both ends relay-required but share no relay → each end reports the
  relay sets it attempted (host's hint, joiner's list) so the two
  operators can agree on one out of band. No automatic relay
  discovery, no central relay directory: a directory of relays would
  be a central index (v2 §5 exclusion holds).
- Relay reachable but untrusted by one end's policy → that end
  refuses; the refusal names the policy (e.g. "operator allows only
  relays R1, R2"), not the relay's contents.

## 6. Local/self-host parity and hosted-backstop boundaries

| Concern           | Self-host / local                                     | Our hosted instance                                       | Boundary                                                                            |
| ----------------- | ----------------------------------------------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Registry defaults | Follows none until the operator subscribes            | Ships subscribed to our curated list                      | A setting (`fresh_self_host` vs `hosted_with_curated`); same `client_resolver` code |
| Relay defaults    | No relay until configured; direct where possible      | Operates a default relay; announces its hint              | A setting; same `route_for` gate                                                    |
| Mirror tooling    | Same `Registry::import` + `publish` + `verify_from`   | Same, plus scheduled re-fetch of the curated subscription | Operation cadence only; no protocol branch                                          |
| Public endpoint   | Anyone may serve one over their subscriptions (v2 §9) | Serves one over the curated subscription                  | Same query code; attribution always names the curator                               |
| Portal grants     | Mutual-grant check, both ends (#1068)                 | Same                                                      | No hosted override; a listing never overrides a grant                               |
| Offline backstop  | Two self-hosts federate with the backstop offline     | Backstop is reachable, not required                       | Federation must be demonstrated with the backstop off                               |

## 7. Synthesis: the smallest interoperable protocol

Three rules, each testable against the landed modules:

1. **Mirror rule (§3).** Verify the fetched list against its key,
   re-verify each announcement against its world key, copy entries
   verbatim under local policy, treat tombstones as attributed
   notices, assign local seq, sign locally. Interop test: A publishes,
   B mirrors, C fetches from B — C's copy verifies against B's key
   and each announcement verifies against its world key; B's `seq`
   never appears in C's counters.
2. **Join rule (§4).** Resolve (`plan_join` order), authorize at
   both ends, open the portal channel by mutual grant. Interop test:
   a world listed in a mirrored registry but with no grant for the
   joiner resolves and then refuses at authorization — with the
   refusal attributed to the refusing end.
3. **Relay rule (§5).** Never open direct when relay-required;
   name the relay used in both ends' records; fail closed with the
   disclosure string when no shared relay exists. Interop test: a
   relay-required joiner against a relay-less announcement fails with
   the `route_for` error, not a direct attempt; packet captures show
   relayed bytes only.

Anything not in these three rules is not in the protocol: no
transitive subscription, no federated query, no ranking, no popularity,
no relay directory, no grant-in-discovery, no policy merge.

## 8. Open choices — decided here with defaults

| Choice (deferred by #6289/#16051) | v3 decision                                                                                                                                                        |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| What crosses a mirror boundary    | Announcements + tombstones-as-notices cross; signatures, seq, policy do not — mirror re-signs under local policy (§3)                                              |
| Automatic discovery / refresh     | Operator-directed subscription only; scheduled re-fetch of subscribed lists is mechanical; transitive auto-subscription forbidden without a new design step (§3.1) |
| Loop and fan-out bounds           | No transitive traversal, so no loop protocol; fan-out is the reader's subscription set, never a crawl (§3.1)                                                       |
| Join handshake minimum            | Resolve → authorize-at-both-ends → mutual-grant portal channel; listing never grants, portals never leak through discovery (§4)                                    |
| Relay federation semantics        | Reuse `route_for`; anyone may operate a relay; relay learns timing/size only; fail closed with named reason when no shared relay (§5)                              |
| Interop minimum                   | Mirror + join + relay rules (§7); everything else explicitly not in the protocol                                                                                   |

## 9. Implementation breakdown

Filed from this record as separately scoped follow-on work (each its
own issue; design work only here — no code is authorized by this
record):

1. **Mirror-conformance vectors** — fixture registries A → B → C
   proving the §3 boundary: verbatim-entry checks, tombstone-notice
   handling, per-registry seq independence, unverifiable-list
   whole-drop. Known-answer vectors over `registry.rs` import/verify.
2. **Subscription-order resolver tests** — `client_resolver` drops
   non-subscribed fetched lists and keeps subscription order across
   mirror hops; a second list for the same key is dropped.
3. **Join-composition tests** — listed-but-ungranted resolves then
   refuses at authorization with the refusing end attributed;
   unlisted-in-registry is refused at read time across a mirror hop
   (`arbitrate` + `plan_join`).
4. **Relay-naming and fail-closed tests** — `route_for` matrix over
   host-hint × joiner-policy × shared-relay absence; both ends log
   the relay used; relay-required + relay-less fails with the
   disclosure string and no direct attempt.
5. **Book coverage** — mirror rules, join handshake, relay threat
   model, and the parity/backstop table (§6) written into the book
   from this record, where the v1/v2 registry and endpoint coverage
   lives.

## 10. Out of scope (explicit)

- Portal grant semantics themselves (#1068's step — composed with,
  not reopened).
- Hosted-instance operations (rate limits, abuse response, #1067).
- Key recovery / identity handover (accepted v1 limitation, v1
  §9.3, unchanged).
- Transitive/bounded automatic discovery (forbidden by default here;
  a future step must justify it — §3.1).
- Search ranking, recommendations, traffic-derived signals, relay
  directories, or any central index (excluded in v2 §5, upheld).
- Adjudicating name disputes, moderating tombstone reasons, or
  merging registry policies (curators' business — shown, never
  judged).
