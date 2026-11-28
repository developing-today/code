# Discovery v4 — hosted delist-transparency and appeals practice (#16502)

Ops note for issue #16502 ("id(phase-14): Discovery v4 — hosted
delist-transparency and appeals practice"). This is §9 item 4 of the
v4 curation design (PR #16496, merged 2026-11-28T06:22:44Z, merge
commit `0d7612ff5d1c3f1dd4cecdcfedde73ac8f5a9491`; record
`doc/2026-11-28T06-10-00Z_design_id6290_open_discovery_v4/`, §6 and
§9). Parent outline #6290; core topic #6277; ops parent #1067 —
hosted-operation scheduling under #1067 is the owning ops step for
the practice this note specifies. Sibling steps: #16499 (§9 item 1),
#16500 (§9 item 2, record
`doc/2026-11-28T08-31-00Z_design_id16500_curator_attestation/`),
#16501 (§9 item 3, record
`doc/2026-11-28T08-13-00Z_design_id16501_local_reputation/`).

Status: ops note only. No code, and no protocol changes: the v3 §3
notice semantics stand exactly as decided; this note introduces no
cross-curator veto and no in-protocol adjudication. It specifies the
practice our hosted curated registry commits to as one curator's
operations promise (v4 §6 item 1) — any curator may adopt the same
shape, none is required to.

**Grounding — verified 2026-11-28 (lane 0222-L, firsthand from the
v4 record and the issue).**

- V4 §6 fixes the semantics this note operationalizes and does not
  re-decide: tombstones cross as notices, never as vetoes; a mirror
  may ignore a notice, display it alongside a continued listing, or
  adopt a delisting under its own key with its own reason; and no
  appeal is adjudicated by the protocol.
- The hosted commitments are enumerated in v4 §6 item 1: every
  delist recorded with world key, reason, timestamp, and the curator
  key that signed it; the log publicly readable; every delist naming
  its appeal channel.
- The appeal shape is fixed by v4 §6 item 2: an appeal is a new
  signed statement by the world key; acceptance re-lists by a new
  include under the curator's own key — there is no "un-tombstone"
  and no edit of history; the record shows both the delist and the
  re-list, each attributed.
- Issue #16502's acceptance criteria require exactly: this note
  merged and linked under #1067 covering the delist-log shape, the
  public read path, the published appeal channel, and the
  re-list-as-new-include procedure; the outcome posted on the issue;
  and no protocol changes.

## 1. The delist log — shape

Every delist performed by the hosted curated registry appends
exactly one entry to the hosted delist log. Entries are append-only:
an entry is never edited, reordered, or removed after publication
(corrections are new entries that reference the entry they correct,
§4 step 5).

| #   | Field            | Shape                                                                  | Rules                                                                                                                                                           |
| --- | ---------------- | ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L1  | `world_key`      | public key of the delisted world                                       | The subject of the delist — the key, never a display name (names are hints, v4 §4 rule 1).                                                                      |
| L2  | `reason`         | the tombstone-notice reason string, verbatim                           | The same reason carried by the tombstone-notice the delist issued under the curator's key (v3 §3 rule 3). The log quotes the notice; it does not paraphrase it. |
| L3  | `timestamp`      | seconds, UTC                                                           | The instant the delist took effect in the hosted registry's served views (the notice's issuance time).                                                          |
| L4  | `curator_key`    | the hosted registry's signing (curator) key                            | The key that signed the tombstone-notice. Every entry is attributable to this key (v4 §7 attribution rule).                                                     |
| L5  | `notice_ref`     | reference to the tombstone-notice object (its `registry_seq` under L4) | Lets any reader holding the notice verify the log entry against the signed object itself — the log is an index over signed notices, not a replacement for them. |
| L6  | `appeal_channel` | the published appeal channel identifier (§3)                           | Recorded per entry, so an entry stays self-contained even if the channel later changes: the channel named is the one in force for that delist.                  |

