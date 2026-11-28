# Discovery v4 — curator-attestation annotation format design (#16500)

Design record for issue #16500 ("id(phase-14): Discovery v4 —
curator-attestation annotation format"). This is §9 item 2 of the v4
curation design (PR #16496, merged 2026-11-28T06:22:44Z, merge commit
`0d7612ff5d1c3f1dd4cecdcfedde73ac8f5a9491`; record
`doc/2026-11-28T06-10-00Z_design_id6290_open_discovery_v4/`, §4 and §9).
Parent outline #6290; core topic #6277. Sibling steps: #16499 (§9
item 1, subscription policy budgets, record
`doc/2026-11-28T07-59-00Z_design_id16499_subscription_policy_budgets/`)
and #16501 (§9 item 3, local reputation computation, record
`doc/2026-11-28T08-13-00Z_design_id16501_local_reputation/`).

Status: design only. No code in this issue — format design only. This
record names the attestation object's fields, its signing and expiry
rules, its storage/serving class, and its display rules beside
key-derived identifiers. Nothing here authorizes implementation
beyond a future separately scoped slice.

**Grounding — verified 2026-11-28 (lane 0222-L, firsthand from the v4
record and the issue).**

- The v4 design §4 fixes the semantics this record elaborates and
  does not re-decide: identity is the world key, full stop; display
  names are self-asserted, untrusted hints; there is no protocol name
  registry (v4 §4 rule 2, refused again in v4 §10); and "verified" is
  a curator-signed attestation — a signed object under the curator's
  key, stored and served curator-locally, in the same never-crosses
  class as policy and `seq` (v4 §4 rule 3). The v4 record decided the
  attestation's trust class and expressly deferred its bytes to this
  breakdown item (§9.2).
- A curator, in the v4 sense, is an operator with a signed list:
  lists are verified against the curator's key (`verify_from`, v3 §3
  rule 1, restated in v4 §3), and independent curators are counted by
  distinct signing keys (v4 §5). The attestation therefore signs with
  the curator's existing list-signing key and the list substrate's
  signature scheme — it introduces no new key, no new key store, and
  no new trust root.
- Issue #16500 carries no live claim and no prior discussion (0
  comments at claim time, 2026-11-28T08:27Z); its Done check requires
  exactly three elements — the attestation field table, the
  signing/expiry rules, and the display-rule statement — all present
  below (§2, §3, §5).

## 1. Governing rules (from v4 §4, restated as constraints)

A1. **Key identity only.** The attestation asserts a curator's
opinion about a (name, key) pairing. It never creates, reserves,
or transfers a name, and it is never protocol truth: the protocol
layer neither issues, verifies-for-display, nor adjudicates
attestations.
A2. **Curator-signed, curator-local.** An attestation is signed under
exactly one curator's list-signing key, stored and served only by
that curator, in the never-crosses class with policy and `seq`
(v4 §4 rule 3).
A3. **Never crosses mirrors as trust.** An attestation is never
embedded in a mirrored announcement, never propagated across a
mirror boundary, and never re-served by a mirror as its own. A
reader that does not subscribe to the signing curator never sees
the badge (§4, stated testably in §6).
A4. **Attributed display.** Wherever a badge derived from an
attestation is shown, it is attributed to the signing curator,
and the key-derived identifier for the subject key is shown
beside the name regardless of any badge (v4 §4 rule 1).
A5. **No adjudication.** Conflicting attestations from different
curators (two curators attesting different keys as canonical for
the same name) are both shown, each under its own curator's
attribution. Nothing ranks, merges, or resolves them (v4 §4
rule 4: confusables are curator/client UI policy, never protocol
adjudication).

## 2. The attestation object — field table

The attestation is a single signed annotation object, format id
`curator-attestation/1`. The signed payload is the canonical
serialization of every field except `signature` itself; any change
to a signed field invalidates the signature.

