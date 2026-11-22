# Metadata schema v1 and evolution doctrine

**Status:** proposed contract for Atlas corpus metadata and PM issue/PR Metadata. This is the workstream-C elaboration for #9095; it does not define Atlas's other workstreams or implement a producer/reader.

## 1. Goals and boundaries

One versioned Metadata contract serves two related surfaces:

1. Atlas corpus artifacts: source items, sections/chunks, embeds, and classification records.
2. PM records: the JSON dictionary in the `<!-- pm-metadata:start/end -->` block on issue and PR bodies (`pm_meta` is its version key; see the PM hierarchy design §12).

The surfaces share the evolution rules, identity rules, extras behavior, and link semantics. Their domain fields remain distinct. A consumer may project either surface into its own typed view, but must not silently reinterpret one surface as the other. Metadata describes and indexes an artifact; it does not itself grant authority, prove a classification, or replace the artifact's bytes.

Goals: old metadata remains valid under newer software; authors can add fields without schema-rewrite permission; consumers can safely filter/index selected fields; malformed or unknown values do not silently weaken safety; and an artifact has a stable main-store identity whether signed or unsigned.

## 2. Versioned envelope and identity

Every Metadata object carries a positive integer schema version. PM uses `pm_meta`; Atlas uses `schema_version`. Version is part of the object being interpreted, not inferred from the producer, filename, deployment, or database migration level. V1 writers emit `1`; readers dispatch by the carried version and preserve the original bytes/value when projecting it.

The artifact identity is external to the bytes whose digest it names, avoiding a self-referential hash:

```text
MetadataArtifactId = "sha256:" + lowercase_hex(SHA-256(exact_metadata_file_bytes))
```

For a PM record, which is embedded in an issue or PR body rather than stored as a file, `exact_metadata_file_bytes` is the JSON object text inside the block: take the bytes between the `<!-- pm-metadata:start -->` and `<!-- pm-metadata:end -->` markers as the body is returned by the source API, remove one opening code-fence line (```` ``` ```` or ```` ```json ````) and its closing fence line if present, then trim leading and trailing ASCII whitespace (space, tab, CR, LF). The remaining bytes, from the object's opening `{` to its closing `}`, are hashed unchanged. The markers, the fence, and the surrounding body are never part of the digest.

The main-store record/reference carries this ID for signed and unsigned artifacts alike. Do not hash a parsed-and-reserialized value, normalize line endings, or include a signature companion in the metadata-file digest. A signature is a companion assertion over the exact data bytes. The `.json.signature` companion contains the detached signature and its verification profile; `.cert` carries certificate material/reference. A `.json.signed` container may package the original data and its asserted primary signature, but does not change the data artifact ID. **V1 convention: sign the data bytes**, not the signature companion; an envelope may attest to both as an explicit higher-level statement, but the detached data signature is the primary proof. This is reversible by a new explicit signature profile, never by changing old bytes or guessing from filenames.

V1 Metadata envelope:

```json
{
  "schema_version": 1,
  "extras": {},
  "fields": {}
}
```

For PM bodies, `pm_meta` replaces `schema_version`; the outer JSON dictionary is the metadata object, and the `fields` wrapper is omitted for compatibility with §12. Its version and domain fields are at the top level, with unknown keys held in the `extras` object. The canonical PM V1 form is therefore:

```json
{
  "pm_meta": 1,
  "direct_parent": null,
  "links": [],
  "lineage": {"design": null, "roadmap_item": null, "priority": null},
  "provenance": {},
  "extras": {}
}
```

An absent `extras` on a legacy V1 PM object means `{}`; readers do not synthesize and write it back as a mutation. `direct_parent`, `links`, `lineage`, and `provenance` retain the field meanings and example shape in PM design §12. Atlas `fields` contains the typed Atlas-domain fields defined by its producer profile; `extras` is sibling data, not a replacement for declared fields. A future writer may add optional fields without changing the version only where the applicable profile explicitly reserves an extension point; otherwise a schema revision is required.

## 3. V1 field, enum, and extras rules

### 3.1 Declared fields and validation

Each surface publishes a versioned field registry: for every declared field it names the path, value type, required/optional status, indexing policy, enum policy (if any), and safety effect. The PM V1 registry starts with the exact keys above and the §12 nested PM shape; Atlas's registry is owned by its corpus producer profile. A consumer must not infer a field's type or semantics from its name. Unrecognized keys are not silently promoted to declared fields.

The V1 core enum fields are `kind`, `area`, `group`, and `sensitivity`. Each registry entry publishes its allowed V1 values and one of these policies:

- **closed:** an unknown value is not a recognized core value; preserve the original value in `extras` with its source path and report it as unclassified. Do not substitute a guessed category.
- **open:** preserve unknown values as supplied and allow them only in the explicitly open extension slot; they do not acquire registered semantics or index behavior until registered.