One entry per delist action. A world delisted, re-listed, and
delisted again produces two delist entries and one re-list entry
(§4) — the log is the full history, not the current state; the
current state is the registry's served view, derivable from the log
but never substituted for it.

## 2. The public read path

- The log is published as a read-only, unauthenticated view of the
  hosted registry, at the same public endpoint family the hosted
  registry already serves its lists from (v2 §9 public endpoints:
  every row attributed by key, curator named). Reading the log
  requires no account, no subscription, and no key.
- The published view serves entries in append order, complete —
  including entries for worlds later re-listed (their re-list
  entries appear alongside, §4). There is no redacted or summary
  variant: a transparency log that can omit entries is a press
  release, not a log.
- The log is a view over the curator's signed tombstone-notices
  (L5), not a mirrored object: other curators and mirrors do not
  replicate it, and its absence from a mirror says nothing (mirrors
  carry announcements and notices, v3 §3 — the practice layer is
  curator-local, v4 §7 locality rule).

## 3. The published appeal channel

- The hosted registry publishes one standing appeal channel for its
  delists, named in every log entry (L6) and on the registry's
  public view beside the log itself. The channel accepts a signed
  statement from the delisted world key — a fresh announcement, or
  a reply signed by that key through the channel (v4 §6 item 2).
- An appeal is addressed to the hosted registry as curator, and to
  no one else: it asks this curator to re-list under its own key.
  It does not petition any other curator, any mirror, or the
  protocol, and no other party can accept or reject it.
- Appeals carry no filing window and no fee-equivalent: a delist
  from any past date may be appealed at any later date. The
  registry's response — re-list (§4) or a published decline entry
  (§4 step 4) — is itself part of the log's history.

## 4. The re-list-as-new-include procedure

1. **Appeal received** through the published channel (§3), signed
   by the world key named in the delist entry (L1). An unsigned
   appeal, or one signed by a different key, is not an appeal from
   that world and is not entered in the log.
2. **Curator decision.** The hosted registry, as curator, accepts or
   declines under its own policy. The protocol has no say and no
   other curator has a vote (v4 §6 item 3).
3. **On acceptance — new include.** The world is re-listed by a new
   include under the curator's own key: a fresh list entry with its
   own new `registry_seq`, exactly as if the world were being
   listed for the first time. The original tombstone-notice is not
   withdrawn, voided, or amended — there is no un-tombstone.
4. **On decline — decline entry.** A short log entry records the
   appeal, its timestamp, and the decline reason, attributed to the
   curator key (L4 shape). A silent decline would leave the log
   claiming a history it does not have.
5. **History stands.** The log after a re-list shows, in order: the
   delist entry, the appeal, and the re-list entry (the new
   include's key/`registry_seq` reference). No entry is edited to
   reflect the outcome of a later one; corrections to any entry are
   appended as new entries referencing the corrected entry.

## 5. Boundaries (what this practice is not)

- **Not a protocol rule.** V3 §3 notice semantics are unchanged:
  notices cross, vetoes do not exist, mirrors decide for themselves.
  This note binds only our hosted registry's own operations, as an
  ops step under #1067.
- **Not a cross-curator mechanism.** A delist by the hosted registry
  leaves the world listed everywhere else it verifies; its invite
  links keep working everywhere (v4 §6 item 3). Nothing in the log
  asks, expects, or enables another curator to follow a delist.
- **Not adjudication.** The appeal is decided by the curator that
  delisted, under its own policy, with its decision logged. There is
  no appeals body, no escalation path, and no precedent effect: one
  appeal's acceptance creates no claim on any other delist.

---

**Verified when:** this note is merged with the `doc/README.md`
index line; the merged note is linked under #1067; the outcome
(note link + merge SHA) is posted on #16502; and the acceptance
elements are all present above — the delist-log shape (§1), the
public read path (§2), the published appeal channel (§3), and the
re-list-as-new-include procedure (§4), with no protocol changes (§5).
