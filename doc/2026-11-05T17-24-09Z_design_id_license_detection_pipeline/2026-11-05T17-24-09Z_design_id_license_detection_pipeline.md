# Design: upload-time license detection pipeline

- **Date:** 2026-11-05
- **Status:** proposed design for review; no scanner implementation is included.
- **Issue:** [#6008](https://github.com/developing-today/code/issues/6008), in the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
- **Inputs:** current repository checkout `08c51c69424611b0929c3dadf5dda4424b667fec`; issue #6008 triage; taxonomy/classifier scope #5977; SPDX/tooling survey #5979; policy and combination-checker boundaries #5984/#5985.
- **Method:** distinguish current repository facts from proposed behavior. The scanner reports evidence; it does not establish intent, ownership, or legal authority.

## Request and scope

Specify a buildable upload-time pipeline that records an uploader's declared license, scans the uploaded artifact, handles disagreements, records registry facts, and exposes inherited licenses to the separate combination checker. Include upload latency/cost and the state of an object whose scan is deferred.

This design does **not** choose license policy for user uploads, implement a scanner, define license compatibility, or decide ownership/attestation. Those decisions belong to the policy and combination-checker work. In particular, GPL/AGPL user-upload handling remains undecided: the server must be able to apply a later policy without changing what the scanner observed.

## Recommendation

Keep three independent, versioned facts for each immutable artifact version:

1. **Declared** — the uploader's assertion, recorded at upload and never rewritten by a scan.
2. **Detected** — scanner observations, including evidence locations, tool/version, and scan status.
3. **Inherited** — licenses reached through the artifact's pinned dependency closure, derived separately and consumed by the combination checker.

After bytes are stored and a name is bound to their content hash, enqueue a scan. Small inputs may be scanned inline within a strict deadline; larger inputs are accepted into a `scan_pending` state and processed asynchronously. Until a usable scan result exists, the registry must not label the object license-verified or feed it to a policy/combination decision as if it were complete. A deployment policy may decide visibility or use; scanner output itself never makes that decision.

## Current state (evidence)

| Existing fact                                                                                                                                                                                                                                                                                       | Design consequence                                                                                                             |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| The upload protocol receives blob bytes before `MetaRequest::Put` binds a name to the hash, and the server refuses a dangling name (`pkgs/id/src/protocol.rs:255-273`).                                                                                                                             | Start license processing only after the name/hash association is committed; never scan an unbound upload as a registry object. |
| File operations centralize name, immutable blob hash, and metadata tags (`pkgs/id/src/fileops.rs:3-20`); generic tags exist, but there is no typed license or scan-state schema (`pkgs/id/src/protocol.rs:315-348`).                                                                                | Add a typed license-observation seam rather than overloading a free-form tag with several meanings.                            |
| The registry architecture describes namespace-to-document mappings and generic auto-tags, not an upload-license registry (`pkgs/id/ARCHITECTURE.md:72-86`).                                                                                                                                         | Treat the license registry record as new data, keyed to an exact artifact version/hash.                                        |
| Existing write metadata stamps provenance at first write and leaves absent read-model values unknown rather than guessing (`pkgs/id/src/fileops.rs:365-434`; `doc/2026-11-04T10-46-17Z_design_listing_origin_and_upload_time/2026-11-04T10-46-17Z_design_listing_origin_and_upload_time.md:26-57`). | Preserve source and timestamp for each license fact; do not backfill a declaration from detection.                             |
| #5977 owns the declared/detected/inherited taxonomy; #5979 researches SPDX formats, scanners, false positives, and registry practices. Both are open; #5979 is parked.                                                                                                                              | Use those seams; leave scanner choice and expression normalization replaceable until #5979 supplies evidence.                  |
| #5955 leaves GPL/AGPL support for user uploads unresolved; #5985 owns compatibility/combination rules.                                                                                                                                                                                              | Record facts and mismatches without hard-coding an upload ban, an allow rule, or compatibility verdict.                        |

## 1. Terms and record shape

- **Artifact version:** one immutable blob hash plus the registry name/version that points to it. A replacement upload is a new version even if its display name is unchanged.
- **Declaration:** uploader-supplied license identifier/expression and optional source (manifest field, UI, or explicit “unknown”). It is an assertion, not a verified fact.
- **Observation:** one scanner's result for one artifact version. It may contain multiple license-text/header matches at different paths; it may also be incomplete or inconclusive.
- **Inherited set:** the distinct declared/detected license facts of resolved dependencies, keyed by dependency artifact hashes and paths. It is not the artifact's own license.
- **Mismatch:** a machine-detected difference between declaration and observations that needs a policy or human decision. It is not a finding of infringement or intent.

The logical record should include:

```text
artifact_hash, registry_name, artifact_version
declared: {expression_or_raw_value, source, actor, recorded_at}
scan: {state, requested_at, completed_at, tool, tool_version, ruleset_digest}
observations: [{identifier_or_raw_match, path, byte_range, evidence_kind, confidence}]
mismatch: {state, reason_codes, compared_facts}
inherited: {dependency_closure_hash, entries: [{artifact_hash, path, license_fact_ref}]}
```

This is a conceptual shape, not a commitment to a storage encoding. #5979 should settle SPDX parsing/canonicalization and tool evidence fields. Preserve unrecognized expressions as raw input plus parse status; never drop them or silently translate them to a known license.

## 2. Upload and scan flow

```text
receive immutable blob bytes
  -> validate upload and compute content hash
  -> bind registry name/version to hash and record declaration
  -> create scan job for that exact hash
       small + within inline budget -> scan inline
       otherwise -> acknowledge as scan_pending and enqueue
  -> append observation and scan status
  -> compare declaration with observations -> mismatch/review state if needed
  -> resolve pinned dependency closure -> publish inherited set
  -> expose complete facts to the policy and combination-checker seams
```

The job is idempotent on `(artifact_hash, scanner_profile_digest)`. A retry of the same upload must not create duplicate current state; it may append a new scan-run record. A scan result is accepted only if its artifact hash still matches the registry version it was requested for. Replacing or deleting a name cannot attach a stale result to the replacement object.

If a declaration is absent, record `unknown`; do not infer it from the scanner. If a declaration cannot be parsed, keep the raw value and record `parse_error` alongside any independent detection result.

## 3. Mismatch handling and registry facts

The scanner never wins over the declaration: it adds an independent observation. For example, a GPL project-level declaration and a permissive source-file header must both remain visible with paths and evidence kinds. The comparison creates `match`, `mismatch`, `unknown`, or `inconclusive`; it does not choose the legally controlling license.

On mismatch:

1. Append a mismatch record referencing the exact declaration and scan run; do not overwrite either fact.
2. Notify the uploader with the declaration, detected evidence, affected paths, and a correction/appeal path. Do not tell them that the server has determined ownership or legal intent.
3. Set the artifact's registry review/availability state according to a server policy. **Proposed safe default:** keep the artifact private and unavailable as a public dependency until review or an explicit policy decision. Servers may configure warn-only behavior; GPL/AGPL allow/reject remains a separate policy choice.
4. Record the actor, time, policy version, and reason for any resolution. A corrected declaration is a new declaration event; the original remains auditable.

Registry facts are append-only observations scoped to the content hash. A later scanner/ruleset run adds a new observation; it does not erase the old run. The current read model may point at the latest successful run, but must expose scanner provenance and `stale`/`unknown` when no current result exists.

## 4. Deferred state and availability

Keep scan progress separate from blob existence and user visibility:

```text
not_requested -> pending -> running -> complete
                             |          -> partial
                             |          -> failed
                             |          -> unsupported
```

- `pending`: the immutable blob and registry name/hash binding exist; work is queued. The object is not represented as verified.
- `running`: a bounded worker owns a lease for the exact hash/profile pair.
- `complete`: all in-scope bytes were examined and observations were committed.
- `partial`: some paths/bytes were examined, but a limit or unsupported container prevented full coverage; it is not equivalent to `complete`.
- `failed`: a transient/tool failure occurred; retry with bounded backoff, then leave a visible failed state for operator action.
- `unsupported`: the input cannot be safely scanned (for example, a format/container the configured tool cannot inspect). Do not treat this as “no license found.”

Large uploads return an accepted/pending result rather than holding the upload connection open. The uploader can inspect their own pending object; public listing, dependency resolution, and downstream policy decisions must consume an explicit availability policy and must not infer `verified` from successful storage. A policy may permit use while pending, but that is an explicit deployment choice recorded with its version.

## 5. Latency and resource budget

Budgets are server-configurable and measured per content hash. Proposed initial targets, to be benchmarked against the selected tool in #5979:

| Path                  | Initial target                                                                                                                 | Behavior on budget exhaustion                                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Inline                | At most 1 MiB total input; 250 ms CPU and 500 ms wall time; upload latency target p95 <= 750 ms including queue/metadata work. | Stop inline work, persist `pending`, enqueue async scan; never extend the request indefinitely.                                                     |
| Deferred worker       | At most 64 MiB per job slice; 10 s CPU and 30 s wall per slice; bounded worker concurrency and queue depth per server.         | Persist a resumable cursor if the format supports it; otherwise record `partial` with the limit and continue only under a configured larger budget. |
| Oversize/unsafe input | Default maximum 256 MiB examined per artifact version; archive expansion and nesting have separate caps.                       | Record `partial` or `unsupported`, keep unverified, and require explicit operator policy for use.                                                   |

These are provisional operational defaults, not scanner guarantees. Record actual bytes read, CPU/wall time, queue delay, and outcome so operators can tune them. Bound archive expansion, file count, nesting depth, and path traversal independently of compressed upload size. Do not launch an unbounded scan from a user-controlled upload.

## 6. Inherited licenses and downstream seam

Resolve inheritance from a pinned, deterministic dependency closure (exact artifact hashes, not mutable names). Preserve the path from the uploaded artifact to each dependency and deduplicate facts by artifact hash while retaining all paths. Cycles, missing dependencies, or an unresolved version produce `inherited: incomplete`, not an empty set. Recompute when a dependency hash changes; publish a closure digest so consumers can tell which graph was evaluated.

The pipeline publishes `(artifact_hash, direct facts, inherited facts, completeness, source run)` to the combination-checker seam owned by #5985. This design does not define compatible combinations, overrides, exceptions, or whether a license is acceptable. That checker receives evidence and applies its own versioned policy.

## 7. Detection limits and security

License scanners identify text, headers, and package metadata. They cannot establish who owns the work, whether a notice is authoritative, whether an exception applies, or what an uploader intended. A high-confidence match remains an observation, not an attestation.

Run scanning in a resource-bounded worker with no network access by default. Treat archives and manifests as hostile: cap expanded bytes, entry count, nesting, and path length; reject path traversal; do not execute package build scripts. Store minimal evidence (path, byte range, match kind, digest, confidence) rather than copying arbitrary source excerpts into public registry records.

## 8. Alternatives and open decisions

- **Block every upload until scanning completes:** simple visibility rule, but makes large uploads depend on scanner latency/availability. Prefer accepted-but-pending with an explicit availability gate.
- **Treat no detection as proof of no license:** rejected; unsupported formats, unrecognized text, and partial scans make that unsafe.
- **Collapse declaration and detection to one “license” field:** rejected; contradicts #5977's taxonomy and destroys provenance.
- **Open:** #5979 must select/validate scanner tooling, expression grammar, confidence/evidence semantics, and benchmark the provisional budgets.
- **Open:** #5969's library manifest design should decide where a declared expression is authored; this pipeline consumes it without defining the manifest syntax.
- **Open:** policy work must decide whether mismatch blocks public use and how GPL/AGPL user uploads are treated. Do not infer the answer from the hard-reject rule for our own dependencies.
- **Open:** define retention and privacy for detailed match evidence and the operator workflow for persistent `failed`, `partial`, or `mismatch` states.

## Acceptance check for #6008

- [x] Stages from declaration through scan, mismatch, registry record, and inherited-license handoff are specified.
- [x] Declared, detected, and inherited facts remain distinct; mismatch does not silently overwrite either side.
- [x] Who is notified, what is recorded, and how policy resolves availability are specified without claiming legal authority.
- [x] Deferred large-upload state and explicit latency/resource budgets are specified.
- [x] Scanner/tool choice, taxonomy, ownership, and combination policy remain at their owning issues.

## Sources and change log

- [#6008](https://github.com/developing-today/code/issues/6008) and triage note [#issuecomment-6659119262](https://github.com/developing-today/code/issues/6008#issuecomment-6659119262).
- [#5977 taxonomy/classifier](https://github.com/developing-today/code/issues/5977), [#5979 SPDX/tooling survey](https://github.com/developing-today/code/issues/5979), [#5955 program map](https://github.com/developing-today/code/issues/5955), [#5984 policy/attestation](https://github.com/developing-today/code/issues/5984), [#5985 combination checker](https://github.com/developing-today/code/issues/5985), and [#5969 library manifest](https://github.com/developing-today/code/issues/5969).
- Current implementation references: `pkgs/id/src/protocol.rs:255-273,315-348`; `pkgs/id/src/fileops.rs:3-20,365-434`; `pkgs/id/ARCHITECTURE.md:72-86`; `doc/2026-11-04T10-46-17Z_design_listing_origin_and_upload_time/`.
- **Change log:** 2026-11-05 — initial proposed pipeline design for #6008; tooling and upload-policy decisions remain open as listed above.
