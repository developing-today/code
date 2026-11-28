# Design — related path links in `hash.metadata` (#9114, S3.21–S3.22)

Status: design record (no code changed by this record). Issue:
[developing-today/code#9114](https://github.com/developing-today/code/issues/9114);
parent epic [developing-today/code#9107](https://github.com/developing-today/code/issues/9107);
program #9106. Statements: S3.21 "The metadata may contain related path
links" and S3.22 "Related links are path references, not restricted to
hash IDs" in `2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`.
Recorded 2026-11-17 by queue task 20307 (lane code-g).

## 1. What this settles

The sidecar design (A-9114.1–A-9114.3 in
[`2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`](2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md#9114--complete-metadata-sidecars-and-mutable-head))
already settles the sidecar envelope, the versioning form, and migration.
It does not settle the related-link rules that #9114's acceptance asks for:
link syntax, allowed target kinds, and dangling-link behaviour. This
record settles those. It does not reopen the versioning form.

## 2. Link record

A link is a typed entry in the sidecar **payload** (the metadata view
carried by `SidecarEnvelopeV1.raw_payload_bytes`, A-9114.1). It is never
part of the subject's content bytes, so adding or changing a link
changes a new snapshot and leaves the subject `ContentId` unchanged
(S3.11, S3.12).

```text
RelatedLink {
  relation: text        ; open vocabulary, see §4
  target: LinkTarget
}
LinkTarget = ContentRef(ContentId)
           | SnapshotRef(SnapshotId)
           | PathRef(text)          ; a path reference, §3
           | LocationRef(LocationRefV1)   ; typed locator, #9115
```

- Links are a list in the payload. Their order is canonical: sorted by
  `(relation, canonical target bytes)` before the payload is hashed, so
  the same set of links always yields the same snapshot ID.
- Unknown `relation` values are preserved verbatim as extras under the
  extras policy of #9095 (readers keep them; they never fail the read).

## 3. Path-reference syntax (S3.22)

A `PathRef` is a path, not an ID, so it can name things that have no
hash (a file in a namespace, a folder, a not-yet-hashed object).

- Encoded as UTF-8 text with `/` as the only separator. Stored in
  Unicode NFC; a non-NFC input is normalised at write, never rejected.
- **Relative to the namespace root** of the sidecar's `namespace_id`
  (A-9114.1), never to the location of the sidecar. A leading `/` is
  rejected. (Chosen default; Q2.)
- Segments may not be empty, `.`, or `..`. Any of these is a write-time
  error (`invalid_path_ref`). Nothing is resolved away, because silently
  normalising `..` could point at a different object.
- No percent-encoding inside the stored form. The stored text is the
  path; encoding for transport belongs to the URI layer (#9117).
- Byte length of the stored path is capped at 4096 bytes; over the cap
  is `resource_limit`, not truncation. (Chosen default; Q4.)
- A `PathRef` carries no authority. Resolving it is an authorisation
  decision made by the caller under its tenant context (#9083), per the
  summit's rule that no content location or URI implies authorization.

## 4. Allowed target kinds and relations

Allowed `LinkTarget` kinds are exactly the four in §2. Anything else
(an external URL, an opaque blob) is rejected at write with
`unsupported_target_kind`. External URLs are out of scope here. Adding
one is a new target kind, added additively under the shape registry
(#9113), not a path-link change. (Chosen default; Q1.)

Relations are an open vocabulary of lowercase ASCII identifiers
(`[a-z][a-z0-9_-]*`). No relation carries behaviour in this design; the
sidecar only records them. Interpretation belongs to whichever consumer
declares the relation. (Chosen default; Q5.)

## 5. Dangling-link behaviour

A link can outlive its target: the target object is deleted, moved, or
never replicated here.

- **Storing a dangling link is valid.** Writing a link does not check
  that its target exists. Requiring existence would make metadata
  unwritable before its targets arrive from a peer.
- **Resolving a dangling link returns `Dangling { target }`**, a distinct
  result, never an error and never a silent substitute. This is the same
  convention as the summit's `MissingObject` for shared content.
- **Readers preserve dangling links** when they re-write a sidecar. A
  writer that drops links it cannot resolve has lost metadata, which is
  forbidden by S3.14 (the sidecar holds ALL metadata).
- **Deleting a target does not cascade** to links that point at it.
  Deletion of a subject follows the summit's fenced deletion protocol
  (`part-a-graph`) and does not rewrite other sidecars.
- **Staleness is a read-time signal, not a write-time rule.** A resolver
  that sees `Dangling` may report it; it must not rewrite or prune the
  link. (Chosen default; Q3.)

## 6. Open questions, with chosen defaults

- **Q1 — external URLs as link targets.** Chosen: rejected in v1 (§4);
  added later as a new target kind. Rejected: accept any string, which
  would let an unverified reference masquerade as a path reference.
- **Q2 — relative to namespace root vs to the sidecar.** Chosen: the
  namespace root (§3). Rejected: relative to the sidecar, which moves
  when a sidecar is re-homed to another location.
- **Q3 — prune dangling links on read.** Chosen: never prune (§5).
  Rejected: prune, which loses metadata that a later peer may resolve.
- **Q4 — path length cap.** Chosen: 4096 bytes, `resource_limit` (§3).
  Rejected: unbounded, which is unsafe parser input.
- **Q5 — closed vs open relation vocabulary.** Chosen: open lowercase
  identifiers, preserved as extras (§4). Rejected: a closed list, which
  would need a version bump for each new relation.

## 7. Implementation split

Filed as one issue, linked from #9114 and #9107: typed `RelatedLink`
in the sidecar payload, canonical link ordering, `PathRef` validation
(§3), `Dangling` resolution (§5), and the `unsupported_target_kind` and
`invalid_path_ref` errors. Test vectors come from the examples in §3 and
§5. It builds on the envelope in #10242 and does not change the CAS head
in #10254.