| #   | Field         | Type / shape                                                                                            | Rules                                                                                                                                                                                                           |
| --- | ------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F1  | `format`      | string, exactly `curator-attestation/1`                                                                 | Versioned as a whole: any field, encoding, or rule change is `/2`, never a silent reinterpretation of `/1` objects.                                                                                             |
| F2  | `curator_key` | the signing curator's public key (same encoding as list-signing keys)                                   | Must equal the key the signature verifies under (§3 S1). A mismatch is a verification failure, not a second signer.                                                                                             |
| F3  | `subject_key` | public key of the world being attested (K)                                                              | The key the badge will sit beside. Attestations about a name without a subject key do not exist in this format.                                                                                                 |
| F4  | `name`        | UTF-8 string, exactly as attested (N)                                                                   | Stored and compared byte-for-byte as issued. Normalization, folding, and confusable handling are display-side client policy (v4 §4 rule 4), never applied to the signed bytes.                                  |
| F5  | `statement`   | string, exactly `canonical-key-for-name`                                                                | The only statement type in `/1`. It asserts: "in this curator's judgement, key K is the canonical key presenting under name N." Other statement types are future format versions, not extensions of this field. |
| F6  | `issued_at`   | timestamp (seconds, UTC)                                                                                | Issuance instant claimed by the curator; sanity-checked against the reader's clock under §3 E2.                                                                                                                 |
| F7  | `expires_at`  | timestamp (seconds, UTC)                                                                                | Hard expiry (§3 E1). Required — an attestation without expiry is invalid, not perpetual.                                                                                                                        |
| F8  | `supersedes`  | optional: signature digest of a prior attestation by the same curator over the same (subject_key, name) | Renewal and correction chain (§3 E3/R2). Absent on a curator's first attestation for the pair.                                                                                                                  |
| F9  | `note`        | optional UTF-8 string, max 280 chars                                                                    | Curator's human-readable basis (e.g. "operated by the team since 2026"). Displayable, never machine-acted-upon; not part of the statement's meaning.                                                            |
| F10 | `signature`   | signature over the canonical serialization of F1–F9                                                     | Under `curator_key`, in the list substrate's signature scheme (§3 S1).                                                                                                                                          |

The field list is **closed**: an implementation may not add a field
to the signed payload without a new format version and a design
record amending this one. In particular, no field may carry a score,
a ranking, a mirror/relay provenance, or any traffic-derived value —
an attestation is a curator's signed statement, not a data feed
(cf. the traffic exclusion, v4 §5).

## 3. Signing and expiry rules

Signing:

S1. **One key, the list key.** The signature is produced with the
curator's existing list-signing key — the same key its signed
lists verify under (`verify_from`). Verification is: recompute
the canonical serialization of F1–F9, verify `signature` under
`curator_key`, and require F2 to equal that key. No countersignatures,
no delegated attestation keys, no third-party witnesses in `/1`.
S2. **Failure is absence.** An attestation that fails verification
for any reason (bad signature, F2 mismatch, malformed encoding,
unknown `format` version) is treated as not existing. It is never
displayed with a warning state — an unverifiable badge is worse
than no badge, because the badge's whole content is "this
verifies".
S3. **Self-asserted names stay self-asserted.** Signing an
attestation does not elevate the name N to protocol data. N
remains the subject's self-asserted display name; the signed
claim is only the curator's (K, N) pairing opinion (A1).

Expiry:

E1. **Hard expiry.** At and after `expires_at` the attestation is
void: clients must not render its badge, and curators must not
serve it as live. There is no grace period and no stale-while-
revalidate display.
E2. **Bounded lifetime.** `expires_at − issued_at` must be ≤ 180
days; the default issuance lifetime is 90 days. Objects claiming
a longer span are invalid whole (not clamped). A reader whose
clock places `issued_at` more than 24 h in the future treats the
object as not yet live.
E3. **Renewal is re-issuance.** A renewed attestation is a new object
with fresh `issued_at`/`expires_at`, linked to its predecessor
via `supersedes` (F8). At most one live attestation per curator
per (subject_key, name) pair: when a successor verifies, the
predecessor stops being served even before its own expiry.

Revocation:

R1. **Revocation is a tombstone, not an edit.** A curator revokes by
publishing, in its own annotation set, a tombstone-notice against
the attestation's signature digest (the v3 §3 notice shape:
issuer key, subject digest, reason, timestamp). The attestation
object itself is never mutated or silently deleted from the
curator's served history view.
R2. **Effect at next fetch.** A client drops the badge at the first
fetch that carries the tombstone — the same withdrawal semantics
reputation inputs already assume for tombstones (v4 §5).
Revocation by any other curator has no effect on this curator's
attestation (A5).

## 4. Storage and serving — curator-local data

- Attestations live in the signing curator's own annotation store,
  alongside its policy — never inside the signed registry list, never
  inside world announcements, and never in any object a mirror
  replicates (A3). The mirror rules are unchanged: mirrors carry
  announcements and lists exactly as before and carry no attestation
  bytes.
- The curator serves its live attestations only to readers that
  subscribe to that curator, through the same curator view that
  serves its lists and scores — there is no public attestation
  endpoint and no cross-curator attestation query (a central badge
  index would be the refused name registry by another route, v4 §4
  rule 2).
- A client holds attestations per curator, keyed by
  (curator_key, subject_key, name), and evaluates them only against
  its own subscription set: an attestation from an unsubscribed
  curator is not merely unbadged, it is not fetched, not stored, and
  not rendered in any form (A3).

