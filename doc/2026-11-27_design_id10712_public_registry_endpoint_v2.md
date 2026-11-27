# Discovery, registry & federation — v2 design: the public registry endpoint (#10712)

Design record for issue #10712 ("id(phase-14): Discovery, registry &
federation — v2 — design: the public registry endpoint"). Core topic:
#6277 (Discovery, registry & federation). Predecessors: the v1 design
#6287 (record `doc/2026-11-16_design_id6287_discovery_registry_federation_v1.md`,
merged in PR #10703) and the v2 outline #6288 (refined outline posted
2026-11-16, which this record decides point by point). Sibling step:
#6283 (hosted worlds v1) — this record composes with its decisions and
does not re-decide them.

**Entry condition — verified 2026-11-27 (task 26089, lane code-f).**
The condition had two halves, both now met and recorded on #10712:
(1) the v1 implementation items this layer stands on have landed —
#10707 (registry list format + reference publisher + mirror tooling,
`pkgs/id/crates/id-sync/src/registry.rs`) and #10709 (client
discovery settings, `discovery_client.rs` via PRs #14671 and #15694),
with the supporting resolver #10708 closed (`discovery_resolver.rs`);
and (2) no hold labels and no linked PRs on #10712 at check time.
#1067 is OPEN (hosted-operation fair scheduling — governs the
endpoint's operation, not its protocol; see §10). #1068 is CLOSED
(portals by mutual grant — a registry inclusion never overrides a
portal grant; see §10).

Status: v2 design. The #6288 outline's open questions are decided in
§12 with defaults; nothing here authorizes implementation beyond the
breakdown in §13.

## 1. Governing principles (#1240) — how each one holds for the endpoint

- **Local-first always.** The endpoint is a window, never a
  dependency: every query it answers is computable by any client over
  its own local copies of subscribed lists (`Resolver` +
  `search` in `discovery_resolver.rs` are the same code the endpoint
  runs). A client that never touches the endpoint loses nothing but
  convenience.
- **Hostable by anyone at any scale.** The endpoint is an instance of
  a general presentation tool over any signed list, not a privileged
  platform. Anyone holding a registry list file can serve the same
  views over it; operating one requires no permission, no key of
  ours, and no central index.
- **P2P behavior identical to hosted behavior.** Same lists, same
  query code, same arbitration (`arbitrate`'s fresher-seq-wins rule
  applies everywhere). The hosted instance differs only in which
  registry it subscribes to by default (our curated list) — a
  setting, not a protocol branch.
- **Our hosted instance is the public backstop, never a
  requirement.** The first endpoint serves our curated registry
  because that curation exists, not because discovery routes through
  it. Self-hosts default to no subscription
  (`ClientDiscoveryConfig::fresh_self_host`); the endpoint never
  gates joining, publishing, or resolving.

## 2. Disposition of the #6288 outline (accept / decide)

All five outline bullets are accepted as the shape of v2; the items
the outline deferred are decided in §12:

- **Browsing, search, filters** — accepted: text over
  `name`/`description`, exact match over `tags`, enum filters over
  `occupancy`/`visibility`, computed over the local copy of each
  subscribed list. The query layer generalizes the landed
  `search()` (name-contains + tag-exact, deduped to one row per
  world by highest seq, ordered by name) with attribution (§6) and
  honesty display (§7) added.
- **Freshness / decay from announcement fields** — accepted as
  mechanical derivation only (§4): `expires_at` voids, heartbeat
  republication keeps a live world visible, monotone `seq`
  arbitrates. Soft demotion inside a lifetime is decided in §12:
  no — age and heartbeat continuity are displayed as facts, never
  folded into a score.
- **Signal scope** — accepted with the burden kept: only
  verifiable, non-gameable, list-computable signals (§5).
  Popularity stays out; the reconciliation with v1 §12 is in §5,
  and it upholds the exclusion.
- **Multi-registry semantics** — decided in §6: union of worlds
  with per-registry attribution, never intersection and never a
  merged ranking.
- **Listing honesty** — decided in §7: key provenance is mandatory
  display; names stay claims.

## 3. The query layer

The endpoint answers queries over **local copies of subscribed
registry lists** — the lists the serving operator subscribes to,
fetched and verified exactly as a client verifies them
(`SignedRegistryList::verify_from` against the chosen registry key,
every entry announcement re-checked against its own `world_id`).
There is no index of worlds outside subscribed lists, no crawl, no
aggregation feed. A query names a set of lists (default: all the
operator subscribes to) and returns rows; each row carries the
attribution of §6 and the honesty display of §7.

Filters, all optional and conjunctive:

- `q`: case-insensitive substring over `name` and `description`
  (empty lists everything live — browsing is just search without a
  query, as in the landed `search()`).
- `tag`: exact case-insensitive match against the announcement's
  `tags`. Tags are discovery hints only: a match says what a
  registry holds, never that the world is what its tags claim (§7).
- `occupancy`: one of `has_room` / `near_full` / `full`. Coarse by
  design (v1 §3) — the endpoint must not invent precision the
  announcements deliberately withhold.
- `visibility`: `listed` only, always. Unlisted worlds are refused
  at inclusion and re-checked at read time (`registry.rs`
  `check_shape`, `arbitrate`'s `check`); the endpoint has no way to
  show what it must not hold.

Ordering is fixed and opinion-free: by name, then by `world_id`
(the landed rule). No relevance ranking exists to tune, on purpose.

## 4. Freshness and decay — functions, not scores

Derived mechanically from announcement fields; the endpoint holds no
per-world server-side state:

- **Expired is void.** `live_entries(now)` (`now <
  expires_at`) is the only liveness gate. An expired announcement
  does not appear softened, greyed, or "possibly stale" — it does
  not appear. Silence is self-cleaning (v1 §9.1) and the endpoint
  must not second-guess it.
- **Heartbeat continuity is a displayed fact.** Republication at
  half the record's effective lifetime (v1 §4) means a live world
  normally shows a chain of increasing `seq` with fresh
  `issued_at`s. The endpoint shows, per row, the current `seq` and
  the age since `issued_at`. A world whose announcement is days old
  but unexpired looks exactly like what it is: valid, ageing, one
  missed heartbeat away from void. No curve, no half-life badge, no
  "freshness score" — any soft curve would be a ranking function,
  and ranking is excluded (§5).
- **Cross-layer freshness follows `arbitrate`.** Where layers
  disagree about one world, the highest valid `seq` wins; a
  tombstone in one registry is an outcome (`Outcome::Delisted`),
  never a veto over a live listing elsewhere. The endpoint shows
  both: the live row with its attribution, and the delisting notice
  alongside it (§6).

## 5. The signal set — and the gaming analysis that bounds it

Admitted signals must be **verifiable** (checkable from the lists),
**non-gameable** (costly or pointless to fake), and
**list-computable** (no traffic telemetry, no accounts, no
per-client state). Exactly three qualify:

1. **Curator inclusion, attributed by signature.** "Registry R,
   key K, includes this world under policy P" — verifiable by
   anyone holding R's list. Gaming it requires persuading (or
   becoming) a curator whose key readers trust; that cost *is* the
   signal. The endpoint attributes every row to every subscribed
   list that carries it (§6), so inclusion is always somebody's
   stated opinion, never the endpoint's.
2. **Tombstone-history transparency.** Delistings propagate with
   reasons (`Tombstone { world_id, removed_at, reason,
   registry_seq }`). A world delisted-and-relisted, or delisted by
   one curated registry while kept by another, shows that history.
   Gaming it (delist to suppress) fails openly: the tombstone is
   signed by the delister and the world remains listed wherever
   else it is listed.
3. **Republication continuity.** The `seq` chain and `issued_at`
   freshness of a world's announcements, per §4. Faking liveness
   requires the world key — which is to say, requires *being* the
   world. Self-attested uptime is weak evidence of quality and is
   presented as exactly that: continuity, not endorsement.

**Popularity and ranking: excluded, with reasons (reconciling v1
§12).** Join counts, view counts, and any traffic-derived metric are
out for three compounding reasons: (a) they are gameable by the
world operator at near-zero cost (self-joins, refresh loops);
(b) collecting them truthfully requires per-client observation,
which turns the endpoint into a tracking central index — the exact
platform v1 §12 refused ("discovery is lists and keys, not a
platform"); and (c) only a high-traffic endpoint could compute
them, which would centralize discovery around whoever already has
traffic. This step was charged to overturn the exclusion with a
gaming-resistant design or uphold it; it upholds it. Revisit only
with a signal that is verifiable from lists alone — e.g. a future
design where inclusion *policies* themselves become comparable
objects, which is a new design step, not a parameter tweak.

## 6. Multi-registry disagreement semantics

Two registries disagreeing about a world is the expected state (v1
§5), so the endpoint **presents disagreement, never resolves
it**:

- **Union of worlds, one row per world.** A world listed by any
  queried registry appears once. Dedupe key is `world_id`; where
  registries hold different `seq`s for one world, the row shows the
  highest valid one (the `arbitrate` rule), with each registry's
  held `seq` visible in the attribution.
- **Per-registry attribution on every row.** Each row names, for
  every queried registry: listed (with that list's `included_at`
  and `registry_seq`), delisted (with `removed_at` and the
  tombstone `reason`), or silent (neither entry nor tombstone —
  "never seen", honestly labeled as such, not as disapproval).
- **No intersection mode, no merged score.** Intersection would let
  the strictest curator silently veto the rest; a merged score
  would be ranking by another name (§5). A reader wanting "worlds
  my three trusted registries all list" gets it by querying those
  three lists and reading the attribution — the tool shows, the
  reader decides.
- **Tombstones do not veto.** A delisting in registry A appears as
  A's stated opinion on the row; the row itself stands while any
  queried registry lists it live. Suppression-by-delist fails in
  the open, by construction.

## 7. Listing display — names are claims

The endpoint's honesty rules, mandatory on every surface (page,
JSON, CLI):

