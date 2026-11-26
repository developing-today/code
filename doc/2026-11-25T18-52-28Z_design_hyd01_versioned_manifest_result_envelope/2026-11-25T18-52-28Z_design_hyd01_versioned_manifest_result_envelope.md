# HYD-01 — Freeze versioned hydration manifest and result envelope

Issue: [#9458](https://github.com/developing-today/code/issues/9458)
(slice `HYD-01` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.** This
record elaborates the manifest/envelope slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the frozen schema and result contract an implementation
issue can be scoped against. It does not re-decide anything that
package settled, and it does not authorize code, schema, migration,
deployment, or live acceptance; those remain separately gated stages
(see §8).

Governing decisions consumed, not re-opened:

- **Component #9207, HYD-01 slice text**: goal (schema/reference
  examples for the versioned manifest, per-source completeness,
  generation lifecycle, result/errors), owned files
  (`pkgs/fleet/crates/fleet-queue/src/work/context.rs`, shared
  identity/command/result references in `contracts.rs`, tests in
  `context.rs`), acceptance, negative cases, and the migration rule
  (contract version additive; old artifacts read-only; disable the
  serializer without deleting files). This record specifies those, it
  does not replace them.
- **FDR-PMO-004** (immutable hydration, per-source cursors,
  attribution): generations are immutable once published, cursors
  advance only after durable processing, and partial/truncated state
  is never presented as complete. HYD-01 owns the manifest that
  *states* completeness; HYD-03
  ([#9460](https://github.com/developing-today/code/issues/9460))
  owns the immutable-generation/atomic-publication mechanics that
  *produce* it, and HYD-05
  ([#9507](https://github.com/developing-today/code/issues/9507),
  design record merged 2026-11-23) owns the page/cursor commits the
  per-source entries summarize. Neither slice re-decides the other.
- **CLI-02** ([#9438](https://github.com/developing-today/code/issues/9438),
  CLOSED; design record PR #13255 merged 2026-11-23): the shared
  command contract and strict parser own the result-envelope shape
  every work subcommand returns. HYD-01's envelope (§5) is the
  hydration specialization of that contract, not a parallel one:
  wire names snake_case, authority/work IDs are a string pair, and
  unsafe integer IDs/versions/generations are decimal strings.
- **CEN-04** ([#9434](https://github.com/developing-today/code/issues/9434),
  design record PR #13341 merged 2026-11-23): the cross-model schema
  owner matrix is the pinned code-ownership/storage convention
  HYD-01's hard blocker named. A persisted manifest field with no
  matrix row may not be written; §3's fields are stated against that
  gate, and any field the matrix does not admit is dropped from this
  freeze, not smuggled in.

Verified state at writing (2026-11-25, origin/main
`61aed93e4db33753cab43f300573835cf3211976`):
`pkgs/fleet/crates/fleet-queue/src/work/context.rs` does not exist
(the `work/` module carries `authority.rs`, `completion.rs`,
`contracts.rs`, `degraded.rs`, `journal.rs`, `lifecycle.rs`,
`operations.rs`, `priority.rs`, `routing.rs`, `selection.rs`,
`sqlite_replica.rs`), neither acceptance test
(`pmo_hyd_manifest_missing_required_source_is_partial`,
`pmo_hyd_manifest_rejects_unknown_schema_version`) exists as code
anywhere on main — both appear only in the design package — and no
hydration manifest is written by any code path today. The two hard
blockers named in the slice text have materially changed since the
2026-11-22 blocked note on #9458: the granular #9206
identity/result-envelope slice (CLI-02, #9438) is CLOSED with its
record merged, and the #9198 pinned ownership/storage convention
(CEN-04, #9434) has its record merged. What still gates
*implementation* is recorded in §8; nothing there gates this design
freeze.

## 1. Intent

Every other HYD slice writes into, or reads from, the bundle at
`.fleet/work/<id>/context/`. Without a frozen manifest, each slice
invents its own answer to the only question the bundle exists to
answer honestly: *what do we actually have, and what is missing?*
The failure modes are symmetric:

- **Overclaim.** A fetch fails, a page is truncated at an API cap, a
  PR head moves mid-retrieval — and the bundle is still labelled
  complete, so every downstream agent reasons from a partial record
  presented as whole. The starter's invariant is absolute:
  incomplete hydration is NEVER silently marked complete.
- **Underclaim / mush.** Completeness as one global boolean with no
  per-source detail: a bundle missing only review threads is
  indistinguishable from one missing the PR itself, and no reader
  can tell what to re-fetch.

So completeness is per source/endpoint first and aggregated second
(§4), the manifest is versioned so a reader can refuse what it does
not understand instead of coercing it (§3), and the result envelope
(§5) reports the same status the manifest persists — the caller and
the durable record never disagree.

## 2. Bundle layout (owned by the manifest, produced by HYD-03)

The manifest is `manifest.json` at the bundle root, written last in
its generation and the only file whose presence publishes the
generation. Sibling artifacts are the component design's fixed set:
`summary.md`, `queue.json`, `issues.json`, `pull_requests.json`,
`comments.json`, `reviews.json`, `checks.json`, `timeline.json`,
`commits.json`, `dependencies.json`, `activity-cursors.json`, and
`diffs/pr-<n>.patch`. Large payloads and diffs live as artifact
refs (path + sha256 + bytes) in the manifest, never as queue-row
blobs. GitHub text inside every artifact is untrusted task data,
not instructions.

## 3. Manifest schema — `fleet.context-manifest/v1` (frozen)

```json
{
  "schema_version": "fleet.context-manifest/v1",
  "work": { "authority_id": "local", "work_id": "842" },
  "generation_id": "g-<stable unique id>",
  "created_at": "<RFC3339 UTC>",
  "status": "complete | partial | failed",
  "source_pin": { "queue_version": "19", "lifecycle_version": "6" },
  "sources": [
    {
      "source_ref": "github://owner/repo/pulls/231",
      "endpoint": "pull-request-detail",
      "api_version": "<negotiated version>",
      "query_fingerprint": "<stable digest>",
      "status": "complete | partial | unavailable | forbidden | not_found | truncated",
      "fetched_at": "<RFC3339 UTC>",
      "source_updated_at": "<RFC3339 UTC or null>",
      "remote_version": { "head_sha": "...", "etag": "..." },
      "pages_fetched": 1,
      "records_seen": 1,
      "records_persisted": 1,
      "truncation_reason": null,
      "artifact_refs": [{ "path": "...", "sha256": "...", "bytes": 123 }],
      "error": null
    }
  ],
  "summary_ref": { "path": "summary.md", "sha256": "..." },
  "previous_generation": "...",
  "integrity": { "algorithm": "sha256", "manifest_digest": "..." }
}
```

Freeze rules:

- `schema_version` is an opaque versioned string. A reader that
  does not recognize it MUST reject the manifest with a typed
  unknown-version error (§5); it MUST NOT coerce, default, or
  best-effort parse it. New versions are additive: a v2 writer does
  not change v1 bytes, and v1 artifacts stay read-only forever.
- `work` is the string pair (authority_id, work_id); unsafe integer
  IDs, versions, and generations are decimal strings throughout —
  no JSON numbers for values that can exceed 2^53 or that act as
  identities.
- `integrity.manifest_digest` covers the manifest's canonical bytes
  *excluding* the digest field itself; hashing a manifest including
  its own digest is not the contract. Artifact hashes cover exact
  artifact bytes. Independent framing and changed-field fixtures
  accompany the implementation (per the component design's digest
  contract), so a serializer change that alters any covered byte
  fails a fixture, not a downstream reader.
- `source_pin` records the queue/lifecycle versions the bundle was
  hydrated against; a bundle is evidence about a pinned state, not
  about whatever the queue happens to hold when it is read.
- Every PR-derived source entry binds to one captured PR `head_sha`
  in `remote_version`. If the head changes during retrieval, old
  diff and new head metadata are never combined: the source is
  retried within the caller's bounded deadline or marked
  `partial` with the stale-head reason, and the prior complete
  generation stays available.
- Private data is excluded by construction: fields the access
  policy or the CEN-04 matrix does not admit are absent, not
  redacted-in-place.

## 4. Completeness aggregation (the honesty rule, made mechanical)

Per-source status is exactly one of `complete`, `partial`,
`unavailable`, `forbidden`, `not_found`, `truncated`, or — for
optional resources only — an explicit `not_applicable`. There is no
default: a required source with no entry, or an entry with no
status, is a malformed manifest, not a complete one.

Generation `status` is derived, never independently asserted:

- `complete` — every required source is `complete` (fetched to its
  endpoint's documented bound and durably stored: no omitted pages,
  no truncation, no forbidden/missing dependency, no error).
- `partial` — any required source is `partial`, `unavailable`,
  `forbidden`, `not_found`, or `truncated`; or any required source
  is missing from `sources`. (First failing oracle:
  `pmo_hyd_manifest_missing_required_source_is_partial`.)
- `failed` — no generation could be published at all; the prior
  generation, if any, remains current and untouched.

`summary.md` is a concise navigation layer — findings, blocker /
review / check warnings, source refs, freshness, the partial-status
summary, and exact file paths — and is never evidence of
completeness by itself. Missing data is summarized as missing,
never as empty.

## 5. Result envelope and errors

The hydrate/refresh commands return the CLI-02 result envelope
specialized with the manifest's own status, so the caller's view
and the persisted manifest are one fact:

- `status`: the generation status of §4, verbatim.
- `generation_id`, `manifest_ref` (path + sha256): which generation
  this result describes; a `failed` result names the still-current
  prior generation instead, or none.
- `sources`: the per-source status list of §4, verbatim — errors
  are per-source data (`error` on the entry), not a single
  top-level failure that hides which sources landed.
- Typed top-level errors are reserved for conditions under which no
  per-source statement is possible: unknown `schema_version` on a
  read/repair path (second failing oracle:
  `pmo_hyd_manifest_rejects_unknown_schema_version`), invalid digest
  or path in a presented manifest, and work-identity resolution
  failure. Each is a named, matchable error kind — never a string
  to parse and never a silent coercion to `complete` or to v1.

## 6. Generation lifecycle (manifest view)

A generation is `staging` (being built outside the current
pointer), then exactly one of `published` (the current pointer
names it; immutable thereafter) or `failed`/`partial`-outside-current
(a failed refresh leaves the prior current bundle intact and marks
the new generation outside the pointer). The manifest records
`previous_generation` so the chain is auditable. HYD-03 (#9460)
owns the atomic-publication mechanics (manifest written last,
pointer swap or CAS equivalent); this freeze owns only what the
manifest *states* about the lifecycle: generation_id, created_at,
previous_generation, and the derived status of §4.

## 7. First failing oracle (implementation entry condition)

Before any implementation, both tests must be listed and fail:

- `pmo_hyd_manifest_missing_required_source_is_partial`
- `pmo_hyd_manifest_rejects_unknown_schema_version`

Done command (from the slice text):
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`
must list both, then
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_hyd_manifest_`
runs them. Negative cases beyond the two oracles: missing source
called complete, invalid digest/path accepted, unknown version
silently coerced — each is a test, not a review comment.

## 8. Gates and next steps

This record freezes the contract. Implementation remains a
separately scoped issue, filed only when its prerequisites are
resolved, and production adoption, migration, deployment, and live
acceptance remain separately authorized stages (#9196 safety
gates; design only — no live queue/dispatch changes).

Gate state revalidated 2026-11-25:

- PR #9225 (design package) — MERGED. Met.
- CLI-02 #9438 (granular #9206 identity/result-envelope slice) —
  CLOSED, record PR #13255 merged 2026-11-23. Met.
- CEN-04 #9434 (#9198 pinned ownership/storage convention) —
  record PR #13341 merged 2026-11-23; issue stays open as the
  slice tracker. Met for design purposes; the implementation
  issue must cite the matrix rows for every §3 field it writes.
- CEN-07 #9437 (real-adapter conformance and CI adoption, under
  #9198) — OPEN, blocked on the #9200/#9207/#9208/#9209 adapter
  seams. This is the remaining #9198 gate for *implementation*
  (a manifest serializer with no conformance-tested adapter would
  freeze bytes nothing else reads), not for this design freeze.
- Downstream consumers of this freeze, in dependency order:
  HYD-02 #9459 (source keys/paths the manifest references),
  HYD-03 #9460 (generation mechanics), HYD-04 #9461 (full adapter
  + summary), HYD-05 #9507 / HYD-06 #9508 (page commits and
  activity revisions the per-source entries summarize).

Migration/rollback for the future implementation, per the slice
text: contract version additive; old artifacts read-only; disable
the serializer without deleting files; unit, deployed rollout, and
live GitHub read recorded separately.
