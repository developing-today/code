# Design: Joplin importer for the ID knowledge layer (#5848)

- Date: 2026-11-08
- Status: **decision-ready importer profile** — design only; no importer is implemented here.
- Issue: [#5848](https://github.com/developing-today/code/issues/5848), under programme [#5678](https://github.com/developing-today/code/issues/5678) and epic [#2056](https://github.com/developing-today/code/issues/2056).
- Target model: [#2048](2026-11-01_design_id2048.md), landed by PR #3885.
- Related seams: [#5839](https://github.com/developing-today/code/issues/5839) (content model), [#5844](https://github.com/developing-today/code/issues/5844) (attachments and media), [#6439](https://github.com/developing-today/code/issues/6439) (interchange and full-fidelity export).
- Scope: Joplin export → ID knowledge objects and the Joplin-specific round trip. This does not decide the common export format for all tools or when to implement the importer.

## Decision

Build the first Joplin adapter around a **JEX archive**, with a Joplin Markdown + Front Matter directory as a documented lower-fidelity fallback. Import each Joplin note as one stable ID note with an initial immutable revision; keep note identity separate from title, preserve tags as tags, and convert resolvable in-archive note links to stable-ID typed references. Import resources as content-addressed blobs and preserve their authored filenames as display metadata. Never match a link target by title when its Joplin ID is missing or ambiguous.

JEX is the preferred source because Joplin describes it as a lossless archive of notes, notebooks, tags, resources and metadata; its implementation extracts JEX and delegates to the raw importer, which remaps item IDs and rewrites linked IDs. The remapping is expected behavior, not a defect: Joplin's current import is “as new notes,” not merge/replace. The adapter must therefore maintain a source-to-ID mapping in its import manifest rather than treating a Joplin ID as an ID-platform identity. Since archives do not identify a profile namespace for this purpose, the user reuses a namespace label across exports from the same profile; the archive digest records content snapshots.

Two mappings are explicit compromises against #2048's current constructs:

1. **Notebook hierarchy:** #2048 defines notes, claims/tags, aliases and references, but no notebook/container object. Import each note's full notebook breadcrumb as a namespaced generic tag (`joplin.notebook/<encoded-path>`) when the encoded tag fits the target model's tag-name limit (`MAX_NAME_BYTES = 128` in `pkgs/id/src/tag_claims.rs`; the 16-byte prefix leaves 112 bytes for the encoded path). When it does not fit, the tag is a bounded key instead: `joplin.notebook/<encoded path truncated at a segment boundary to at most 79 bytes>~<first 32 hex characters of the SHA-256 of the full encoded path>` (128 bytes total), and the full exact breadcrumb is carried in the import manifest and the note's source metadata — the tag is never the sole carrier of the path, and export reconstructs the breadcrumb from that full-path copy, not from the truncated tag. This keeps path identity and grouping searchable without inventing a container object. It does not preserve empty notebooks or native parent/child notebook operations; report those losses, and report every breadcrumb that had to use the bounded form. Revisit only through #5839, not by extending this importer.
2. **Binary attachments:** #2048 supplies content blobs but does not define a typed note-to-resource edge. Keep each attachment's bytes as a separate content-addressed blob and refer to it from the note revision through the shared attachment-reference/manifest shape settled by #5844. Do not encode bytes into Markdown or misclassify attachments as topic links. The adapter's attachment round trip is gated on that shared reference contract.

Imported objects follow #5678's already-ruled read-authority order (object → user default → server config → public); this design does not invent a Joplin-to-ID ACL translation. The fidelity report must call out that Joplin sharing/ownership settings are not carried over. Any narrower read grant is an explicit user choice under the existing policy.

## Source formats and fidelity

### Preferred: JEX

Use a user-selected `.jex` archive. Joplin's official import/export guide describes JEX as a tar archive that preserves notes and metadata such as location, updated time and tags; the JEX importer extracts it and calls the raw-directory importer. The export service enumerates folders, notes, linked resources, note-tag relations and tags. The raw importer constructs an old-ID → new-ID map and rewrites note/resource IDs in note bodies. These behaviors give the adapter a coherent archive-wide graph to translate.

Treat JEX as an **additive import**. It does not replace or merge an existing ID object. On first import, create new target note IDs and record `source namespace + Joplin item ID → target object ID` in a versioned local import manifest; the archive digest identifies each snapshot, not the source identity. The first import creates a namespace label, and later exports from that same Joplin profile must reuse it. A different profile gets a different namespace. Re-import of the same archive into the same store reuses that mapping and is idempotent; changed source content creates an explicit new revision or a conflict proposal, never a silent overwrite. Do not infer identity from titles. The manifest is per-store importer bookkeeping, not a replicated or public ID object type; include it in ID-native export. If it is missing, offer “import as new” or explicit reconciliation, never title-based auto-matching.

JEX does not provide a promise of source edit-history transfer in the documented export contract. Import the current exported note body as the initial revision; do not fabricate prior revisions. Preserve the source's user-visible created/updated times in source metadata where the target representation supports it, and list any value not representable in the report.

### Fallback: Markdown + Front Matter directory

Accept a user-selected Markdown + Front Matter export when JEX is unavailable. Preserve directory breadcrumbs as notebook tags, parse only fields Joplin documents as supported (`title`, `updated`, `created`, `source`, `author`, location fields, todo state/due date, and `tags`), and keep other front matter as untrusted source metadata in the manifest rather than silently promoting it into ID semantics. Joplin's specification says extra front-matter fields are ignored by its importer, so the ID importer must not promise them in a Joplin round trip.

The fallback is lower fidelity than JEX. Emit warnings for absent source IDs, lost item metadata, path/title sanitization, duplicate paths, unsupported body constructs, and unresolved links/resources. Never merge solely because two files have the same title.

## Mapping

| Joplin source | ID target | Rule |
| --- | --- | --- |
| Note | Stable note ID + `note.create` / initial `note.revise` | One source note maps to one logical note; revision content is the converted Markdown snapshot. The source ID is an import key in the manifest, not the target ID. |
| Title | Alias/name claim on the stable note ID | Preserve the exact title as the authored display name. Duplicate titles are allowed; resolution remains ambiguous rather than selecting a winner. |
| Notebook and nested notebook path | Generic tag on each contained note | One namespaced tag per complete path, so equal leaf names under different parents remain distinct. A path whose encoded tag would exceed the 128-byte tag-name limit uses the bounded truncated-plus-digest key defined in the Decision section, with the full exact path preserved in the manifest and source metadata. Empty notebooks are report-only because #2048 has no container object. |
| Joplin tags | Generic note tags | Preserve spelling and distinct values. Tag normalization/collision, if required by the target store, is reported and never silently coalesced. |
| Internal note link `:/<item-id>` or a Joplin note URI | Typed reference to mapped stable note ID | Resolve by the archive-wide source-ID map; preserve link surface and authored form. A target outside the archive is unresolved, not title-matched. An optional pinned revision is absent because the export supplies only the current snapshot. |
| Resource link `:/<resource-id>` and resource bytes | Content-addressed blob + shared attachment reference | Preserve bytes, media type when known, original filename, and source resource ID in the manifest. Missing bytes produce an explicit broken-resource warning and unresolved reference. The reference shape is owned by #5844. |
| Created/updated/source/author/geolocation/todo metadata | Supported note metadata or source metadata | Preserve supported values exactly when the target has a corresponding field; otherwise retain them in the import manifest and report that they are not first-class #2048 fields. |
| Joplin share/owner state | No automatic target grant | Do not infer private or group visibility from source-client state. Explain the public-read default and require an explicit user choice for narrower grants. |

Ordinary external URLs remain external links. Joplin application URIs that identify an included note are converted to typed references; unsupported callback actions remain external/unresolved and are reported. Internal links to notes omitted from a partial export retain their authored label and unresolved state; they are never rebound by title.

## Joplin-specific round trip

The Joplin adapter exports a Markdown + Front Matter directory profile: one Markdown file per note, notebook breadcrumb directories where representable, a separate attachments directory, relative Markdown links for included notes/resources, and a versioned sidecar manifest mapping exported paths and resource names back to stable ID note/blob IDs. The common interchange decision remains with #6439; this profile must not claim it has settled the multi-tool format.

Forward round-trip contract for `ID → Joplin import` over the exported set:

- Every note body and title survives, subject only to an explicit conversion warning for unsupported Markdown extensions.
- Tags and notebook breadcrumb values survive exactly; folder hierarchy is represented as breadcrumbs/tags, not reconstructed as native ID containers. Breadcrumbs that used the bounded tag form are reconstructed from the manifest/source-metadata copy of the full path, never from the truncated tag alone.
- Every included note link targets the intended Joplin note after import, even when titles collide or change. The exporter resolves the stable ID reference to an archive-relative path; Joplin resolves that path and assigns its own local note ID. No stable ID is promised across the Joplin boundary.
- Every included attachment's bytes match by cryptographic digest, and each note's attachment reference resolves to the same blob. Filenames are labels, not identity.
- The report lists omitted history, empty notebooks, unsupported metadata/sharing, broken or external links, missing resources, and path/name sanitization. A successful import with warnings is not described as lossless.

For a reverse `Joplin → ID → Joplin` check, compare the resulting note/link/tag/resource graph rather than Joplin IDs: JEX import intentionally remaps local IDs. Do not promise that Joplin's own export preserves arbitrary metadata, external references, or note history. JEX is the Joplin-native backup; Markdown + Front Matter is the interoperable working format.

## Rejected alternatives

| Alternative | Why it is not selected |
| --- | --- |
| Import only Markdown files and ignore JEX | Loses the structured archive's IDs, tags, notebook relationships, resource inventory and much metadata; it also makes internal-link repair depend on filenames. Keep as fallback only. |
| Treat Joplin IDs as permanent ID note IDs | Couples identity across systems, risks collisions and confuses source identity with the stable ID object identity. Joplin's importer deliberately remaps IDs; retain a mapping instead. |
| Match unresolved links by title | Duplicate titles and title changes make the result nondeterministic and can silently point at the wrong note. Preserve an unresolved reference and report it. |
| Turn notebooks into new container objects in this adapter | #2048 does not define that object; it would re-decide the content model. Preserve breadcrumb tags and route container semantics to #5839. |
| Embed attachment bytes in Markdown or model them as topic notes | Corrupts content semantics and violates the distinct binary-resource relationship. Use blob storage and the shared #5844 contract. |
| Merge/replace existing notes automatically on import | Makes a backup/import destructive and risks false identity matches. Import is additive and manifest-idempotent; conflict resolution is explicit. |

## Invariants and test corpus

1. The same source archive and import manifest produce the same target mapping; a second import does not duplicate notes, tags, or blobs.
2. No title, filename, or notebook leaf name is an identity key. Duplicate titles and sanitized-path collisions remain distinguishable in the manifest and report.
3. Every internal note reference either resolves through the source-ID map to the intended stable note ID or remains explicitly unresolved; no title-based fallback exists.
4. Every imported attachment reference resolves to bytes whose digest equals the source bytes. Missing source bytes never become an empty successful attachment.
5. Importing a subset never rebinds an omitted target to an included note with the same title.
6. Conversion is deterministic: the same fixture and adapter version yield byte-identical canonical note content, manifest mapping, and warning set.
7. Visibility uses the program's existing public-read resolution order; Joplin sharing metadata is reported, not silently translated.
8. Every notebook breadcrumb tag is at most 128 bytes (`MAX_NAME_BYTES`). The bounded form's digest is computed over the full encoded path, so two distinct over-limit paths do not share a tag short of a SHA-256 collision, and the manifest always carries the full path for exact reconstruction.

Golden corpus (synthetic and redistributable; no private user export): nested notebooks including an empty notebook; duplicate note titles in separate paths; one notebook path long enough to force the bounded tag form; Unicode and long/illegal filenames; two tags differing only by case; note links within and across notebooks, a title-collision link, a dangling link, and an external URL; image/PDF/arbitrary binary resources with repeated filenames but different bytes; one missing resource; todos with due/completed metadata; location/source/author metadata; unsupported front matter; and a partial-export case. The test oracle is a checked-in expected object graph plus manifest and fidelity report: exact note/link/tag identity, exact attachment digests, and exact warning categories. Add one JEX golden fixture and one Markdown + Front Matter fixture; do not use a live profile as the oracle.

## Follow-on seams and scope

- #5844 owns the general attachment/media decision, including typed blob references. This importer contributes the Joplin resource-link cases to that work; it does not create a competing resource-model ticket.
- #5839 owns whether notebook-like hierarchy becomes a first-class content organization primitive. Until then, this adapter uses breadcrumb tags.
- #6439 owns the shared interchange format and broad multi-tool round-trip policy. This document specifies only the Joplin profile.
- This design inherits #5678's read-authority order and does not reopen its program questions. Importer implementation timing remains separate.

## References

### ID model

- [`doc/2026-11-01_design_id2048.md`](2026-11-01_design_id2048.md) — stable note identity/revision graph, title claims, tags, typed topic references, content blobs and explicit access boundary.
- [#5678 program issue](https://github.com/developing-today/code/issues/5678) — ruled read-authority order and import/export programme scope.

### Joplin primary sources

- [Joplin import/export guide](https://github.com/laurent22/joplin/blob/4dbbf2c3/readme/apps/import_export.md) — JEX, Markdown directory import, metadata and supported formats.
- [Markdown + Front Matter interop specification](https://github.com/laurent22/joplin/blob/4dbbf2c3/readme/dev/spec/interop_with_frontmatter.md) — supported metadata fields and import/export behavior.
- [JEX importer](https://github.com/laurent22/joplin/blob/4dbbf2c3/packages/lib/services/interop/InteropService_Importer_Jex.ts) — extracts JEX then delegates to raw importer.
- [Raw importer](https://github.com/laurent22/joplin/blob/4dbbf2c3/packages/lib/services/interop/InteropService_Importer_Raw.ts) — remaps note/folder/resource IDs, copies resource bytes and rewrites linked IDs.
- [Interop service](https://github.com/laurent22/joplin/blob/4dbbf2c3/packages/lib/services/interop/InteropService.ts) — export inventory includes folders, notes, linked resources, tags and note-tag relations.
- [Markdown importer](https://github.com/laurent22/joplin/blob/4dbbf2c3/packages/lib/services/interop/InteropService_Importer_Md.ts) — preserves directory nesting and converts relative links to local notes/resources.
- [Joplin attachments guide](https://github.com/laurent22/joplin/blob/4dbbf2c3/readme/apps/attachments.md) — resource links and attachment behavior.
