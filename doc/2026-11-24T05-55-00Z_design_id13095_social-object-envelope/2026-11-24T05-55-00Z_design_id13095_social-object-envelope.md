# Social object envelope and profiles in id-core

- **Created:** 2026-11-24
- **Tracking issue:** [#13095](https://github.com/developing-today/code/issues/13095)
- **Parent design:** [#9128 social app reference model](../2026-11-23_design_id9128_social_app_reference_model.md)
- **Parent epic:** #9107; program #9106
- **Status:** implementation design; no code changes are included in this record.

This supplement specifies the #13095 id-core slice. The parent design remains
authoritative for application semantics; this record makes the envelope,
validation, bounded traversal, and implementation sequence concrete enough to
build and verify.

## 1. Problem and context

The parent design defines social objects and uncapped `parent_ref` nesting, but
does not prescribe a Rust contract or executable validation boundary. Issue
#13095 asks for the envelope, four application profiles, and bounded cycle-safe
traversal. Without a shared core contract, producers and readers can disagree
on required fields, accept malformed references, or recurse through attacker-
controlled graphs without resource bounds.

The current `id-core` crate is the foundation leaf crate, with public modules
declared in `pkgs/id/crates/id-core/src/lib.rs`; its dependencies already
include serde, serde_json, and thiserror. The source tree has no social-object
module today. The package's established pattern is a focused public module with
typed errors and unit/property tests in the crate. The parent model in
`doc/2026-11-23_design_id9128_social_app_reference_model.md` §2 defines the
envelope; §3–5 define feed-entry, profile, topic, comment, and reaction shapes;
§7 fixes the default traversal bounds at depth 32, 2,000 visited nodes, and
8 MiB per page.

## 2. Goals and non-goals

### Goals

- Define a serializable, versioned `SocialObject` envelope with stable typed
  fields for object id, author, untrusted display-only creation claim, optional
  parent reference, content reference, schema version, optional succeeds
  reference, and profile tag.
- Validate envelope well-formedness and profile-specific required fields using
  a pure API; do not perform storage, signature verification, or network I/O.
- Support exactly the parent-design profile tags: `feed-entry`, `profile`,
  `topic`, `comment`, and `reaction` (four application profiles, with topic
  and comment as distinct shapes within the board profile family).
- Provide bounded, cycle-safe parent-link traversal with configurable limits,
  the §7 defaults, back-reference markers, and an opaque continuation cursor.
- Prove deterministic serialization round trips, rejection behavior, and exact
  boundary behavior in tests.

### Non-goals

- No append-list admission, ordering, query, storage, feed ranking, moderation,
  or tombstone implementation (#9125–#9127 and #9130 own those contracts).
- No `succeeds_ref` lineage validation; its edge semantics belong to #9129.
- No network cursor protocol, signed cursor, authorization decision, or
  freshness-token implementation (#13094 / parent §6).
- No UI rendering policy or data-model nesting cap. Traversal limits bound a
  read, not what may be stored or admitted.

## 3. Alternatives and tradeoffs

1. **One typed envelope plus profile-specific payload references (chosen).**
   The envelope carries identity and graph relations uniformly; profile rules
   validate the content schema through a small typed payload/required-field
   contract. This keeps graph traversal independent of app semantics and
   leaves content-addressed bodies in the content plane. It adds a validation
   seam that must stay synchronized with the parent design.
2. **Separate Rust structs for every app shape.** This gives compile-time
   profile-specific fields but duplicates common identity and relation fields,
   complicates generic traversal, and makes forward-compatible profile tags
   harder to read. Not chosen for the first slice; typed profile views can be
   added later without changing the common envelope.
3. **Store arbitrary JSON in the envelope and validate only at consumers.**
   This is flexible and cheap initially, but lets malformed objects cross the
   shared core boundary and makes the acceptance criteria difficult to test
   consistently. Rejected.
4. **Do nothing / leave the design as prose.** This avoids immediate code and
   dependency work but leaves #13095's acceptance unmet and every consumer to
   invent its own validation and traversal limits. Rejected.

## 4. Chosen contract and safety analysis

### Envelope and validation

Implement a focused `social_object` module in `id-core`, exported from
`lib.rs`. The public envelope mirrors #9128 §2. Use the repository's canonical
ID/reference types where compatible; do not introduce stringly typed IDs just
to avoid a dependency. Serde representation is versioned by `schema_version`
and `profile`; unknown versions or profile tags fail closed with typed errors.
`created_claim` is never used for authorization, ordering, or freshness.

The pure validator checks: required fields are present and non-empty; object
and content references parse as their declared ID types; schema version is
supported; profile-specific required fields are present and correctly typed;
and a parent reference is absent only where that profile permits a root.
Profile rules are explicit and table-driven. A reaction requires a target
(`parent_ref`) and may have an empty body; a topic and comment require their
parent relation; a root profile must not require one; a feed-entry must point
to the surfaced object through a typed `target_ref` in its profile payload;
`content_ref` continues to identify the content-plane body. The exact
profile-required content fields are limited to those stated in #9128 §§3–5;
this slice does not invent a global reserved profile-attribute registry.

Validation deliberately does not assert that referenced objects exist, that
the author signed the envelope, that the signer is authorized, or that
`succeeds_ref` is valid. Those are storage/identity, admission, and #9129
lineage responsibilities. Errors identify the field/rule without echoing
untrusted body content.

### Traversal contract

Expose a pure traversal over a caller-provided parent lookup, with a policy
value defaulting to `max_display_depth = 32`, `max_visited_nodes = 2_000`, and
`max_response_bytes = 8 * 1024 * 1024`. It returns visited objects in the
documented deterministic walk order, explicit `BackReference` results for
already-visited ids, and a continuation cursor when a bound stops the page.
The cursor contains the traversal frontier and policy/version identity; it is
opaque to callers and is not an authorization token. Resume reuses the visited
set/frontier state needed to avoid loops and duplicate output.

All bounds are inclusive: the object at the depth/node/byte limit may be
returned if it fits; traversal stops before adding an item that would exceed a
limit. The root is depth zero and counts as one visited node. If one serialized
item alone exceeds the byte budget, return a typed `ItemExceedsBudget` error
rather than issue an unusable cursor. Cycles produce a back-reference and do
not consume recursive depth. Missing parent targets are reported as a typed
missing-reference result, not followed. A cursor from a different policy or
schema version is rejected and the caller starts a fresh page.

### Security and capabilities

This is a pure core module: no new capabilities, ports, filesystem access,
network access, or secret handling. The principal risks are resource
exhaustion and treating attacker-controlled timestamps/links as trusted.
Finite node, depth, and byte budgets plus iterative traversal bound work;
typed ID parsing rejects malformed references; untrusted timestamps remain
display-only. Callers must still apply access control before supplying objects
to the traversal.

## 5. Dependency-ordered build slices

Each slice is sized for one lane/session; later slices depend on earlier
contracts.

### S1 — Envelope types and well-formedness validator

- **Goal:** add the public envelope, profile tag, typed validation errors, and
  pure well-formedness/profile validation in `id-core`.
- **Enables:** stable serialization and safe reuse by feed/profile/board
  consumers.
- **Acceptance:** unit tests round-trip a valid minimal fixture for each
  profile; malformed IDs, missing required fields, unsupported schema version,
  unknown profile, and forbidden root/parent combinations are rejected with
  the expected typed error. Oracle: #9128 §§2–5 and the validator rules in §4.

### S2 — Bounded iterative parent traversal

- **Depends on:** S1 envelope and ID types.
- **Goal:** implement the parent lookup abstraction, configurable bounds,
  deterministic iterative walk, cycle back-references, and continuation.
- **Enables:** safe views over arbitrarily nested stored objects.
- **Acceptance:** tests hit depth 32, node 2,000, and 8 MiB exactly and prove
  the next item is not emitted; a parent cycle terminates with a back-reference;
  resume neither repeats nor loses a node; oversized single item fails typed.
  Oracle: #9128 §7 defaults and §4 boundary semantics.

### S3 — Public facade and contract-level regression tests

- **Depends on:** S1 and S2.
- **Goal:** expose the module through the intended `id-core`/root facade
  without leaking storage concerns; document public APIs and add integration
  coverage of serde fixtures plus validator-to-traversal behavior.
- **Enables:** downstream implementation issues to consume one stable surface.
- **Acceptance:** crate-level integration tests compile through the public
  export; all five profile tags round-trip; the full acceptance matrix from
  #13095 passes. Oracle: issue #13095 and #9128 §§2–5, §7.

## 6. Rollout and rollback

The feature is additive and initially has no production caller or persisted
wire data. Land the core module and tests first; downstream callers opt in via
the public API. Do not silently relax bounds or reinterpret an existing profile
tag. If a compatibility or resource issue appears before downstream adoption,
revert the additive module/export and its tests. After persisted objects exist,
rollback must retain deserialization compatibility for already-written
envelopes; use a new schema version or reader compatibility change rather than
deleting a version in place. No data migration is required for this design-only
slice.

## 7. Verification plan

- **Serialization oracle:** deterministic fixture equality and round-trip for
  each profile tag; malformed and unsupported versions fail closed.
- **Validation oracle:** table-driven valid/invalid profile cases map to the
  explicit requirements in #9128 §§2–5.
- **Traversal oracle:** boundary tests at exactly and one over each default;
  a generated cyclic graph confirms termination and back-reference output;
  cursor resume confirms no duplicate/lost results; byte accounting uses the
  serialized representation used for the response.
- **Commands:** from `pkgs/id`, run the focused `cargo test -p id-core
  social_object` targets, then `cargo test -p id-core`; run the package's
  formatter and lint checks for changed Rust files. The implementer records
  exact commands and outcomes in the implementation PR.
- **Live proof:** not applicable; the module is pure and this design adds no
  deployed endpoint. Downstream integration tests are the release gate.

## 8. Open questions and named deciders

1. **Should serialized envelope IDs use the existing graph `Term` type or
   dedicated newtypes?** Default: reuse existing canonical graph ID/reference
   types where their semantics match, otherwise add narrow newtypes with
   conversions at the boundary. **Decider:** Drew (ID program product/design
   direction); implementation owner reports evidence before changing this
   default.
2. **Should a byte-boundary continuation cursor carry the complete visited
   set?** Default: yes, or a lossless compact representation, because omitting
   it risks cycle/duplicate errors on resume; do not use a lossy Bloom filter
   as a correctness mechanism. **Decider:** Drew for externally visible
   semantics; implementer selects encoding within that invariant.
3. **How should content-profile required fields evolve?** Default: schema
   versioned profile rules fail closed for unknown versions, while optional
   attributes remain extensible; no reserved global field registry in this
   slice. **Decider:** Drew for schema evolution policy.
4. **Who verifies signatures and object existence?** Default: callers and
   admission/storage layers; this module remains pure and validates shape only.
   **Decider:** Drew for any future change of layer boundary; no such change is
   required to ship #13095.

## 9. Record and tracking links

This is a dated design record under `doc/`, supplements the parent design
`doc/2026-11-23_design_id9128_social_app_reference_model.md`, and is tracked by
open implementation issue #13095. Queue task 21664 produced this supplement;
the linked PR and issue comment provide the delivery trail. The parent
design's implementation split remains: #13094 owns freshness, #13095 owns the
envelope/profiles/traversal, and #13096 owns board-view projection.

## Change log

- 2026-11-24: initial nine-point implementation design supplement for #13095.
