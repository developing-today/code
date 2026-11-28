# Open discovery at scale — v4 design: curation and abuse resistance (#6290)

Design record for issue #6290 ("id(phase-14): Discovery, registry &
federation — v4 — outline: open discovery at scale"). Core topic: #6277
(Discovery, registry & federation). Predecessors: the v1 design #6287
(record `doc/2026-11-16_design_id6287_discovery_registry_federation_v1.md`,
merged in PR #10703), the v2 outline #6288, the v2 design #10712 (record
`doc/2026-11-27_design_id10712_public_registry_endpoint_v2.md`, merged in
PR #15943, commit `8830078fa235d96bee27b95791f39297cfc8551b`), the v3
outline #6289 (closed 2026-11-28T04:54Z as completed), and the v3 design
#16051 (record
`doc/2026-11-27_design_id16051_federation_protocol_v3.md`, merged in PR
#16144, commit `b2fe2f54d74dada8626118ec296daa53a2a1af60`, 2026-11-27T17:11Z).
Sibling steps: #6283 (hosted worlds v1) and #1068 (portals by mutual grant,
CLOSED) — this record composes with both and re-decides neither.

**Entry condition — verified 2026-11-28 (lane oc-code-06).** Both halves
met: (1) the v3 federation design is written and merged (PR #16144); its §8
decides the #6289/#16051 questions with defaults and §9 breaks out the
follow-ons; and (2) the v3 outline #6289 closed as completed
2026-11-28T04:54Z with all four acceptance criteria evidenced or handed to
#16051. #6290 carries no live claim (20 comments re-read 2026-11-28, latest
2026-11-25) and no v4 record exists in `doc/` or its index.

Status: v4 design. The #6290 scope's open questions are decided in §8 with
defaults; nothing here authorizes implementation beyond the breakdown in §9.

## 1. Governing principles (#1240, #6277) — how each one holds for curation

- **Local-first always.** Curation adds signal, never a dependency: a world
  with no registry subscription and no curator anywhere stays fully usable
  by anyone holding its link. Every mechanism in this record — subscription
  budgets, attestations, local scores, tombstone-notices — is an additive
  reading aid on top of that floor. A client that uses none of them loses
  nothing but convenience.
- **Hostable by anyone at any scale.** Curating needs no permission, no key
  of ours, and no central index: a curator is an operator with a signed
  registry file, a published policy string, and opinions expressed as their
  own tombstones. Curation state is files and URL lists an operator can
  copy, not accounts on our service.
- **P2P behavior identical to hosted behavior.** Same lists, same
  `arbitrate` fresher-seq-wins rule, same `client_resolver` subscription
  filter, same tombstone-notice semantics. The hosted instance differs only
  in which registries it subscribes to by default (our curated list), which
  budgets it sets, and which attestations it publishes — all settings, never
  protocol branches.
- **Our hosted instance is the public backstop, never a requirement.**
  Our curated registry, our delist log, and our attestations are defaults a
  fresh self-host does not inherit (`ClientDiscoveryConfig::fresh_self_host`
  follows none). Curation must work with the backstop offline: two
  self-hosts with each other's registry keys curate for themselves fully.

## 2. Disposition of the #6290 outline (accept / decide)

All four scope bullets are accepted as the shape of v4; the items the
outline left rough are decided in §8:

- **Spam worlds** — accepted: floods of signed announcements are a
  reader-side attention problem, not a verification problem. Decided as
  curator-local subscription budgets, never a protocol throttle (§3).
- **Impersonation** — accepted with the boundary kept: identity is the
  world key; display names are hints. "Verified" is a curator-signed
  attestation, curator-local, never protocol truth (§4).
- **Reputation** — accepted as curator-local computation over subscribed
  lists only. No global score, no shared feed, no traffic-derived signals
  (§5). The v2 §5 exclusion (no ranking, no popularity) is upheld and
  extended, not reopened.
- **Takedown and appeals** — accepted: the protocol keeps exactly the v3 §3
  tombstone-notice semantics; the new content is the transparency practice
  for our hosted curated registry and the appeal shape, both ops-side under
  #1067 (§6).

## 3. Spam worlds

Verification is cheap; attention is not. A spammer can sign ten thousand
announcements and every one of them verifies. The protocol therefore says
nothing about volume — a protocol throttle would be a central gate, and a
central gate is a central index by another name (v2 §5 exclusion holds).
Spam defense is three curator-local mechanisms, all settings:

