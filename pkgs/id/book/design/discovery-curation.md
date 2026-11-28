# Design: open discovery — curation and abuse resistance

This chapter carries the v4 open-discovery design (issue #6290, core topic
#6277) into the book: how open discovery stays usable at scale without a
central index. The design record is
`doc/2026-11-28T06-10-00Z_design_id6290_open_discovery_v4/` (merged in PR
#16496); this chapter is its book coverage, beside the v1/v2 registry and
endpoint coverage elsewhere in this book. It is a design record only — no
protocol or client behaviour changes are described as shipped.

The governing floor is unchanged from v1–v3: curation adds signal, never a
dependency. A world with no registry subscription and no curator anywhere
stays fully usable by anyone holding its link. Every mechanism below —
subscription budgets, attestations, local scores, tombstone-notices — is an
additive reading aid on top of that floor, and peer-to-peer behaviour is
identical to hosted behaviour: the hosted instance differs only in its
default settings, never in protocol branches.

## The four curation rules (design §7)

The curation layer is exactly these four rules. Anything not in them is not
in the curation layer:

1. **Copy rule.** Announcements and tombstone-notices cross; trust never
   does. Curation verdicts — delists, scores, attestations — are new signed
   objects under the curator's key, never forwards of another key's objects.
2. **Attribution rule.** Every served view names its curator and the signing
   key of each row; every tombstone reason displays with its author's key;
   every score and badge displays with the curator that computed it.
3. **Locality rule.** Budgets, scores, attestations, and policies are
   computed and stored per curator and never cross a mirror boundary. A
   reader comparing two curators compares two opinions, never two truths.
4. **Identity rule.** The world key is the identity; display names are
   hints; a key-derived identifier is shown wherever a name is shown.

## Spam worlds (design §3)

Verification is cheap; attention is not. A spammer can sign ten thousand
announcements and every one of them verifies, so the protocol says nothing
about volume — a protocol throttle would be a central gate, and a central
gate is a central index by another name. Spam defence is three curator-local
mechanisms, all settings:

- **Subscription budgets.** The importing operator sets, per subscribed
  registry, a per-key entry velocity (new entries per key per window) and a
  per-list size cap. Entries beyond budget are not imported — silence, not a
  tombstone: the mirror never speaks for keys it never listed. Budgets are
  local policy inputs to registry import; they never cross a mirror boundary
  and never appear in a served list.
- **Whole-list drop unchanged.** A list that fails verification against its
  key is dropped whole. Spam that cannot sign is already handled; budgets
  handle spam that can.
- **Spam verdicts travel as notices.** A curator that judges a key a spammer
  expresses it exactly one way: a tombstone-notice under the curator's own
  key, with its own reason. It voids nothing elsewhere — a reader also
  subscribed to a list that still carries the key's live announcement keeps
  seeing it under that list's layer.

## Impersonation (design §4)

Identity is the world key, full stop. Announcement display names, icons, and
descriptions are self-asserted metadata — untrusted hints, useful for
rendering, worthless for authentication. Four rules follow:

- **Key-derived identifier always shown.** Wherever a client shows a display
  name, it shows a key-derived identifier beside it (truncated key or stable
  visual hash; the exact rendering is client UI, not protocol). A name
  without a key beside it is a rendering bug.
- **No protocol name registry.** A global namespace mapping names to keys
  would be a central index. Squatted, confusable, or misleading names are
  handled by curator policy (delist under the curator's own key), never by
  protocol adjudication.
- **"Verified" is a curator-signed attestation.** A badge asserting "this
  key is the canonical key for name N" is a signed object under the
  curator's key, stored and served curator-locally. Readers that do not
  subscribe to that curator never see the badge; readers that do see it
  attributed to the curator, never as protocol truth.
- **Confusables are curator UI policy.** Homoglyph and lookalike handling
  (warnings, grouping, folding) lives in clients and curator front-ends —
  not in the protocol, the mirror rules, or arbitration.

## Reputation (design §5)

Reputation is computed, per curator, over subscribed lists only — and it
never crosses a mirror boundary:

- A curator's score for a world is a function of the lists that curator
  subscribes to (age, persistence across fetches, breadth of independent
  listing, tombstone-notices for and against). The inputs are public; the
  function is the curator's own.
- Scores are served only inside the curator's own views, attributed to the
  curator. They are never embedded in mirrored announcements, never merged
  across registries, and there is no shared reputation feed and no global
  score.
- Clients may combine several curators' signals locally under explicit,
  user-chosen weighting. The weighting is a user setting, like the
  subscription set itself.
- **Traffic-derived signals are excluded** from any reputation the hosted
  instance publishes: no views, no joins, no popularity, no relay metadata.
  An operator that wants traffic-derived ranking runs their own curator with
  their own published method — and clients treat it as one curator's opinion
  among others.

## Takedown and appeals (design §6)

The protocol keeps the v3 tombstone-notice semantics unchanged: tombstones
cross as notices, never as vetoes. A mirror may ignore a notice, display it
alongside a continued listing, or adopt a delisting under its own key with
its own reason. One curator's takedown never voids another curator's
listing, and no appeal is adjudicated by the protocol. The v4 content is the
practice around it:

- **Hosted transparency practice.** The hosted curated registry commits to:
  every delist recorded with world key, reason, timestamp, and the curator
  key that signed it; the log publicly readable; every delist naming its
  appeal channel. This is an operations promise by one curator, not a
  protocol rule — any curator may adopt the same shape, none is required to.
- **Appeal shape.** An appeal is a new signed statement by the world key (a
  fresh announcement, or a reply through the published appeal channel). A
  curator that accepts an appeal re-lists by a new include under its own
  key — there is no "un-tombstone" and no edit of history. The record shows
  both the delist and the re-list, each attributed.
- **No cross-curator veto, no escalation path in-protocol.** A world
  delisted by the curated registry remains listed everywhere else it
  verifies; its invite links keep working everywhere. "Deplatformed from
  open discovery" is not a state the protocol can express.

## Curated-default vs fresh-self-host settings

The hosted instance is the public backstop, never a requirement. The only
differences between the hosted curated default and a fresh self-host are
settings:

| Setting | Hosted curated default | Fresh self-host |
| --- | --- | --- |
| Default registry subscriptions | Subscribes to our curated registry list by default | Follows none — inherits no subscriptions (`fresh_self_host` follows none) |
| Subscription budgets | Budgets set by the hosted operator (per-key velocity, per-list size cap) | No inherited budgets; the operator sets their own |
| Attestations published | Publishes its own curator-signed attestations | Publishes none by default |
| Reputation scores | Serves scores computed by its published method; traffic-derived signals excluded | No inherited scores; computes only over lists it subscribes to, if it curates at all |
| Delist transparency log | Commits to a publicly readable delist log (key, reason, timestamp, signing curator key) with a named appeal channel | Not required; may adopt the same shape voluntarily |
| Appeals | Re-lists accepted appeals as new includes under its own key | Decides its own appeal practice; protocol adjudicates none |

Curation must work with the backstop offline: two self-hosts holding each
other's registry keys curate for themselves fully. The concrete default
budget values, the attestation object format, the local reputation
computation, and the hosted delist-transparency practice are the separately
scoped follow-ons in design §9; this chapter records the decisions and the
settings shape, not those values.
