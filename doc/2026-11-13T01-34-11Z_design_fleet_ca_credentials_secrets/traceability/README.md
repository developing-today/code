# Traceability integration (preparation)

This directory defines the reviewed bridge from source statements to canonical
requirements, specific proposals, design sections, test oracles, and (after
publication) numeric issue receipts. It is intentionally not populated from the
currently mutable inventory or proposal drafts. A mapping is a human-reviewed
assertion, not a keyword match or a domain-starter inference.

## Mapping contract

Each mapping shard is an explicit JSON array of rows, or a version-1 object
containing `rows`. Each row
has exactly `source_id`, `canonical_requirement_id`, `proposal_id`,
`design_section`, `test_oracle`, `disposition`, and `reason`. Every row has a
specific nonempty reason. Dispositions include `proposal`/`mapped`, `partial`,
`unresolved-cross-domain`, `covered-with-limits`, `research`, `future`,
`approval-blocked`, `unsupported`, `existing`, `superseded`, `administrative`,
and `historical`. A `proposal_id` means the row still requires a verified numeric
receipt in final mode, regardless of disposition. Prefix reasons with `COVERED:`
only for a whole atomic outcome, `PARTIAL:` for incomplete coverage, and
`UNRESOLVED/CROSS-DOMAIN:` when the source row spans unresolved outcomes.
Multiple source IDs may map to one canonical requirement. A source ID must
occur only once across all mapping shards.
An empty `canonical_requirement_id` is permitted only for explicitly classified
administrative, historical, or superseded records whose inventory row has no
canonical IDs; the builder never invents a grouping ID.
Nested `atomic_members` and `cohesive_members` are mapped independently in
separate `--member-mapping` shards. Each member row has exactly `member_id`,
`parent_source_id`, `canonical_requirement_id`, `proposal_id`, `design_section`,
`test_oracle`, `disposition`, and `reason` (plus the same narrow
`external_issue` exception). There is no implicit parent mapping inheritance:
each member gets an explicit proposal target and its own oracle.
Draft mode does not select between colliding shard rows: it emits one
`unresolved-cross-domain` row per source ID and lists the collision. Final mode
fails on any collision.
The sole row-schema exception is `disposition: external-issue`, which adds a
required structured `external_issue` object. It is limited to intake-known issue
numbers 9077, 6767, and 2056, with exact URL, title, state, verification time,
verifier, and source-outcome scope evidence. It is an issue reference, not code
evidence and not an implementation receipt. `existing` requires
`design_section: census:<repo-relative-file>` pointing to an actual repo file,
plus an `EXISTING-CODE:` reason citing that path and a concrete test/evidence
oracle.
Final mode requires each research/future row to point to an exact proposal or
whitelisted external issue; issue references are counted separately from
verified existing-code capability evidence.
Oracle quality is reported separately: known generic/raw issue boilerplate is
not counted as a concrete acceptance oracle, and final mode rejects it until a
reviewed mapping supplies an outcome-specific invariant/reference and case.

Every included source statement is emitted with its original source range and
an excerpt hash calculated from the exact inclusive source lines. The builder
does not modify any inputs. `current_status` remains the source's reported
status; `implementation_status` is copied only from explicit source evidence or
`not-recorded`. `issue_publication_status` is a separate, receipt-backed field.

Every mode verifies each row against `source_path`, inclusive `source_lines`,
the current source-file SHA-256, and byte-exact `source_excerpt`; malformed or
stale source evidence fails regardless of disposition. Final mode additionally
requires a reviewed mapping for every source ID and every nested member ID,
proposal IDs that resolve to the shared proposal-array schema, no partial or unresolved outcome, and
verified issue-plus-queue receipts for every referenced proposal. The original
domain starter remains in `domain_starter`; `primary_issue_number` is a numeric
local issue receipt or an explicitly verified external issue reference, while
`numeric_issue_receipt` is reserved for local publication receipts and
`dependent_issue_numbers` come from verified native dependency edges. Source
`implementation_status` is never inferred from issue publication. There is no
template, keyword, or starter-issue fallback.
The default `--source-root` is the immutable snapshot
`source-snapshots/intake-34faffbff2/sources`, pinned to commit
`34faffbff26615e313488f6d04705a444342e41e`. Never point this at the formatted
`sources/` view; generated rows include both the pinned commit and snapshot path.
The snapshot copy, auxiliary note and integrity note were added at commit `40af815bf0`; generated
rows and proposal headers retain this separate locator commit for resolvable links.
Each inventory full-file hash must equal `source-snapshots/provenance.json`'s
original SHA-256 and the current snapshot bytes; excerpt bytes must independently
equal the declared lines.
The sole auxiliary exception is the exact `note.md` recorded in the pinned
snapshot's parent directory; no other path outside the snapshot source root is
accepted.

## Outputs

The command emits deterministic JSONL and CSV coordinator rows and a Markdown
summary. Outputs are only written when `--output-dir` is explicitly supplied;
the directory must already exist. Default invocation validates and prints a
summary to stdout. Existing generated outputs are refused unless `--overwrite`
is explicitly supplied with `--output-dir`. All command-line arguments are
parsed strictly before input reads or output effects.

Example (using frozen inputs only):

```sh
python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/tools/build_traceability.py \
  --inventory <frozen-inventory.jsonl> --proposals <normalized-proposals.json> \
  --mapping <reviewed-shard-1.json> --mapping <reviewed-shard-2.json> \
  --member-mapping <reviewed-member-shard-1.json> --member-mapping <reviewed-member-shard-2.json> \
  --receipts <verified-publication-receipts.jsonl> --published-manifest <published-manifest.json> \
  --source-root doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-snapshots/intake-34faffbff2/sources \
  --output-dir <existing-output-dir> --final
```

Coverage reporting distinguishes top-level source statements from nested
member outcomes, active unique canonical requirements, context groups, source
statements by explicit disposition, and partial or
unresolved source outcomes. Counts are sets of source IDs and unique canonical
IDs, so aliases do not inflate canonical totals. Unmapped, partial, unresolved,
research, future, approval-blocked, unsupported, existing, superseded,
administrative and historical items remain separately visible.

## Proposal normalization

`tools/prepare_manifest.py` is the separate, pure preparation step for authored
proposal arrays. Its explicit taxonomy and urgency projections are in
`normalization-policy.json`. Unknown kind strings fail; composite-kind mappings
are exact entries with review reasons; exact proposal-ID overrides bind an
authored kind to its actual primary deliverable when a composite differs. All optional authored metadata and the
full original kind/ID are retained in an administrative body header, alongside
the design-only lifecycle/authorization guard and common original-source
commit, immutable snapshot root, integrity note, and auxiliary note pin. Only the
publisher's exact 11 manifest fields are emitted. Proposal IDs and dependency
references are not normalized: exact authored case/bytes are preserved per the
publisher's alphanumeric ID contract. Default mode is a no-write dry run. Writing
requires both `--write` and an existing `--output-dir`; replacing an existing
manifest additionally requires `--overwrite`.
Before the final manifest hash, pass `--traceability-matrix traceability.jsonl`
to bind every local proposal reciprocally to reviewed source IDs, canonical
requirement IDs, and per-member planned oracles. The normalizer appends a
standalone issue-body section for each exact source/member binding, preserves
authored requirement IDs as metadata, refuses orphan proposals, and fails
rather than truncating any rendered issue body over 65,536 bytes.