- **Key provenance always surfaced.** Every row shows the
  `world_id` (truncated hex with full key one action away) next to
  the name, and every row states the signing attribution: the
  world's own key for the announcement, each listing registry's key
  for its inclusion. A name without its key on screen is a display
  bug.
- **Same-name collisions are flagged, not hidden.** When two
  listed worlds share a normalized name, both rows carry a
  lookalike notice pointing at each other. The endpoint never
  picks "the real" one — keys distinguish, names merely suggest.
- **Tags labeled as hints.** Any tag-derived match is badged as
  matching an unsigned hint (v1 §3: registries may filter on tags,
  clients must not trust them). Filter UI repeats the warning
  where tags can be entered.
- **Policy linked.** Each attributing registry's inclusion policy
  statement (`RegistryList.policy`, ≤ 2,000 chars) is one action
  from every row it attributes — "included" always comes with "by
  whom, under what stated rules".

## 8. Mirrors

A mirror re-signs its chosen subset under its own key with its own
tombstones (`Registry::import`), so from the query layer's view a
mirror **is** a registry like any other: attribution is by the
list's signing key, full stop. No protocol distinction, no mirror
badge, no origin-listed-vs-mirror-included split in the row model —
a reader trusting mirror M's key gets M's opinions, whatever lists
M imported. Convention (not rule): mirrors SHOULD state what they
mirror in their `policy` text, so readers can follow the chain;
the endpoint renders policy text verbatim and verifies nothing
about it. The §6 attribution already answers "who says so", which
is the only question the protocol asks.

## 9. Endpoint shape (first instance)

- **Read-only HTTP presentation** over the operator's subscribed
  list copies: browse/search with the §3 filters, per-world detail
  (announcement verbatim, §6 attribution, §7 provenance), and the
  registries' policy statements. No submission, no accounts, no
  query logging beyond aggregate operation counters — per-client
  observation would be the telemetry §5 refuses.
- **Same code as the client.** Query, arbitration, and liveness
  are the `discovery_resolver` functions, not a reimplementation;
  a divergence between endpoint answers and client answers is a
  bug in the endpoint.
- **CLI alongside.** A `id registry` surface (browse/search/detail
  over subscribed lists) ships with the endpoint so self-hosters
  get the tool without operating the service — hostable-by-anyone
  includes usable-by-anyone.
