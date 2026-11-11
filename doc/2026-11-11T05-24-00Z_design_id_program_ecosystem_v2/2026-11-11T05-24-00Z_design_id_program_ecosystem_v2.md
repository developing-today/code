# Program ecosystem — v2 design: gallery & updates for #6293

Status: v2 design (design only; no implementation code in this record,
per the #6293 charter).
Ladder step: v2 of the Phase 14 Program ecosystem core topic #6278
(first-pass design of record:
`doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`; v1 design:
`doc/2026-11-11T05-01-15Z_design_id_program_ecosystem_v1/`).
Entry condition: met — #6291 is closed with its v1 design landed by
PR #8541 (merge 44fd6a414c), refining core decisions D1–D5, with the
gallery/registry (D6) and channel/update-policy work explicitly
scoped to this step. This record refines D6 and D4 for the v2 slice
only; it does not re-decide them, and it does not re-decide anything
the v1 record settled. A conflict with the core or v1 record is
resolved by amending that record first, never by silently diverging
here.
Principles: #1240 / core record — local-first always, hostable by
anyone at any scale, p2p behaviour identical to hosted behaviour, our
hosted instance is a public backstop and never a requirement.

## What v2 is, and is not

In scope:

1. The gallery as a registry, in depth (refines D6): the registry
   record, the publish / unpublish / list / fetch operations, the
   three publishing paths, and the validity-vs-admission line.
2. Template packages as registry records: publishing a template
   through a gallery (the v1 record §5 deferred exactly this to
   v2+) reuses the registry object family.
3. Update channels and the update flow into worlds that installed
   earlier versions (refines D4): the channel machinery behind the
   install record's policy field, update discovery, the approval
   envelope, changelog presentation, and rollback via F12.9.
4. The gallery/update client behaviour that ties 1–3 together: one
   client reads every registry identically, and a fetched bundle
   installs through the v1 sideload path.

Out of scope for v2: our hosted public gallery as an operated
instance — curation posture, abuse handling, scale elements (v3,
#6294; all operator policy layered on D6, never protocol);
federation and lineage-aware discovery at scale (v4, #6295);
ranking, payments, and public-gallery moderation UX (named out of
scope by #8281 and belonging to v3 policy, not v2 protocol); any
change to the v1 bundle, import, fork, or template mechanisms —
v2 builds on them unchanged.

## 1. The registry record (refines D6)

- A registry holds records, not programs-in-the-abstract. A bundle
  record is: the D1 identity (author key, name, version, content
  hash), the bundle manifest as defined by the v1 record §1
  (signature, lineage, ABI descriptor, optional licence), and the
  discovery metadata below. A template record is the same shape
  with the template manifest (v1 record §5) in place of the bundle
  manifest: one protocol, two object types, distinguished by an
  object-type field in the record — never two protocols.
- Discovery metadata (descriptions, screenshots, categories) is
  manifest data: it travels signed with the record, exactly as the
  core record's D6 states. A registry may index it; it may not
  alter it.
- Validity is protocol, at ingest and at read: a record whose
  content hash or F12.17 signature does not verify is not a record
  a registry may serve, and not one a client may act on — the
  refusal names the failing check (the v1 §6 taxonomy applies:
  `hash_mismatch`, `signature_invalid`). The provenance rule is
  unchanged: two records claiming the same (author, name, version)
  with different hashes are a provenance violation, surfaced as
  one, never silently resolved — by registries and clients alike.
- Admission is operator policy: which verified records a registry
  lists is its operator's choice (openness, allow-lists, curation),
  and no other operator — including us — inherits it. Validity is
  the only universal gate.

## 2. Publishing: three paths, one record (refines D6)

- The three paths from #1240 carry the identical record: publish
  to a hosted registry, publish on pkarr, self-host a registry.
  A client reads any of them identically; the hosted instance is
  the public backstop, not a different protocol.
- Publishing is author speech: a publish operation is signed by
  the author key in the record's identity. A registry accepts a
  publish only from the key that owns the line — anyone may
  re-host a record (re-hosting changes nothing in the record, as
  with v1 re-export), but only the author can originate or update
  the author's own listing.
- Unpublishing splits along the speech line: an author's
  withdrawal is a signed tombstone for the record (author speech,
  honoured by every conforming registry); an operator's delisting
  is operator speech — the record leaves that registry's listing,
  the tombstone does not exist, and other registries are
  unaffected. Clients and surfaces never present one as the other.

## 3. Registry speech vs author speech

- Anything a registry adds — annotations, warnings, counts,
  ordering — is the registry's speech, unsigned by the author, and
  every surface presents it visually distinct from the author's
  signed data (core D6). Registry speech never becomes manifest
  data by being indexed, cached, or copied to another registry.
- Presentation rule, extending the v1 §4 rule to listings: a fork
  is listed with its lineage chain and is never presented as the
  original author's work. A registry that strips lineage from a
  listing is serving an invalid record (§1), not exercising
  editorial discretion.

## 4. Update channels (refines D4)

- A channel is one author's version line for one program name:
  the (author key, name) pair of the D1 identity, ordered by the
  author's version claim and verified by content hash. There is
  no channel object separate from the line itself — a channel is
  what a sequence of same-line bundles already is.
- The install record's policy field (v1 record §3, step 5) becomes
  live in v2, with exactly the core D4 semantics: `pinned` —
  exactly this hash; updates are new install decisions — is the
  default, and `tracking` — the owner has opted this install into
  update discovery for this line. No third policy exists in v2,
  and no policy is ever set by anyone but the world owner (or a
  role the owner delegates), per install.
- A fork is a different line. Lineage is history, not a channel
  (v1 record §4): a fork's installs never track the parent's line,
  a parent's installs never track a fork, and presenting a fork's
  newer version as an "update" to the parent's install is a
  category error surfaces must not make — it is a new program the
  owner may install, through the ordinary path.
- Update discovery is pull-based, at world-owner action, for both
  policies: a check consults the owner's chosen sources (any
  registry path from §2, or local files) and reports candidates.
  Pinned installs are reported on — never modified — by a check.
  No background auto-fetch exists in v2 for any policy; ambient
  drift is what the pinned default exists to prevent (core record,
  Alternatives).

## 5. The update flow (refines D4)

An update candidate is a newer bundle for the installed line,
arriving by any path — registry fetch, pkarr, self-hosted
registry, or a local file. Path does not change the flow:

1. Verify first, policy second: the candidate runs the v1 import
   pipeline (v1 record §3, steps 1–4) — content hash, F12.17
   signature, F12.7 ABI fit, provenance check. A candidate that
   fails verification is refused with the failing check named;
   policy is never consulted for an unverified bundle.
2. Envelope decision: under a `tracking` policy, the update may
   apply without fresh approval only inside the core D4 envelope —
   it keeps the ABI descriptor, widens no required capabilities,
   and migrates state under the F12.9 (#1066) rules. Anything
   outside the envelope — ABI change, capability widening,
   migration failure — requires explicit world-owner approval,
   and a `pinned` install requires that approval for every
   update, envelope or not.
3. The approval presentation is one fixed summary: from→to D1
   identities, the author's changelog (§6), the ABI/capability
   delta (or "none"), and the migration outcome (or the named
   failure). Nothing applies on a summary the owner has not seen.
4. Apply is the F12.9 act: migrate under F12.9, re-check quotas at
   apply time (F12.10, #1067 — an update that would exceed quotas
   fails with the quota named), write the new install record
   (new D1 identity, policy preserved). Rollback per F12.9 is
   part of the same act: a failed apply leaves the prior install
   running untouched, and a rolled-back install returns to the
   exact prior version and install record.

## 6. Changelogs

- A changelog is author speech about a version, carried as signed
  discovery metadata with that version's record (§1) — the same
  rule as every other discovery field: signed with the record,
  indexable, never alterable by a registry.
- Every update presentation (§5 step 3) shows the target
  version's changelog. Its absence is shown as what it is — "no
  changelog provided by the author" — and is never filled in from
  registry annotations, diffs, or generated text.
- Rollback does not rewrite history: after a rollback, the
  install record names the restored version, and the rolled-back-
  from version's record — and its changelog — remains available
  to the owner. An update history per install (versions applied,
  approvals given, rollbacks) is part of the install record's
  world-side data, not registry data.

## 7. The client ties it together

- One client behaviour for every source: list and fetch from a
  hosted registry, a pkarr publication, or a self-hosted registry
  are the same operations with the same record (§2); a fetched
  bundle is a bundle file, and installing it is the v1 sideload
  import, step for step. Fetch-then-install adds no verification
  shortcut and removes none.
- A registry is a convenience for finding bundles, never a gate
  on installing them (core invariant 1): the same bundle arriving
  as a local file installs identically with no registry consulted,
  and no v2 surface may require an account, a specific registry,
  or our hosted instance for any operation in this record.

## 8. Error and refusal taxonomy (v2 additions)

The v1 record §6 taxonomy stands; v2 surfaces additionally name:

- `registry_record_invalid` — a served record fails §1 validity
  (the failing check is named alongside, from the v1 taxonomy).
- `update_wrong_line` — a bundle offered as an update whose
  (author key, name) differs from the installed line; it is not
  an update and is refused as one (it may still be installed as
  a new program, by the ordinary path, on a fresh decision).
- `update_approval_required` — an update outside the §5 envelope,
  or any update to a `pinned` install, presented for approval;
  this is a state, not a failure, and it carries the §5 step 3
  summary.
- `update_quota_exceeded` — apply would exceed F12.10 quotas
  (quota named), per §5 step 4.
- Registry unreachability is data, not a refusal class: a check
  reports the sources it could not reach and proceeds with the
  ones it could; sideload and the remaining paths are unaffected.

Refusals are data, not panics — the v1 rule stands.

## 9. Invariants v2 implementation must hold (test oracles)

Each invariant names its oracle, per the fleet testing doctrine:

1. Three-path identity: the same record published via a hosted
   registry, pkarr, and a self-hosted registry is read by one
   client as the identical record. Oracle: field-by-field
   equality of the three reads, signature verification included.
2. Validity is not admission: a registry configured with a
   restrictive admission policy still refuses invalid records,
   and an open registry still refuses them. Oracle: an invalid
   record is unservable under both policy fixtures.
3. Pinned means pinned: update checks against a pinned install
   never modify it, across any number of checks and candidates.
   Oracle: install record and installed hash unchanged after
   checks offering in- and out-of-envelope candidates.
4. Envelope: an in-envelope update applies under `tracking`
   without an approval act; each out-of-envelope variant (ABI
   change, capability widening, migration failure) does not.
   Oracle: per-variant apply/refuse outcomes against fixtures.
5. Rollback exactness: a rolled-back install is the prior
   install — version, hash, and install record. Oracle:
   field-by-field equality against the pre-update record.
6. Lineage in listings: a fork's registry listing carries its
   full lineage chain. Oracle: chain equality against the
   bundle's signed lineage, per §3.
7. Sideload equivalence: a bundle fetched from any registry and
   the same bundle as a local file produce identical installs.
   Oracle: install-record equality across the two paths, with
   networking disabled for the file path.

## 10. Open questions, explicitly deferred

- Template records (§1): one protocol with an object-type field
  is this record's decision; whether the reference registry
  stores templates in the same index as bundles is an
  implementation choice deferred to #8281's build — it does not
  change the record or the client behaviour.
- Push notification of new versions (a registry telling worlds
  an update exists) is deferred: v2 discovery is pull-only (§4).
  Push, if it ever exists, is an operated-instance concern (v3,
  #6294), never a protocol requirement — a world that never
  hears a push must lose nothing.
- Multi-registry disagreement about annotations needs no
  resolution rule: annotations are per-registry speech (§3) and
  are never merged. Disagreement about the record itself is a
  §1 provenance violation and already has its rule.
- Whether a tracking install may track a line through a change
  of the owner's chosen registry (line identity is the author
  key, so the line survives the move by construction) is noted
  as expected-free behaviour, to be confirmed by an #8546 test —
  not a design decision this record needs to take.

## 11. Implementation breakdown for v2

Filed and linked from #6293 and #6278:

- #8281 — gallery registry protocol + reference registry service
  (D6; §§1–3, §7 of this record). Already filed from the core
  record; this record is its design input for the record format,
  the tombstone/delisting split, and template records.
- #8546 — program update channels + update flow into worlds
  (D4; §§4–6), filed from this record: the channel machinery
  behind the install record's policy field, pull-based update
  discovery, the §5 envelope flow, changelog presentation, and
  F12.9 rollback as part of the apply act. (No earlier issue
  owned this: #8281 is the registry, and the v1 record explicitly
  deferred channel machinery to this step.)

Dependencies the breakdown rides on: #8280 (bundle format +
sideload import — the §5 verification pipeline and the install
record), #8540 (fork operation — the line boundary in §4), #8282
(template packages — the §1 template record's manifest), and the
landed F12.7 (#1064), F12.9 (#1066), F12.10 (#1067), and F12.17
(#1069) specs.

Explicitly NOT v2 breakdown items: our hosted public gallery
instance (v3, #6294); federation and lineage-aware discovery at
scale (v4, #6295).

## Acceptance mapping for #6293

- Entry condition met: #6291 closed, v1 design landed (PR #8541),
  its v2-scoped items (D6 gallery, channel/update-policy UX)
  named there and taken up here. ✓
- Outline updated in place on #6293: tightened gallery/update
  scope (§§1–7), open questions (§10), dependencies (§11). ✓
- Next-step breakdown filed and linked from #6293 and #6278:
  §11 — #8281 (already filed) and #8546 (filed with this
  record). ✓
- Phase 14 principles (#1240) honoured: local-first, hostable by
  anyone, p2p identical to hosted — §2's three identical paths,
  §7's no-gate client, and invariant 7. ✓
