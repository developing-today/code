# FORK.md — the developing-today fork of the `kdl` crate

**Origin.** `kdl-org/kdl-rs`, the `kdl` crate **v6.7.1**, taken from
the crates.io package (`kdl-6.7.1.crate`, sha256
`082ddf81b2acd76fe04412655d17befedfff1837db772f8e74c38050d25ed670`).
License Apache-2.0 (`LICENSE`, `NOTICE` retained). Selected as the
fork base by developing-today/code#5753 §3; design record
`doc/2026-11-16_decision_id5753_kdl_fork_section5.md`.

**Why a fork.** The KDL spec allows at most one type annotation, only
on node names and values. ID needs annotations on every part —
property keys and children blocks included (upstream Discussions
kdl-org/kdl#344 and #362, not adopted) — and several per part. The
change is a contained AST generalization in this crate and a
rebuild-everything proposition anywhere else (#5753 §3).

**Tracking upstream.** The fork keeps the upstream package name
(`kdl`), version (6.7.1), file layout, and test suite so upstream
releases diff cleanly against this tree. To update: download the new
release's `.crate`, diff it against this directory, and reapply the
fork changes below. The changes are deliberately concentrated: the
AST fields and accessors in `src/node.rs` / `src/entry.rs`, the
annotation-run parser (`tys`) and its three call sites in
`src/v2_parser.rs`, and the additive `src/kdlx.rs` module.

## Fork changes (Packet A, #5753)

Scaffolding:

- Vendored at `pkgs/id/vendor/kdl`, outside the id workspace
  (`[workspace] exclude` in `pkgs/id/Cargo.toml`), following the
  convention in `../README.md`. `publish = false`.
- `Cargo.toml` rewritten from upstream's `Cargo.toml.orig` with the
  workspace-inherited `miette` dependency resolved inline; the
  `[workspace]` and `[profile.dist]` sections and upstream's release
  tooling (`.github/`, `cliff.toml`, `Makefile.toml`,
  `dist-workspace.toml`, `README.tpl`, `Cargo.lock`) are not carried.
- No member of the id workspace depends on this crate yet; wiring it
  into the #5688 pipeline is Packet B.

AST (`src/node.rs`, `src/entry.rs`):

- `KdlNode.ty: Option<KdlIdentifier>` →
  `KdlNode.annotations: Vec<KdlIdentifier>`; same change for
  `KdlEntry`. Empty vec is the spec case.
- `KdlEntry` gains `key_annotations: Vec<KdlIdentifier>` (property
  keys had no annotation slot in the spec AST).
- `KdlNode` gains `children_annotations: Vec<KdlIdentifier>` beside
  `children` (children blocks had no annotation slot either).
- The upstream accessors keep working: `ty()` / `ty_mut()` address
  the first annotation, `set_ty()` replaces the list with a single
  annotation. New accessors: `annotations()` / `annotations_mut()` /
  `set_annotations()`, `key_annotations()` + mut/set variants,
  `children_annotations()` + mut/set variants.
- Serde (`de.rs` / `se.rs`) and the document API are unchanged in
  behavior: they read through `ty()`, i.e. the first annotation. The
  list-shaped serde surface is Packet B (design record Q3).

Parser (`src/v2_parser.rs`):

- New `tys` parser: a run of annotations wherever upstream parsed a
  single optional one (node name, values via `value()`, and the
  entry head in `node_entry()`). A run in front of a property key
  becomes its key annotations; a run in front of a value merges into
  the value's annotations.
- Children-block annotations: after a node's entries, an annotation
  run directly before `{` becomes the node's
  `children_annotations`.
- Annotation payloads stay exactly the spec's slot — one identifier
  inside the parens (design record Q1). Parameterized payloads from
  `pkgs/mappy/migrations/limit.kdl` (`object#book#author:BrianGriffin`)
  are quoted identifiers under KDL 2.0 rules, since `#` is not a bare
  identifier character; decomposition into triples is the conversion
  layer's job (Packet B), never parser grammar.

`kdlx` declaration (`src/kdlx.rs`, design record Q2):

- `KdlDocument::kdlx_declaration()` parses a first-node
  `kdlx version=1 extensions="…"` declaration; `KdlDocument::lint_kdlx()`
  warns (never errors) on extension use without a declaration, use
  beyond the declared tokens, and unknown declared tokens. Extension
  tokens: `multi-annotation`, `prop-key-annotation`,
  `children-annotation`.

Tests: upstream's suite is carried unmodified except for struct
literals updated to the new AST fields (`src/v2_parser.rs`,
`src/entry.rs`, `src/node.rs` unit tests). Fork behavior is covered
by `tests/fork_extensions.rs` and the unit tests in `src/kdlx.rs`.
The official KDL test-suite cases under `tests/test_cases/` run via
`tests/compliance.rs` (v2 parse, as upstream ships the harness);
v1 parsing is covered by the conversion tests in
`tests/fork_extensions.rs` and upstream's unit tests under the `v1`
feature. Note the one intentional divergence the harness does not
flag: `type_before_prop_key_fail.kdl` asserts the spec rejects an
annotation before a property key — the fork accepts it, as the
prop-key extension it exists to provide. (Upstream's harness only
checks cases that carry an `expected_kdl` file, so the case passes
vacuously; the divergence is recorded here rather than hidden by
removing the case.)

## Known Packet A limits

- **v1 surface is spec-only.** v1 documents parse through the
  upstream `kdlv1` crate (the `kdl` 4.7.0 package) and convert into
  this AST with zero-or-one annotations; the extension grammar is a
  v2 surface in Packet A. (The v1 grammar Drew's `limit.kdl`
  predates is not the fork's extension target; the record's examples
  are all 2.0-shaped.)
- **Format fidelity is first-annotation-exact.** The upstream format
  structs keep one annotation's inner whitespace
  (`before_ty_name` / `after_ty_name` / `after_ty` describe the first
  annotation). Additional annotations render canonically as `(name)`
  with no inner whitespace, and whitespace _between_ annotations, and
  between a children annotation and its `{`, is normalized on
  render. AST round-trips are exact; byte round-trips are exact for
  all spec documents and for extension documents in canonical form.
- **Dead upstream files are carried as-is.** `src/v1_parser.rs`,
  `src/nom_compat.rs`, `src/query.rs`, and `src/query_parser.rs` are
  shipped in the crate but commented out of `src/lib.rs` upstream;
  they do not compile into the fork either and still mention the old
  AST. They are kept so upstream diffs stay clean.