`sensitivity` is safety-critical and closed. Missing, ill-typed, or unrecognized sensitivity resolves to the most restrictive value defined by the active deployment profile, and the original input is retained in `extras` for correction. This fallback is never applied to ordinary `kind`, `area`, or `group` fields. Readers preserve unknown enum tokens; writers emit only registered values unless the field is explicitly open. Adding enum values is additive only for fields declared open; changing a closed set is a breaking candidate.

### 3.2 Extras and type mismatch

`extras` is a root-level JSON object keyed by the original field name/path. It stores unrecognized optional data and data a reader cannot safely interpret. Preserve JSON value types, nested structure, and original enum token; do not stringify, discard, or claim successful classification. A sidecar provenance entry may record original path, version, reason (`unknown-field`, `unknown-enum`, `type-mismatch`, or `unconvertible`), and observation time; it must not overwrite the value.

Relocation never overwrites. When the key a relocated value would take is already present in `extras` with a different value, the existing value stays, and the relocated value is appended to the reserved array `extras["$relocated"]` as `{"path": <source path>, "value": <original value>, "reason": <reason>}`. Keys beginning with `$` are reserved for this doctrine; a source key beginning with `$` that the doctrine does not define is itself relocated into `$relocated`. Both values and both paths therefore survive an update.

If a value has the wrong type for a declared field, keep the declared-field view absent/defaulted according to that field's rule and relocate the original value unchanged to `extras`; report the mismatch. Lossless, explicitly registered coercions are allowed only when round-trip safe for the source value. For example, an integer array may become a string array if every element has a canonical decimal rendering; a string array may become an integer array only if every element parses exactly and re-encodes to the same string. Partial conversion is forbidden. If any element fails, preserve the whole original value in `extras`. A projection never rewrites source bytes as a side effect of reading.

### 3.3 Optionality and indexing

Optionality is the forward-compatibility mechanism: V1 optional fields remain optional forever in the additive path. They may be indexed when extracted, but an absent/unindexed field is not evidence that the property is false. Indexes are derived and rebuildable; the complete Metadata JSON and source bytes remain authoritative. Extra fields are preserved but are not automatically indexed or queryable as typed fields.

## 4. Links, provenance, and reporting amendment

PM `links` retain the existing record identity/status fields (`target`, `relation`, `status`, `since`, `until`, `reason`) and add an additive expression form for graph consumers. A link expression may name arrays of subjects and predicates and one or more object targets; each member is a typed reference, not a display string. The scalar PM shape remains readable as a one-subject/one-predicate/one-target expression. Writers must not split a multi-subject/multi-predicate assertion into unrelated records when doing so would lose its assertion identity. This aligns with the multi-subject/multi-predicate expression model on #9125/#9126; those issues own the graph wire format.

For the 2026-11-13 reporting-architecture amendment:

1. Every Metadata artifact has a main-store ID, including unsigned artifacts; the exact-byte content hash above is the minimum V1 ID.
2. Metadata artifacts are first-class link sources and targets. Links support multi-subject and multi-predicate expressions, without claiming that the PM JSON block is the graph protocol.
3. Signature companions are in the artifact family. V1 signs the data bytes; the detached signature is separately addressable and the `.json.signed` form can preserve an asserted original signature. The signature profile and reversal path are explicit, rather than emergent filename convention.
4. A schema bump triggers a bounded reclassification review: a fast, inexpensive model reviews schema changes section by section, then classifies affected sections using the latest schema and all prior plus new data. “Affected” means a section containing a field, enum, coercion, or interpretation changed by the bump, plus sections whose dependency/provenance graph says they consume such a field. Unchanged sections remain valid at their existing version and are not silently re-stamped. Record the change set, affected-section selection, model/profile, inputs, output IDs, and failures.

The signature decision is intentionally narrow: this design selects signing the data rather than signing only the detached signature. It does not define key custody, certificate trust, or authorization; those remain with the owning signing/identity work.

## 5. Evolution doctrine

### 5.1 Additive path (default)

- Never remove or rename a field; never change its meaning or type incompatibly; never make an optional field required.
- Add optional fields only, with documented type, defaults/projection behavior, index policy, privacy/sensitivity effect, and unknown-reader behavior.
- Add values only to enums explicitly declared open. For closed enums, park the change for a breaking batch.
- Writers emit the current version. Readers accept every prior version indefinitely. An older embed or Metadata object must run under a newer system; do not require a parallel deployed schema or a producer rewrite merely to read it.
- Keep raw prior-version data and its content ID. Readers may build a current in-memory projection/upcast, but must preserve unknown values and must not rewrite or re-stamp the stored artifact on read.

“Indefinitely” means a supported reader path remains available for every published version. It does not require every consumer to maintain separate business logic for every old schema: a version adapter maps prior representations into the current canonical projection, while the original remains intact and auditable.

### 5.2 Breaking-change parking lot and batch gate

