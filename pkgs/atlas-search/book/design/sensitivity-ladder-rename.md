# Design: renaming the sensitivity ladder in code

> **Partly shipped.** Stage 1 below is in the code. Stages 2 to 4 are a plan, and the open question at the end is not settled here. Tracking: #13506.

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

## Stage 2: dual-read stored labels, behind the open question

Teach the label readers to accept canonical ceiling labels in the published `chunks` and `session_labels` columns, and have SQL filters bind both spellings for each admitted rung. This stage must wait for the one-ladder-or-two decision below, because it is exactly where the tier word `internal` and the ceiling name `internal` meet.

## Stage 3: migrate stored labels

An explicit, reversible migration rewrites published labels from legacy to canonical spellings, verified against a copy of a staging and a publish database before it touches production. Dual-read from stage 2 stays on throughout, so readers never see a half-migrated database fail.

## Stage 4: output cutover

Switch emitted JSON (`max_sensitivity`, `effective_ceiling`, per-hit `sensitivity`), the CLI help and default spelling, and the MCP tool descriptions to the canonical names, with a documented date. Keep accepting legacy input for at least one release after that, then retire it. The Rust enum variants can be renamed at this stage with no wire effect.

## Open question: one ladder or two

After the rename, the ceiling ladder (`public < internal < private < sensitive`) shares the names `public` and `internal` with the corpus-tier vocabulary (`public / internal / secret`), whose `internal` tier spans two ceiling rungs. Whether the end state is one unified ladder or two vocabularies that share two names is a decision for the design owner. Stage 1 does not depend on it; stage 2 does.
