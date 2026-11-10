# Program ecosystem (share, fork, gallery) — first-pass research + design for #6278

Status: first-pass design (research + design only; no implementation in
this record, per the #6278 charter).
Core topic: #6278. Filed from the next-horizons discussion #1240
(Drew, 2026-11-05: all themes are planned). Design ladder: v1 #6291,
v2 #6293, v4 #6295, v3 #6294. Foundations: F12.7 ABI (#1064, closed),
F12.8 guest SDKs (#1065, closed), F12.9 upgrades (#1066, closed),
F12.17 provenance (#1069, closed), F12.4 provider registry (#1061,
closed), F12.10 scheduling/quotas (#1067, open).

This record is the core topic's first-pass findings: it broadens the
topic beyond the issue's starting sketch, settles the vocabulary and
the load-bearing decisions the four ladder steps share, and names the
implementation next steps. Each ladder step (v1-v4) refines its own
slice; none of them may silently fork a decision recorded here — a
step that needs a different answer amends this record first.

## Principles applied (from #1240, restated as constraints)

- Local-first is always the goal, AND a hosted version is intended:
  central coordination supporting a large graph of broadly accessible
  objects. Neither half cancels the other.
- Anything built must be completely usable by others hosting a small
  or large server themselves. P2P-hosted worlds and data behave the
  same as a large hosted public server; differences live in backend
  scale elements and access controls, never in protocol behaviour.
- Users connect and transfer directly wherever possible; hosted
  resources are an offloadable backstop, not a requirement. The app
  must not technically need us to function.
- Our hosted instance is essentially public; other operators open
  theirs only as much as they want. Publishing paths: share to a big
  server, publish on pkarr, or self-host. All comms through iroh on
  the backend; first-class clients.

Consequence used throughout: every mechanism below has a file-based,
no-server form first (a bundle is a file; a template is a file; a
registry record resolves from pkarr), and the hosted form is the same
mechanism at larger scale.

## Where the pieces already stand

- Programs already have an ABI to target (F12.7, #1064), SDKs to
  build with (F12.8, #1065), an upgrade-with-migration-and-rollback
  story (F12.9, #1066), and a signing/provenance model (F12.17,
  #1069: module signatures against an admin trust list). The
  ecosystem does not re-decide any of these; it is the distribution
  layer on top of them.
- The provider registry (F12.4, #1061) is the nearest registry
  precedent: capabilities as manifests plus provider components. A
  gallery is the same shape pointed at programs instead of
  providers, which is why D6 keeps the two protocol families aligned.
- Nothing today defines: a program's portable identity, what a fork
  records, install/update authority in a world, template worlds, or
  a gallery protocol. Those are D1-D6 below.

## D1. Program identity

A program's identity is the tuple (author public key, name, version,
content hash):

- The author key is the identity root. Names are per-author: two
  authors may publish the same name; there is no global name
  registry and no first-come name ownership (see Alternatives).
- Version is the author's ordering claim (monotonic per author +
  name); the content hash is the fact. Two bundles with the same
  (author, name, version) and different hashes are a provenance
  violation, and clients must surface it as one, never silently pick.
- Identity survives transport unchanged: sideloaded, gallery-fetched,
  and template-carried copies of a program carry the same identity
  and verify against the same hash and signature.

## D2. The bundle is the unit of travel

One file carries a program anywhere (implemented by #8280):

- Payload (the program), a manifest stating the D1 identity, the
  F12.7 ABI descriptor, the F12.17 signature over manifest + payload,
  and fork lineage (parent identity + parent content hash, empty for
  originals).
- Sideloading a bundle file must always work, fully offline, with
  verification (signature, hash, ABI fit) at import. A gallery is a
  convenience for finding bundles, never a gate on installing them.
- A fork is a new bundle: new content hash, its own version line,
  lineage naming the parent it forked from. Attribution is therefore
  structural — it lives in every downstream bundle's lineage — not a
  courtesy field a fork can delete without breaking its own
  provenance record.

## D3. Forking and attribution

- Anyone may fork any bundle they hold; forking needs no permission
  from the original author and no registry. (Licence terms are the
  author's to state in the manifest; the protocol records them, it
  does not adjudicate them.)
- A fork keeps the parent's author key in lineage forever; a fork of
  a fork records the full chain, so attribution accumulates rather
  than flattening to the last forker.
- Authorship of new work in a fork belongs to the forking author's
  key from the fork point on. UIs present the lineage chain; they
  never present a fork as the original author's work.

## D4. Install semantics and update authority

- Install is a world act: the world owner (or a role the owner
  delegates) installs a bundle into a world, and the install record
  names the D1 identity, the hash installed, and the install policy.
- Policy is per install: pinned (exactly this hash; updates are new
  install decisions) or tracking a channel (the author's version line
  for this name). Default is pinned — an update is an authority
  decision, not ambient drift.
- What an update may change without fresh world-owner approval is
  bounded by F12.7/F12.9: an in-channel update that keeps the ABI
  descriptor and migrates state under the F12.9 rules may apply
  under a tracking policy the owner chose; an update that widens
  the ABI surface, changes required capabilities, or fails migration
  always needs explicit owner approval, and rollback per F12.9 is
  part of the same act.
- Quotas and scheduling for installed programs are F12.10's (#1067);
  an install that would exceed them fails at install time with the
  quota named, not at first run.

## D5. Template worlds

A template is a world snapshot as a starting point (implemented by
#8282):

- Contents: world state (the template-scoped part), installed
  programs as bundle references (identity + hash), and role
  definitions.
- Stripped by default, and the stripping is recorded in the template
  manifest: member lists, credentials and secrets, per-user data,
  and grants. Anything retained is named in the manifest. A template
  never carries another user's identity material — this is an
  invariant, not a preference.
- Instantiating a template mints a fresh world identity; roles are
  recreated empty; programs install through the ordinary D4 path
  (hash-verified from their bundles), so a template cannot smuggle
  in a program that would fail normal install verification.

## D6. The gallery is a registry, and registries are plural

- A gallery is a registry of bundle records (D1 identity, version,
  hash, signature, lineage, manifest) — the same protocol family as
  world discovery (T2), a different object type, transported over
  iroh (implemented by #8281).
- Publishing paths are the three from #1240: share to a big server,
  publish on pkarr, or self-host. All three carry the same record;
  a client reads any of them identically. Our hosted gallery is the
  public instance of the protocol, not a different protocol.
- Admission is operator policy; validity is protocol. A registry
  must reject records whose signatures or hashes do not verify
  (protocol). Which verified programs it lists (curation,
  allow-lists, openness) is the operator's choice, and our public
  instance's near-total openness is our choice, not a protocol rule
  other operators inherit.
- Discovery metadata (descriptions, screenshots, categories) is
  manifest data signed with the record; a registry may index it but
  may not alter it — altered metadata is a different, unsigned
  record and clients treat registry-side annotations as the
  registry's speech, visually distinct from the author's.

## Invariants implementation must hold

1. Sideloading always works: no install path requires a registry, a
   hosted server, or us.
2. Identity is (author key, name, version, content hash); a hash
   mismatch or a same-version/different-hash pair is surfaced as a
   provenance violation, never resolved silently.
3. Fork lineage is structural and cumulative; attribution cannot be
   removed without breaking the bundle's provenance.
4. Default install policy is pinned; updates beyond the F12.7/F12.9
   no-approval envelope require explicit world-owner approval.
5. Templates strip members, credentials, per-user data, and grants
   by default, and the manifest records what was stripped and what
   was retained.
6. Registry validity (signature/hash verification) is protocol;
   admission and curation are operator policy. No registry is
   required, and ours is not special in the protocol.
7. P2P and hosted behaviour are identical at the protocol level;
   scale differences live in backend elements and access controls.

## Alternatives rejected

- **A central app-store as the distribution mechanism.** Forks the
  #1240 principles outright: it makes the hosted instance a
  requirement, gives one operator admission power over everyone's
  programs, and has no self-hosted or pkarr form.
- **Global name registry / name ownership.** Names collide honestly
  across authors in every real ecosystem; pretending otherwise buys
  a land-rush and a dispute process. Author-key-scoped names plus
  content hashes are unambiguous without one.
- **Floating installs by default (always-latest).** Ambient updates
  break worlds silently and contradict the owner's authority over
  their world; tracking is available, but it is a chosen policy.
- **Attribution as a display courtesy (credits field only).**
  Removable metadata gets removed. Lineage in the signed bundle
  makes attribution part of provenance instead.
- **Templates as full world clones (members and data included).**
  A clone copies other people's identity material and private data
  into a new owner's hands; the strip-by-default manifest is the
  entire point of a template.

## The ladder, mapped

- **v1 — first-iteration design (#6291):** share/fork basics against
  D1-D5 — signed bundle export, import into a world, fork with
  attribution, template-world export/import, all local-first. Its
  entry condition (this record's first-pass findings) is met by this
  document. Implementation: #8280 (bundle + sideload), #8282
  (template worlds).
- **v2 — outline: gallery and updates (#6293):** the D6 registry in
  depth plus D4 channel/update policy UX. Implementation seed:
  #8281 (registry protocol + reference registry).
- **v3 — outline: public gallery (#6294):** our hosted instance as
  the public backstop — curation posture, abuse handling, and scale
  elements, all as operator policy layered on D6, never protocol.
- **v4 — outline: ecosystem at scale (#6295):** federation of
  registries, lineage-aware discovery (forks of forks), and the
  large-graph coordination the #1240 principles anticipate.

## Next steps filed from this record

- #8280 — signed program bundle format + sideload import (D1, D2).
- #8281 — gallery registry protocol + reference registry service
  (D6).
- #8282 — template-world snapshot export/import (D5).
- The v1-v4 design steps proceed as #6291, #6293, #6294, #6295;
  anything the research surfaces that fits none of them gets its own
  issue, per the core topic's standing rule.