1. **Subscription budgets.** The importing operator sets, per subscribed
   registry: a per-key entry velocity (new entries per key per window) and
   a per-list size cap. Entries beyond budget are not imported — silence,
   not a tombstone (v3 §3 rule 2: the mirror never speaks for keys it never
   listed). Budgets are local policy inputs to `Registry::import`, in the
   same class as the existing policy string: they never cross a mirror
   boundary and never appear in a served list.
2. **Whole-list drop unchanged.** A list that fails `verify_from` against
   its key is dropped whole (v3 §3 rule 1). Spam that cannot sign is already
   handled; budgets handle spam that can.
3. **Spam verdicts travel as notices.** A curator that judges key K a
   spammer expresses it exactly one way: a tombstone-notice under the
   curator's own key with its own reason and `registry_seq` (v3 §3 rule 3).
   It voids nothing elsewhere: a reader also subscribed to a list that still
   carries K's live announcement keeps seeing K under that list's layer
   (`arbitrate` treats the notice as `Outcome::Delisted` for the curator's
   layer only).

## 4. Impersonation

Identity is the world key, full stop. Announcement display names, icons,
and descriptions are self-asserted metadata — untrusted hints, useful for
rendering, worthless for authentication. From this follow four rules:

1. **Key-derived identifier always shown.** Wherever our clients show a
   display name, they show a key-derived identifier beside it (truncated key
   or stable visual hash — the exact rendering is client UI, not protocol).
   A name without a key beside it is a rendering bug.
2. **No protocol name registry.** A global namespace that maps names to
   keys would be a central index (v2 §5 exclusion holds). Squatted,
   confusable, or misleading names are handled by curator policy (delist
   under own key, §6), never by protocol adjudication.
3. **"Verified" is a curator-signed attestation.** A badge asserting "this
   key is the canonical key for name N" is a signed object under the
   curator's key, stored and served curator-locally, in the same
   never-crosses class as policy and `seq`. Readers that do not subscribe to
   that curator never see the badge; readers that do see it attributed to
   the curator, never as protocol truth. The attestation format itself is
   deferred to the breakdown (§9.2) — this record decides its trust class,
   not its bytes.
4. **Confusables are curator UI policy.** Homoglyph and lookalike handling
   (warnings, grouping, folding) lives in clients and curator front-ends. It
   is not in the protocol, not in the mirror rules, and not in `arbitrate`.

## 5. Reputation

Reputation is computed, per curator, over subscribed lists only — and it
never crosses a mirror boundary, in the same class as `seq` and policy:

- A curator's score for a world is a function of lists that curator
  subscribes to (age, persistence across fetches, breadth of independent
  listing, tombstone-notices for and against). The inputs are public; the
  function is the curator's own.
- Scores are served only inside the curator's own views, attributed to the
  curator. They are never embedded in mirrored announcements, never merged
  across registries (no policy merge, v3 §3 rule 5 — scores are policy), and
  there is no shared reputation feed.
- Clients may combine several curators' signals locally under explicit,
  user-chosen weighting. The weighting is a user setting, like the
  subscription set itself.
- **Traffic-derived signals are excluded** from any reputation our hosted
  instance publishes: no views, no joins, no popularity, no relay metadata.
  This extends the v2 §5 exclusion to v4. An operator that wants
  traffic-derived ranking runs their own curator with their own published
  method — and our clients treat it as one curator's opinion among others.

## 6. Takedown and appeals

The protocol is unchanged from v3 §3: tombstones cross as notices, never as
vetoes; a mirror may ignore a notice, display it alongside a continued
listing, or adopt a delisting under its own key with its own reason. At open
discovery scale this neutrality is the whole point — one curator's takedown
never voids another curator's listing, and no appeal is adjudicated by the
protocol. The new content in v4 is the practice around it:

1. **Hosted transparency practice (ops, under #1067).** Our hosted curated
   registry commits to: every delist recorded with world key, reason,
   timestamp, and the curator key that signed it; the log publicly readable;
   every delist naming its appeal channel. This is an operations promise by
   one curator, not a protocol rule — any curator may adopt the same shape,
   none is required to.
2. **Appeal shape.** An appeal is a new signed statement by the world key
   (a fresh announcement, or a reply through the published appeal channel).
   A curator that accepts an appeal re-lists by a new include under its own
   key — there is no "un-tombstone" and no edit of history. The record shows
   both the delist and the re-list, each attributed.
3. **No cross-curator veto, no escalation path in-protocol.** A world
   delisted by our curated registry remains listed everywhere else it
   verifies; its invite links keep working everywhere. "Deplatformed from
   open discovery" is not a state the protocol can express.

## 7. Synthesis: the smallest interoperable curation layer

Four rules, each testable against the landed modules plus curator-side
behavior. Anything not in these four rules is not in the curation layer:

1. **Copy rule (v3 restated).** Announcements and tombstone-notices cross;
   trust never does. Curation verdicts — delists, scores, attestations —
   are new signed objects under the curator's key, never forwards of
   another key's objects.
2. **Attribution rule.** Every served view names its curator and the signing
   key of each row; every tombstone reason displays with its author's key;
   every score and badge displays with the curator that computed it.
3. **Locality rule.** Budgets, scores, attestations, and policies are
   computed and stored per curator and never cross a mirror boundary. A
   reader comparing two curators compares two opinions, never two truths.
4. **Identity rule.** The world key is the identity; display names are
   hints; a key-derived identifier is shown wherever a name is shown.

## 8. Open choices — decided here with defaults

| Choice (deferred by #6290)                      | v4 decision                                                                                                                                                                                                           |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Spam: protocol throttle vs curator budgets      | Curator-local subscription budgets (per-key velocity, per-list size cap) as `import` policy inputs; no protocol throttle; verdicts travel only as tombstone-notices (§3)                                              |
| Impersonation: name registry vs key identity    | No protocol name registry; identity is the key; key-derived identifier shown with every name; "verified" is a curator-signed, curator-local attestation; format deferred to §9.2 (§4)                                 |
| Reputation: shared vs local                     | Per-curator computation over subscribed lists only; never mirrored, never merged, no global score; client-side combination under user weighting; traffic-derived signals excluded from our hosted scores (§5)         |
| Takedown: veto vs notice                        | Protocol keeps v3 notice semantics; our hosted registry adds a transparency log + appeal channel as ops practice under #1067; appeals re-list as new includes; no cross-curator veto (§6)                             |
| Open discovery at scale without a central index | Open means anyone may publish/subscribe; scale via mirror fan-out bounded by the reader's subscription set, never crawl; public endpoints (v2 §9) attribute every row by key and name the curator; no federated query |

## 9. Implementation breakdown

Filed from this record as separately scoped follow-on work (each its own
issue; design work only here — no code is authorized by this record):

1. **Subscription policy budgets** — the per-key velocity and per-list size
   cap settings as `import`/client-config inputs: setting shape, default
   values for a fresh self-host vs our hosted defaults, over-budget silence
   semantics, and named tests over `registry.rs` import/verify.
2. **Curator-attestation annotation format** — the signed "key K is canonical
   for name N" object: fields, signing, expiry, display rules beside
   key-derived identifiers, and the rule that attestations never cross
   mirrors as trust.
3. **Local reputation computation** — per-curator scoring over subscribed
   lists: named inputs, the hosted instance's published method, client-side
   combination weighting as a user setting, and the traffic-signal exclusion
   stated as a testable rule.
4. **Hosted delist-transparency and appeals practice** — ops note under
   #1067: delist-log shape (key, reason, timestamp, curator key), public
   read path, appeal channel, and re-list-as-new-include procedure.
5. **Book coverage** — the four curation rules (§7), the spam/impersonation/
   reputation/takedown decisions, and the curated-default vs fresh-self-host
   settings table, written into `pkgs/id/book/` where the v1/v2 registry and
   endpoint coverage lives.

## 10. Out of scope (explicit)

- Portal grant semantics themselves (#1068's step — composed with, not
  reopened).
- Hosted-instance operations: rate limits, abuse response staffing (#1067).
- Key recovery / identity handover (accepted v1 limitation, v1 §9.3,
  unchanged).
- Transitive/bounded automatic discovery (forbidden by default in v3 §3.1;
  a future step must justify it — this record does not).
- Federated query, search ranking, relay directories, or any central index
  (excluded in v2 §5, upheld in v3 §7, upheld here).
- A protocol name registry or global namespace (refused in §4 rule 2).
- Traffic-derived reputation signals in our hosted scores (excluded in §5).
- Adjudicating disputes between curators, or merging curator policies
  (curators' business — shown, never judged).

---

**Verified when:** the four #6290 acceptance criteria hold — the entry
condition evidenced above (§1 header: PR #16144 merged, #6289 closed); this
record merged with the `doc/README.md` index line; the §9 breakdown filed as
linked issues from #6290 and the core topic #6277; and the outcome (record
link, merge SHA, verification evidence) posted on #6290.
