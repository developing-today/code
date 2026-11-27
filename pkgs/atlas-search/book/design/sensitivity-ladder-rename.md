# Design: renaming the sensitivity ladder in code

> **Stages 1–2 shipped.** Stage 1 accepts canonical names as ceiling input; stage 2 dual-reads explicitly rung-typed published labels. Stages 3–4 remain planned. The staging-tier distinction is settled by the decision on #13506. Tracking: #13506.

The sensitivity ladder's canonical names are `public < internal < private < sensitive`. The code still spells the rungs `shareable`, `personal`, `local-only` and `secret-bearing`, and those spellings are not only identifiers: they are stored labels in the corpus and staging databases, values in service and user configuration, CLI arguments, and fields on the wire. A one-step rename of the enum variants and their serde spellings would make every existing database, config and script unreadable. The rename therefore moves through four stages, each safe to ship on its own.

| Legacy spelling  | Canonical name |
| ---------------- | -------------- |
| `shareable`      | `public`       |
| `personal`       | `internal`     |
| `local-only`     | `private`      |
| `secret-bearing` | `sensitive`    |

## The rule that shapes every stage

Ceilings and stored labels are parsed by different functions, and only ceilings learn the new names first.

A ceiling is a request: it can only narrow what a caller sees, and the server clamps it to the caller's cap and the destination context. Accepting `internal` as a synonym for the `personal` ceiling is safe.

A stored label is a fact about a chunk. Staging databases already store the corpus-tier word `internal` (see [Sensitivity](../concepts/sensitivity.md)), and that tier covers both `personal` and `local-only` chunks. If the label parser read `internal` as the `personal` rung, a `local-only` chunk in staging could be served under a `personal` ceiling. So `Sensitivity::from_name` stays strict, and the new names enter through a separate `Sensitivity::from_ceiling_name`.

## Stage 1: accept the canonical names as ceiling input (shipped)

- `atlas-core` and `atlas-proto` gain `from_ceiling_name` (legacy or canonical in, the same rung out) and `canonical_name`.
- Every ceiling input uses it: `--max-sensitivity` on the Rust CLI, the via-service request, the service's per-client `ceiling_cap`, and the `ceiling` field of a `SearchRequest` on the wire (a field-level deserializer; every other sensitivity field keeps the strict serde names).
- The Python `atlas_search.py` maps the canonical names to the legacy spellings before validation, so the two implementations agree.
- Nothing emitted changes. SQL filters still bind the legacy stored spellings, envelopes still echo `max_sensitivity` and `effective_ceiling` in legacy spelling, and validation error texts are unchanged for parity with the Python fixtures.
- Unchanged on purpose: `atlas-stage` segment files and every stored-label read stay strict.

Consumers outside this package that pass a ceiling through (`fleet search --max-sensitivity`, the Worker and MCP pinned ceiling strings, `pkgs/id`, `pkgs/mcpx`, `pkgs/note-submission`, the pm-stack aggregator) keep working unchanged and can switch to the canonical names at any time after stage 1 is deployed.

## Stage 2: dual-read stored labels (shipped)

Teach label readers to accept canonical names only in explicitly rung-typed published `chunks.sensitivity` and `session_labels.sensitivity`, and have SQL filters bind both spellings for each admitted rung. This does not change the strict `Tier::parse` path for staging-tier fields: the unversioned tier word `internal` continues to mean the physical internal corpus tier, never the canonical personal ceiling rung. Rust and Python search/classification readers use the dual-read; emitted values remain legacy until stage 4. The chosen two-vocabulary interpretation is recorded in the #13506 decision.

## Stage 3: migrate stored labels

An explicit, reversible migration rewrites published labels from legacy to canonical spellings, verified against a copy of a staging and a publish database before it touches production. Dual-read from stage 2 stays on throughout, so readers never see a half-migrated database fail.

## Stage 4: output cutover

Switch emitted JSON (`max_sensitivity`, `effective_ceiling`, per-hit `sensitivity`), the CLI help and default spelling, and the MCP tool descriptions to the canonical names, with a documented date. Keep accepting legacy input for at least one release after that, then retire it. The Rust enum variants can be renamed at this stage with no wire effect.

## Decision: two vocabularies

The ceiling ladder (`public < internal < private < sensitive`) and corpus-tier vocabulary (`public / internal / secret`) remain distinct, even though they share two names. The corpus-tier `internal` spans the `personal` and `local-only` ceiling rungs. Consequently, stage 2 accepts canonical names only in explicitly rung-typed published labels and does not reinterpret unversioned staging-tier values. See the decision comment on #13506 for rationale and the conditions required before any future unification.