- **Operation is the hosted step's business.** Rate limits, abuse
  response, and relay-gateway posture belong to hosted operations
  (with #1067 fair scheduling named, still OPEN); none of them
  changes the protocol or the query semantics above.

## 10. Dependencies — named, not assumed (fresh states, 2026-11-27)

| Dependency                    | State              | Bearing on v2                                                                                                                                     |
| ----------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| #812 world directory feed     | CLOSED (not built) | Still not built; the announcement remains the primitive (§2 of v1)                                                                                |
| #1061 F12.4 provider registry | CLOSED             | Manifest/capability pattern stays a closed input                                                                                                  |
| #1067 F12.10 fair scheduling  | OPEN               | Hosted-operation concern: governs the endpoint's operation (rate limits, abuse), not its protocol (§9)                                            |
| #1068 F12.14 portals          | CLOSED 2026-11-16 | Mutual-grant messaging exists; a registry inclusion never overrides a portal grant (v1 §8 holds unchanged)                                        |
| #1069 F12.17 trust            | CLOSED             | Signature-vs-admin-list pattern generalized per-registry in v1; v2's per-key attribution is the same pattern at display time                        |
| #6283 hosted worlds v1        | CLOSED             | Predecessor step; the worlds this endpoint lists are the worlds that step hosts                                                                   |
| #10707 registry publisher     | CLOSED             | `registry.rs`: list format, publisher, mirror import, tombstones — the objects §3–§8 query                                                        |
| #10709 client settings        | CLOSED (#14671, #15694) | `discovery_client.rs`: subscription setting + visibility scopes — the reader side the endpoint mirrors                                        |
| #10708 resolver               | CLOSED             | `discovery_resolver.rs`: `search`, `arbitrate`, `registry_offer` — the functions the endpoint reuses, not reimplements                            |
| `pkgs/pkarr-worker`           | operational        | The pkarr layer under §4's freshness story                                                                                                        |

## 11. What breaks first — and the v2 mitigation for each

1. **Lookalike names on a public page.** A public endpoint
   amplifies squatting: anyone can announce any name ≤ 64 chars.
   Mitigation is display, not adjudication: mandatory key
   provenance + collision flags (§7). The endpoint never delists
   for confusing similarity — that judgment is each curator's, in
   their own list.
2. **Curated-default gravity.** If everyone reads our endpoint,
   our curation becomes discovery in practice. Mitigation is
   structural honesty: attribution always names the curator (§6),
   the CLI ships the same tool for any list (§9), and self-hosts
   still default to no subscription. Gravity we admit; gatekeeping
   we refuse.
3. **Stale-list staleness.** The endpoint's copies can lag the
   registries it mirrors. Mitigation: every view states each
   list's `published_at` and `seq` — a reader can see the copy's
   age and distrust it accordingly. Serving a list whose copy is
   older than the announcement lifetime it contains is a
   configuration error the endpoint should refuse, not smooth
   over.
4. **Tombstone grudges.** A registry can delist with a hostile or
   false reason, displayed verbatim. Mitigation: reasons render
   attributed and unedited (§6) — the delister owns their words
   publicly, and other lists' continued inclusion answers them.
   No endpoint-side moderation of reasons: that would make the
   endpoint a curator of curators.
5. **Gaming the continuity signal.** Heartbeat chains prove key
   custody, not quality (§5). Mitigation is labeling: continuity
   is presented as continuity, and any future attempt to read
   quality into it is a new design step with its own gaming
   analysis, not a display tweak.

## 12. Open choices — decided here with defaults

| Choice (deferred by #6288) | v2 decision |
| --------------------------- | ----------- |
| Query-layer scope | General tool over subscribed list copies; text/tag/occupancy filters, conjunctive; fixed name-then-key ordering (§3) |
| Soft demotion before expiry | No — hard expiry only; age and heartbeat continuity displayed as facts (§4) |
| Signal set | Curator inclusion + tombstone transparency + republication continuity; popularity/ranking excluded, v1 §12 upheld (§5) |
| Multi-registry disagreement | Union of worlds, one row per world, per-registry attribution; tombstones never veto (§6) |
| Listing honesty | Mandatory key provenance, lookalike flags, tags badged as hints, policy one action away (§7) |
| Mirror presentation | No protocol distinction; attribute by signing key; origin declared in policy by convention (§8) |
| First instance | Read-only HTTP over our curated subscription + same-code CLI; operation (limits/abuse) is the hosted step's, with #1067 named (§9) |

## 13. Implementation breakdown

Filed from this record as separately scoped follow-on work (each its
own issue; design work only here — no code is authorized by this
record):

1. **Query filters over subscribed lists** — extend the landed
   `search()` with `occupancy` filtering and multi-list
   attribution payloads (the §3 filter set and §6 row model),
   with known-answer vectors over fixture lists.
2. **Freshness display fields** — per-row `seq`, `issued_at` age,
   and per-list `published_at`/`seq` staleness refusal (§4,
   breakage 3), computed, never scored.
3. **Signal computation from lists alone** — curator-attribution
   sets, tombstone histories, continuity chains (§5), each with
   its gaming-analysis test (forged popularity input must change
   nothing).
4. **Read-only endpoint service** — HTTP browse/search/detail over
   the operator's subscriptions (§9), reusing `discovery_resolver`
   functions; aggregate-only counters, no per-client state.
5. **CLI surface** — browse/search/detail over subscribed lists
   for self-hosters (§9), same query code path as the endpoint.
6. **Book coverage** — the endpoint's views, filters, and honesty
   rules written into the book from this record, so the hosted
   instance's public window is documented where its curation
   lives.

## 14. Out of scope (explicit)

- Federation _messaging_ between worlds (portals, #1068's step).
- Hosted-instance operations (rate limits, abuse response) — the
  hosted step's record, not this protocol's (§9).
- Key recovery / identity handover (accepted v1 limitation, v1
  §9.3, unchanged).
- Search ranking, recommendations, traffic-derived signals, or any
  central index (excluded in §5, upholding v1 §12).
- Adjudicating name disputes or moderating tombstone reasons
  (curators' business, §§7/11.4 — the endpoint shows, never
  judges).