## 5. Display rules (the display-rule statement)

D1. **Key first, always.** Wherever a client shows name N for a
world, it shows the key-derived identifier for K beside it —
badge or no badge (v4 §4 rule 1). A name without a key beside it
is a rendering bug, attested or not.
D2. **The badge is the curator's, not the name's.** A live, verified,
unexpired attestation renders as a badge attached to the
(name, key) pairing and visually/textually attributed to the
signing curator — the shape is "N ✓ per <curator>", never a bare
checkmark on N. An unattributed badge is a spec violation,
because it would read as protocol truth (A1/A4).
D3. **Non-subscribers see nothing.** A reader without a subscription
to the signing curator sees the name and the key-derived
identifier exactly as if no attestation existed — no badge, no
hint, no "verification available" teaser (A3).
D4. **Conflicts are shown, not settled.** If two subscribed curators
attest different keys as canonical for the same name, the client
shows each pairing with its own curator's badge. The client does
not pick a winner, does not order by badge, and does not warn in
protocol voice; confusable warnings remain client UI policy
(v4 §4 rule 4).
D5. **Expiry and revocation are silent.** When an attestation
expires or is revoked, the badge simply disappears at the next
fetch. No "formerly verified" state is rendered — a lingering
badge-ghost would keep asserting trust the curator no longer
asserts.

## 6. The mirror rule, as testable checks

**Rule R-MIRROR.** Attestations never cross mirrors as trust: no
mirror-replicated object contains an attestation or a badge claim,
and no reader ever sees a badge sourced from a curator it does not
subscribe to.

The implementation slice must carry these named tests (oracles
stated here, code in that slice):

T-ATT-1 (signature). A fixture attestation with one signed field
altered fails verification and renders no badge. Oracle: S1/S2.
T-ATT-2 (expiry). A fixture attestation scored one second before
`expires_at` renders its badge; at `expires_at` it renders none, with
no other input changed. Oracle: E1.
T-ATT-3 (lifetime bound). A fixture with
`expires_at − issued_at` = 181 days is invalid whole. Oracle: E2.
T-ATT-4 (mirror boundary). A mirrored announcement/list fixture
contains no attestation object, badge field, or signature digest from
§2. Oracle: R-MIRROR / A3.
T-ATT-5 (non-subscriber invisibility). A client fixture subscribed to
curator A but not curator B renders B's attestation for (K, N) in no
form, while rendering A's for the same pair. Oracle: D3.
T-ATT-6 (attribution). Every rendered badge fixture carries its
signing curator's identity adjacent to the badge. Oracle: D2.
T-ATT-7 (supersession). Given a live attestation and a verifying
successor naming it in `supersedes`, only the successor renders.
Oracle: E3.
T-ATT-8 (revocation). A tombstone-notice fixture against a live
attestation's digest removes its badge at the next fetch; a tombstone
from a different curator does not. Oracle: R1/R2.

## 7. Open choices — decided here with defaults

- **Lifetime:** 90-day default, 180-day hard maximum (E2). Long
  enough that renewal is routine rather than a chore that trains
  curators to demand perpetual badges; short enough that a
  compromised or abandoned curator key stops asserting pairings on
  its own.
- **Badge wording:** the statement type is fixed as
  `canonical-key-for-name` (F5) and the display attribution names the
  curator (D2). A generic "verified" label without the curator named
  is refused — it would manufacture the global namespace v4 §4
  rule 2 refuses.
- **Renewal chaining:** `supersedes` links by signature digest (F8),
  not by a curator-assigned id — the digest needs no registry and
  cannot collide across curators.

## 8. Out of scope (explicit)

- A protocol name registry or global namespace (refused in v4 §4
  rule 2 and §10, upheld here).
- Confusable-handling UI beyond the display rules in §5 (client and
  curator front-end policy, v4 §4 rule 4).
- Any code: the annotation store, the serving surface, signature
  verification wiring, and the client badge UI are future separately
  scoped slices; the tests in §6 are named here, written there.
- Cross-curator attestation exchange, attestation of curators
  themselves, and transitive trust ("a curator I trust trusts…") —
  each would rebuild a central index or a web-of-trust protocol v4
  does not have.
- Key recovery / identity handover (accepted v1 limitation,
  unchanged): an attestation cannot transfer a name to a new key;
  a curator whose subject rotates keys issues a fresh attestation
  for the new key, and the old one expires or is revoked.

---

**Verified when:** this record is merged with the `doc/README.md`
index line; the outcome (record link + merge SHA) is posted on
#16500; and the three Done-check elements are all present above —
the attestation field table (§2), the signing/expiry rules (§3),
and the display-rule statement (§5).
