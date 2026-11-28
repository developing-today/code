# Program ecosystem — v1 design: share/fork basics for #6291

Status: v1 design (design only; no implementation code in this record,
per the #6291 charter).
Ladder step: v1 of the Phase 14 Program ecosystem core topic #6278
(first-pass design of record:
`doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`).
Entry condition: met — #6278 is closed with its first-pass findings
landed by PR #8288 (decisions D1–D6, invariants 1–7). This record
refines D1–D5 for the v1 slice only; it does not re-decide them. A
conflict with the core record is resolved by amending the core record
first, never by silently diverging here.
Principles: #1240 / core record — local-first always, hostable by
anyone at any scale, p2p behaviour identical to hosted behaviour, our
hosted instance is a public backstop and never a requirement.

## What v1 is, and is not

In scope (the remaining v1 slice, after F12.2):

1. Export a program as one signed bundle file.
2. Import a bundle into a world (sideload; fully offline).
3. Fork a held bundle with structural attribution.
4. Export a world as a template-world package file and import it as a
   new world.

Out of scope for v1: the gallery/registry (D6 — that is v2, #6293,
seeded by #8281), channel/update-policy UX (v2), our hosted public
gallery (v3, #6294), federation and lineage-aware discovery at scale
(v4, #6295), and any change to the F12.2 world fork/template/
provenance implementation (`pkgs/id/src/world_store.rs`:
`fork_world`/`ForkSpec`, `ModuleDir`) — v1 builds beside it, reusing
its module store and world identity machinery, and does not redo it.

## 1. The bundle file (refines D1, D2)

One file is the unit of travel. Format decisions for v1:

- Container: a single archive file with a fixed manifest entry at a
  known path and the payload beside it — no external references, no
  network fetches at any point in its life. Extension and exact
  archive encoding are #8280's implementation choice; the manifest
  schema below is the contract.
- Manifest fields (all required unless noted):
  - `identity`: author public key, program name, version (author's
    monotonic ordering claim per D1), content hash of the payload.
  - `abi`: the F12.7 ABI descriptor the payload targets (#1064).
  - `signature`: the F12.17 signature over manifest + payload (#1069),
    verifiable against the author key in `identity`.
  - `lineage`: ordered list of ancestor identities + content hashes,
    empty for an original; the last entry is the direct parent.
  - `licence` (optional): the author's stated licence text or
    identifier. The protocol records it; it never adjudicates it
    (core D3).
  - `provenance_notes` (optional): free-form author notes; unsigned
    annotations by anyone else are never manifest data (core D6's
    author/registry speech distinction applies to bundles too).
- Identity is exactly the D1 tuple (author key, name, version,
  content hash). Two bundles claiming the same (author, name,
  version) with different content hashes are a provenance violation:
  every v1 surface that encounters the pair must surface it as one
  and refuse to silently pick (core invariant 2).

## 2. Export (authoring side)

- Export takes an installed or authored program in a world (or an
  authoring checkout) and produces one bundle file: payload copied
  from the module store by hash (`ModuleDir::load`), manifest built
  from the install record, signed with the author's key.
- Export is a pure file operation: no server, no registry, no world
  host round-trip beyond reading the local module.
- Exporting a program the exporter did not author is allowed and is
  just re-export: the bundle is bit-identical in identity, hash, and
  signature. Re-export is not a fork and creates no lineage.

## 3. Import into a world (refines D2, D4)

Import is a world act performed by the world owner (or a role the
owner delegates), in this exact order — a failure at any step refuses
the import and names the failing check:

1. Verify the content hash against the payload.
2. Verify the F12.17 signature over manifest + payload against the
   author key.
3. Check the F12.7 ABI descriptor against the world's host ABI.
4. Provenance check: if the world already holds a bundle with the
   same (author, name, version) and a different hash, refuse and
   surface the provenance violation (both hashes named).
5. Install through the ordinary module path (payload into
   `ModuleDir` by hash) and write the install record: D1 identity,
   hash installed, lineage as carried, and install policy —
   **pinned by default** (core D4). Tracking a channel is an explicit
   owner choice recorded in the same record; v1 records the policy
   field but builds no channel machinery (that is v2).
6. Quota/scheduling fit is F12.10's (#1067): an install that would
   exceed quotas fails at step 5 with the quota named (core D4).

Import works fully offline from a local file (sideloading always
works — core invariant 1). There is no step at which a registry, a
hosted server, or our infrastructure is consulted.

## 4. Fork with attribution (refines D3)

- Anyone holding a bundle may fork it; no permission from the
  original author and no registry is involved.
- The fork operation produces a NEW bundle: the forking author's key
  becomes the author of the fork's new version line, the fork's
  lineage is the parent's lineage plus the parent's full identity
  (author key, name, version, content hash), and the fork is signed
  with the forking author's key. Attribution is therefore structural
  and cumulative (core invariant 3): removing it breaks the fork's
  own provenance chain and verification.
- A fork starts pinned-only: lineage is history, not an update
  channel. Nothing in a fork subscribes it to the parent's versions
  or vice versa.
- Presentation rule for every v1 surface (CLI first): a fork is shown
  with its lineage chain ("forked from <author>/<name>@<version>,
  itself forked from …"); a fork is never presented as the original
  author's work (core D3).

## 5. Template-world package (refines D5)

- A template package is one file, structurally a sibling of the
  bundle: a template manifest plus the template-scoped world state.
  Its manifest lists: template identity (exporting author key, name,
  version, content hash — the D1 shape applied to templates), the
  installed programs as bundle REFERENCES (identity + hash, never
  silently re-hosted binaries the exporter does not hold), the role
  definitions, and the strip record below.
- Strip-by-default, recorded in the manifest (core invariant 5):
  member lists, credentials and secrets, per-user data, and grants
  are stripped; anything retained is named in the manifest. A
  template never carries another user's identity material — this is
  an invariant, not a preference.
- Import instantiates a NEW world: fresh world identity (the F12.2
  world machinery mints it), roles recreated empty, and each program
  reference resolved to a bundle the importer holds and installed
  through the §3 import path — hash-verified, so a template cannot
  smuggle in a program that would fail normal install verification
  (core D5). A reference the importer cannot resolve is a named,
  resumable gap (the world is created with the gap recorded), never
  a silent skip and never a network fetch the user did not ask for.
- Export and import are file operations; no server is required.
  Publishing a template through a gallery is v2+ and reuses the
  registry object family, not a v1 mechanism.

## 6. Error and refusal taxonomy (v1 surfaces name these)

- `hash_mismatch` — payload does not match the manifest content hash.
- `signature_invalid` — signature does not verify against the author
  key.
- `abi_incompatible` — ABI descriptor does not fit the world host ABI
  (descriptor and host ABI both named).
- `provenance_violation` — same (author, name, version), different
  hash (both hashes named).
- `quota_exceeded` — install would exceed F12.10 quotas (quota
  named).
- `template_reference_unresolved` — template names a bundle the
  importer does not hold (identity + hash named).

Refusals are data, not panics: each carries the failing check and the
values above so a CLI, a log, or a future UI can present them
identically.

## 7. Invariants v1 implementation must hold (test oracles)

Each invariant names its oracle — a check an implementation issue's
tests must implement, per the fleet testing doctrine:

1. Round-trip: export → import into a fresh world yields a runnable
   install whose recorded identity, hash, signature, and lineage
   match the bundle. Oracle: field-by-field equality of the install
   record against the manifest.
2. Tamper refusal: flipping any payload or manifest byte makes
   import refuse with the failing check named. Oracle: the refusal
   class (§6) for each tamper site.
3. Offline: import and template instantiation complete with no
   network access. Oracle: the operation run with networking
   disabled succeeds identically.
4. Fork lineage: a fork of a fork carries the full ordered ancestor
   chain; editing lineage invalidates the fork's signature.
   Oracle: chain equality against the parents' identities, plus
   signature verification failure on a lineage-mutated bundle.
5. Template strip: a fixture world containing members, grants,
   credentials, and per-user data exports a template containing none
   of them, whose manifest strip record names each stripped class.
   Oracle: absence checks over the package plus manifest equality
   against the fixture's strip classes.
6. Provenance violation: two bundles with the same (author, name,
   version) and different hashes are surfaced as a violation, never
   silently resolved. Oracle: `provenance_violation` refusal naming
   both hashes.

## 8. Implementation breakdown for v1

Filed and linked from #6291 and #6278:

- #8280 — signed program bundle format + sideload import (D1, D2;
  §§1–3 of this record). Already filed from the core record.
- #8540 — program fork operation with structural attribution
  (D3; §4), filed from this record: the fork command/operation,
  lineage append, re-signing, and the lineage presentation rule.
  (The core record filed no dedicated issue for the fork operation
  itself; #8280 carries the lineage FIELD, not the operation.)
- #8282 — template-world snapshot export/import (D5; §5). Already
  filed from the core record.

Explicitly NOT v1 breakdown items: #8281 (gallery registry protocol

- reference registry) belongs to v2 (#6293) per the core record's
  ladder mapping; channel/update-policy work is v2; hosted public
  gallery is v3 (#6294); federation is v4 (#6295).

## Acceptance mapping for #6291

- Entry condition met: #6278 closed, first-pass findings landed
  (PR #8288, core design record). ✓
- Design document covering signed bundle export, import into a
  world, fork with attribution, template-world export/import, all
  local-first with no server required: this record, §§1–5. ✓
- Implementation breakdown filed and linked from #6291 and #6278:
  §8 — #8280, #8540 (filed with this record), and #8282. ✓
- Phase 14 principles (#1240) honoured: local-first, hostable by
  anyone, p2p identical to hosted — §3's no-server import, §5's
  file-only template flow, and the core record's principles applied
  throughout. ✓
