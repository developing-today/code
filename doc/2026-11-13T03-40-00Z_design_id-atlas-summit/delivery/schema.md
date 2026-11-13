# Delivery row schema

This is the canonical authored row format for a single issue-worthy implementation or research slice. A slice is not publishable merely because it parses: the validation report must have `valid: true`, complete 384-statement coverage, all 38 component homes, the approved manifest digest, and a separate #9112 orchestration record. Authored metadata is not inferred from nearby text.

## Fields

| Field | Type | Requirement |
|---|---|---|
| `id` | string | Stable unique identifier; used in the idempotency marker. |
| `parent` | positive issue number or `developing-today/code#N` | Existing component home #9110–#9111 or #9113–#9148. Only the named original-30/producers research scopes may use #9112. |
| `title`, `goal`, `enables` | nonempty strings | Separate fields. `enables` is never copied into `goal`. |
| `kind` | nonempty string | Authored classification such as `implementation`, `research`, or a component-specific kind. |
| `version` | nonempty string/list | Explicit V1–V4 contract scope. |
| `tier` | `crucial`, `recommended`, `contingent`, or `horizon` | A crucial slice may not have a direct or transitive hard dependency on a lower tier. |
| `statements` | array of IDs or same-section ranges | Must expand to known IDs in the fixed 384-statement inventory. Empty only for the separate original-30 provenance research row; it does not claim statement coverage. |
| `owned_files` | array of `{path,status,owner?}` | Exact repo-relative file paths. `status` is `new` or `existing`; absent owner defaults to this slice ID. Directory-level surfaces are not accepted. A path marked new must not exist in the current checkout; existing must exist. |
| `requires` | array of refs | Hard prerequisites. Ref is an exact slice ID, `{ "ref": "slice-id", "description": "..." }`, or `{ "issue": 9083, "description": "..." }`. A `#9083 description` string is also normalized as issue identity plus description. |
| `after` | array of refs | Soft order only. These edges never enter the hard DAG, native GitHub blocker list, or queue dependency set. |
| `test` | object `{file,command,oracle}` | Exact new owned test/artifact path, proposed command scoped to that path/target, and an oracle. For research, `file` is a new owned report/fixture or an exact new owned domain validator. The command must invoke an existing validator or that explicitly owned new script, and state its real domain oracle; a future validator is not claimed to have run. Research may use golden/reference oracles for compatibility findings, not only matrix oracles. Do not invent unrelated production-code tests for research. |
| `test.oracle` | structured object | `{ "kind": "invariant"|"model"|"golden"|"reference"|"research-matrix", "expected": "...", "verifier": "..." }`. `expected` is the asserted outcome; `verifier` identifies the comparison/checker. No keyword heuristic is used to certify oracle quality. |
| `test.oracle` (authored-text alternative) | nonempty string | Accepted only alongside `fresh_judge_review: {reviewer,reviewed_at,decision:"accepted",reference}` with ISO-8601 review time. The tool records but does not independently attest the judge. |
| `rollback` | nonempty string | Explicit slice-scoped rollback behavior. |
| `design_source`, `design_section` | exact repo file and exact heading | Section accepts the exact ATX source line or exact rendered heading text after removing only the `#{1,6}` prefix. No substring matching. Anchors mirror [`github-slugger`'s `slug()`](https://github.com/Flet/github-slugger/blob/master/index.js): punctuation is removed and each literal space is replaced (`.replace(/ /g, '-')`); `## 4. #9125 — Foo` renders as `4. #9125 — Foo`, slug `4-9125--foo`. |
| `owner` | string | Slice owner; distinct from per-file owner. |
| `research_scope`, `trigger`, `research_evidence` | research rows | Required scopes must appear exactly once; additional lowercase kebab-case scope tags are allowed. Research rows need explicit trigger and nonempty source/evidence references. |

`requires` and `after` are intentionally distinct. A string that combines an ID and prose, such as `C9133-01 result schema`, is rejected; use `{ "ref": "C9133-01", "description": "result schema" }`. This avoids fuzzy token extraction and prevents soft ordering from silently becoming a blocker.

Paths are normalized with POSIX `./` removal before file ownership, test-path and collision comparisons. Each path's effective owner is its explicit `owner`, or the slice ID if omitted. Every pair sharing a file must have distinct effective owners and be ordered in one direction by transitive hard dependency; a single unrelated edge elsewhere in a collision group is insufficient. Draft output names are checked for sanitized collisions before any directory/file write.

Explicit report/draft outputs are refused if they resolve to, contain, or are the same inode (including an existing hard link) as an input source. Report and draft destinations must also be disjoint.

The manifest digest is SHA-256 over UTF-8 canonical JSON bytes: JSON objects use sorted keys, separators `,` and `:` without spaces, and `ensure_ascii=false`; the top-level digest value is the ordered list of `[repo-relative manifest path, parsed manifest object]` pairs. Exact default order is:

1. `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/implementation-slices.json`
2. `.../part-a-addressing/implementation-slices.json`
3. `.../part-a-graph/implementation-slices.json`
4. `.../part-b/implementation-slices.json`
5. `.../part-c/implementation-slices.json`
6. `.../crosscut/research-slices.json`
7. `.../delivery/research-slices.json`

## Valid row example

The example below is row-valid as authored metadata. It is a fixture, not a claim that the whole manifest set passes. Its new path is intentionally absent until the slice creates it.

```json
{
  "id": "EXAMPLE-S0-01",
  "parent": 9110,
  "title": "Preserve explicit port ownership in the contract test",
  "kind": "implementation",
  "version": "V2",
  "tier": "crucial",
  "statements": ["S0.1"],
  "goal": "Keep the blob, event, world, and sync store authorities distinct at the port boundary.",
  "enables": "Independent adapters can conform without claiming a generic store owns every authority.",
  "owned_files": [
    {"path": "pkgs/id/tests/example_port_ownership.rs", "status": "new"}
  ],
  "requires": [],
  "after": [],
  "test": {
    "file": "pkgs/id/tests/example_port_ownership.rs",
    "command": "cargo test --manifest-path pkgs/id/Cargo.toml --test example_port_ownership",
    "oracle": {
      "kind": "invariant",
      "expected": "Each authority is assigned to exactly its owning port; a generic-store assignment fails the assertion.",
      "verifier": "The named Rust integration test compares the authority-to-port table against this fixed expected mapping."
    }
  },
  "rollback": "Revert only the new test and its contract change; do not migrate data or weaken another port's guarantees.",
  "design_source": "crosscut/versioned-design.md",
  "design_section": "Stable APIs and ordering",
  "owner": "id-store-slice-owner"
}
```

## Offline validation report interface

The explicit `prepare_delivery.py --report PATH` output is JSON. `entries` is a normalized array in the exact manifest/row order above; each object includes `id`, `source`, `parent`, `title`, `goal`, `enables`, `version`, `kind`, `tier`, `statements`, `owned`, `test_file`, `command`, structured `oracle`, `rollback`, `research_evidence`, `design_source`, `design_section`, `owner`, `trigger`, `research_scope`, separated `hard` and `soft` edges, and stable `marker`. `orchestration` is a normalized separate #9112 record.

```json
{
  "manifest_sha256": "<sha256>",
  "valid": false,
  "records": 125,
  "component_homes": 38,
  "component_homes_expected": 38,
  "statements_covered": 384,
  "statements_expected": 384,
  "exemptions": 0,
  "entries": [{"id":"...", "source":"doc/.../implementation-slices.json", "parent":9110, "hard":[], "soft":[], "marker":"..."}],
  "orchestration": {"id":"meta-9112-sequencing", "parent":9112, "hard":[], "soft":[]},
  "errors": ["all gate failures"]
}
```

Consumers must require `valid: true` and exact approved `manifest_sha256`; a failed report still includes partial normalized rows for diagnosis, never authorization. This preparation worker does not publish issues or queue tasks.

Research artifacts use the same strict `--input` CLI. Generic `research-matrix` reports require at least two options, nonempty criteria, exactly one assessment per option/criterion pair, statuses `supported|unsupported|unknown`, evidence references resolvable to a dated source ledger, decision/reason/limitations, and explicit experiment records for any measured/empirical claim. A local benchmark plan with no runs can say `unmeasured` and mark claims unknown. The validator verifies structural completeness and traceability only; it does **not** verify factual truth, source interpretation, or performance. The issue's research Done check still requires fresh primary-source review.

For original-30 provenance, `delivery/research-slices.json` adds one contingent #9112 research row. Its standard-library shared artifact validator is invoked as `validate_research_report.py --input <future-report.json>`; the future report path belongs to the row, not the validator. The original-30 format accepts either exactly 30 unique source-backed records or an `unrecovered` status with completed bounded scan and no fabricated item list. A bounded negative result is a valid research outcome.