Required-ification, field removal/rename, incompatible type/meaning change, closed-enum expansion, or a change that would invalidate prior readers is a **candidate**, not an implementation permission. Record each candidate on #9095 with: field(s), proposed change, reason, old-reader impact, affected sections/artifacts, additive alternative considered, reclassification cost estimate, and evidence. Keep the lot append-only; mark disposition by a new comment, never erase history.

A breaking batch is justified only when either (a) at least three independently actionable candidates can be applied coherently in one version event, or (b) one candidate has documented correctness, integrity, or safety impact that cannot be contained additively. The batch proposal must list the complete ready set, explicitly defer the rest, compare the additive workaround, estimate reader/writer/reclassification cost, and receive a design review before changing the contract. A high item count alone is not a reason to break consumers.

For an approved batch:

1. Publish one new schema version and one migration map for every changed field/enum, with old and new examples and deterministic conversion or explicit failure behavior.
2. Readers continue accepting all prior versions indefinitely through version adapters; the compatibility adapter is part of the release, not a temporary dual-schema deployment. Old bytes/IDs remain valid and unchanged.
3. Writers switch to the new current version at a declared cutover. During rollout, old writers may still submit old versions; readers accept them and surface a compatibility observation. Never silently label old data as new.
4. Reclassify affected sections only, using the latest schema plus all prior and new source data. Unaffected sections remain at their prior version. Persist a manifest of selected sections, inputs, output IDs, model/profile, success/failure, and retry state. Failed sections remain visibly old-version and retryable; do not claim the batch complete until all affected sections either succeed or have an explicit, reviewed terminal disposition.
5. Verify counts and hashes, test representative old-version reads and new writes, inspect safety-enum fallback behavior, then mark the lot entries batched with the version and migration evidence. A later batch cannot erase earlier version history.

No dual-schema support burden means no indefinite duplication of domain logic or a fleet of concurrent writers: one current writer and one canonical projection, with thin permanent readers/upcasters for historic encodings. Historic versions are not deleted or made invalid to obtain that simplicity.

## 6. Consumer obligations and decision ownership

Classifier/embed authors may add optional, registered fields within their owning profile, preserve unknown fields via `extras`, and declare indexing/enum behavior without requesting a rewrite of this shared doctrine. They may not redefine shared field semantics, weaken sensitivity fallback, make optional fields required, or change signatures/identity. Such proposals use the parking-lot/batch path. DB/query implementations index extracted registered fields and retain full Metadata; body/source data wins on content divergence, and divergence is surfaced for healing rather than silently overwritten.

Schema readers must expose version and compatibility status, preserve unknowns, make coercion/mismatch visible, apply fail-closed sensitivity fallback, and distinguish missing from false. Writers must write the current version and registered values, preserve unrelated Metadata on update, and never mint an ID that omits the exact file bytes. Reclassification is section-scoped and auditable as specified above.

## 7. Implementation split implied by this design

1. **Contract and fixtures:** freeze PM and Atlas V1 field registries, exact enum inventories/policies, JSON examples, hash/signature vectors, extras and coercion cases, and prior-version compatibility fixtures. Resolve Atlas's exact metadata-file boundary with its producer; do not infer it from chunk/content hashes.
2. **Shared readers/writers:** parse versioned PM blocks and Atlas documents; preserve unknowns; implement mismatch reporting, registered lossless coercions, current-version writes, and exact-byte identity without mutating reads.
3. **PM `fleet gh` create/update path (#9073):** merge existing PM Metadata, stamp only fields fleet owns, preserve caller fields/unknown extras, enforce required V1 fields, and insert the same full record into the DB. Body remains content authority; report divergence.
4. **Metadata store and query (#9074):** store complete JSON plus exact source/artifact ID and extracted registered fields; index only declared fields; support field filters with missing-vs-false semantics; expose source/DB divergence and rebuild/reconciliation.
5. **Compatibility and reclassification:** add version adapters, affected-section selection, bounded section-by-section model review, provenance manifests, retry/failure states, and no-restamp tests for unchanged sections.
6. **Conformance and safety:** property/model tests for exact-byte IDs, parse/serialize preservation, unknown fields/enums, enum fallback, lossless-only coercion, previous-version reads, multi-subject/multi-predicate links, signature companions, and batch/migration completeness. Use a reference parser/projection and byte-hash vectors as explicit oracles.

These are design slices, not new queue issues. #9095 owns the design record; #9060 §12, #9073, and #9074 consume the contract in their existing scopes. No schema implementation is included here.

## Sources and change record

- #9095 issue body and comments: 2026-11-12 parking-lot record; 2026-11-13 reporting-architecture amendment.
- `doc/2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md` §C: six-rule source doctrine.
- `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md` §12–15: PM Metadata block, fleet superset, and DB/query contracts.
- #9073 and #9074 current issue records: implementation acceptance and boundaries.
- 2026-11-21: expanded §C into a versioned contract, selected V1 signature convention and batch threshold, incorporated the reporting amendment, and stated the implementation split. No other Atlas workstream is specified.
