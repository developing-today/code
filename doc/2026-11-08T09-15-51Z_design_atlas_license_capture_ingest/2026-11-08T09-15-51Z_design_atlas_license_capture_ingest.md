# Atlas — license capture at ingest: per-repo license + D.6 flag in provenance

Design doc — 2026-11-08. This note is the deliverable for #6053
(license capture at ingest), filed under parent #6024 (licensing gate
for the external corpus). Provenance substrate: #3125 (provenance
registry — chunk occurrences, content-addressed extracts). Detection
machinery survey: #5979 (SPDX identifiers and license-detection
tooling for the classifier). Research source: the Atlas ingestion
prior-art brief, Cluster 6 (GitHub content licensing — GitHub ToS
§D read in full, effective 2026-04-27). Design only; no
implementation is authorized by this note. The go/no-go is §6.

State verified at `origin/main` `1cc430ad99` (2026-11-08), fresh for
this design pass (task 12899, re-verifying triage task 12464's
confirmed verdict):

- Issue #6053 is OPEN, `kind/design`, `status/triaged`, no held
  labels. Parent #6024 and relations #5979, #3125 are all OPEN.
- No ingest license provenance exists: the provenance Atlas records
  today is _search/model_ provenance (e.g. `atlas-core/src/search.rs`
  records which model/table produced a result pool; `atlas-embed`
  fixtures record the ONNX Runtime version). Nothing records, per
  external repo, what license was detected at ingest, whether the
  D.6 inbound=outbound argument applies, or which policy zone the
  repo was assigned. A policy change today means re-crawling; this
  design makes it a re-evaluation over recorded rows.

---

## 1. Problem

The #6024 policy can only be re-evaluated per repo if the corpus
remembers what it knew at ingest time. Three facts decide a repo's
treatment, and all three are ingest-time observations:

1. the repo's license, as detectable at ingest (it can change later;
   the ingest-time value is the one the ingest decision rested on);
2. whether the repo _carries a license notice_ — the precondition of
   the GitHub ToS §D.6 inbound=outbound argument for its comments
   (contested for comments; see §4 — we record the flag and the
   license version, never a conclusion);
3. the policy zone the repo was assigned under the sibling #6024
   policy (own repos / participated external / general external /
   GitHub's own docs), because zone + license together determine
   what the corpus may do with the repo's content.

If these are not captured at ingest, every later policy change — a
lawyer answer under #6024, a zone redefinition, a license-family
ruling — forces a re-crawl of the web to reconstruct them, and
re-crawling cannot recover the ingest-time state at all (a repo
relicensed last week was ingested under last month's license).

## 2. The record: `ingest_repo_provenance`

One row per (repo, ingest generation), stored in the #3125
provenance registry alongside the chunk-occurrence records, keyed by
repo identity — not per chunk. Chunks already carry their repo
identity through the registry's occurrence records, so the license
facts join at query time; denormalizing them onto ~107k chunk rows
would rewrite the corpus on every re-detection for a fact that
changes per-repo, not per-chunk.

| Field                | Type             | Meaning                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repo_id`            | text, PK part    | Canonical repo identity: `github.com/<owner>/<name>` lowercased.                                                                                                                                                                                                                                                                                                              |
| `ingest_generation`  | integer, PK part | Monotonic per-repo ingest counter; a re-ingest under changed facts appends a new generation, never edits history.                                                                                                                                                                                                                                                             |
| `ingested_at`        | timestamp (UTC)  | When this generation's content was ingested.                                                                                                                                                                                                                                                                                                                                  |
| `license_spdx`       | text, nullable   | SPDX _expression_ (per #5979's format recommendation), e.g. `MIT`, `MIT OR Apache-2.0`, `LicenseRef-Unidentified`. `NULL` means detection ran and found nothing — distinct from "not yet detected", which is a missing row.                                                                                                                                                   |
| `license_raw`        | text, nullable   | The verbatim detected license name/notice summary as the detector reported it, kept so a taxonomy change (#5979) can re-map without re-detecting.                                                                                                                                                                                                                             |
| `detection_source`   | enum             | `github_license_api` \| `licensee` \| `scancode` \| `manifest_field` \| `manual`. Which rung of the §3 cascade produced `license_spdx`.                                                                                                                                                                                                                                       |
| `detection_evidence` | text             | Pointer to the evidence: the LICENSE file path + content hash at the ingested commit, or the API response digest. Content-addressed, per #3125's extract model.                                                                                                                                                                                                               |
| `ingested_commit`    | text, nullable   | The commit SHA ingested (for repo-content ingests); `NULL` for comment-only ingests, where `ingested_at` + the API snapshot window is the anchor.                                                                                                                                                                                                                             |
| `has_license_notice` | boolean          | The repo contains a license notice (LICENSE/COPYING file or a license header tree) — the §D.6 precondition fact.                                                                                                                                                                                                                                                              |
| `d6_applicable`      | enum             | `yes` \| `no` \| `contested`. Derived, never hand-set: `no` when `has_license_notice` is false; `contested` when a notice exists (the comments-as-content reading is Lawyer Question 1 under #6024); `yes` is reserved for a future lawyer ruling and is not emitted by any current rule. Recording the flag + license version, not a conclusion, is the issue's requirement. |
| `license_version`    | text, nullable   | The version/variant of the noticed license where the detector distinguishes it (e.g. `GPL-3.0-only` vs `GPL-3.0-or-later` is carried in `license_spdx`; this field carries the notice's own version string when it differs, e.g. an unversioned GPL notice).                                                                                                                  |
| `zone`               | enum             | `own` \| `participated_external` \| `general_external` \| `github_docs`. The #6024 Cluster-6 zone assigned at ingest. Zone is a policy input recorded verbatim; re-zoning is a policy-layer act that writes a new generation (§5).                                                                                                                                            |
| `detector_version`   | text             | Version of the detection tooling that produced this row, so a detector upgrade can target stale rows for re-detection instead of re-running everything.                                                                                                                                                                                                                       |

Invariants (property-shaped, per the fleet testing doctrine —
each is a PBT oracle over the registry):

- **I1.** Every ingested external repo has ≥1 provenance row whose
  `ingested_at` ≤ the ingest time of its earliest chunk. No silent
  unprovenanced ingest.
- **I2.** `d6_applicable = no` ⟺ `has_license_notice = false`, for
  every row the derivation rule produced. The flag is a function of
  recorded facts, never an independent opinion.
- **I3.** Rows are append-only: corrections are new generations.
  The row a past policy decision cited is byte-identical forever.
- **I4.** `license_spdx` always parses as an SPDX expression or is
  `NULL` (detected-nothing); unparseable detector output lands in
  `license_raw` with `license_spdx = LicenseRef-Unidentified`,
  never as free text in the SPDX field.

## 3. Detection source: a cascade, recorded per row

Per #5979's survey (licensee, ScanCode, `cargo-about`/`cargo-deny`,
REUSE), in cheapest-first order; the first rung that yields a
determination wins, and `detection_source` says which:

1. **GitHub license API** (`GET /repos/{owner}/{repo}/license`) —
   licensee-based, returns the SPDX id GitHub itself detected plus
   the notice file. Zero crawl cost; this is also the natural
   capture point for `has_license_notice` and the notice file's
   content hash.
2. **licensee on the ingested tree** — for non-GitHub sources and
   for repos where the API reports `NOASSERTION` but a notice file
   exists in the tree we already fetched.
3. **ScanCode** — the fallback for partial/variant texts licensee
   declines; slower, run only on the notice files, not the tree.
4. **Manifest fields** (`manifest_field`) — where the repo declares
   its license in a package manifest the other rungs missed
   (crates.io/npm/PyPI metadata, per #5979's registry survey).
5. **manual** — a human ruling, with the ruling's link in
   `detection_evidence`. Manual rows outrank detector rows in
   re-evaluation (§5) until superseded by a newer manual row.

The cascade order is policy, not convenience: it prefers the
platform's own detection (the counterparty whose ToS frames the
question), then deterministic local matching, then the heavy
scanner. A detector disagreement is not merged or voted: the
winning rung's answer is the row, and the losing answers are kept
in `detection_evidence` so a later taxonomy change can revisit
them without re-fetching.

## 4. The D.6 flag — record, don't conclude

ToS §D.6 (verbatim, from the Cluster-6 research): _"Whenever you
add Content to a repository containing notice of a license, you
license that Content under the same terms…"_ Whether an issue
comment is "Content added to a repository" in D.6's sense is
contested — it is Lawyer Question 1 under #6024 and this design
does not answer it. What the corpus needs is the _inputs_ to that
answer, frozen at ingest:

- `has_license_notice` — the D.6 precondition ("a repository
  containing notice of a license");
- `license_spdx` + `license_version` — "the same terms" the flag
  would point at;
- `ingested_at` — because D.6's grant, if it applies, applies to
  content as licensed when added; a repo relicensed later does not
  retroactively change the terms for comments added earlier, and
  per-generation rows keep those eras separate.

`d6_applicable` therefore ships with exactly two emitted values —
`no` (no notice: the argument cannot start) and `contested`
(notice present: the argument exists and is unresolved). Emitting
`yes` requires the #6024 lawyer answer; when it arrives it is a
derivation-rule change (§5), not a data migration.

## 5. Re-evaluation: the policy layer's query

A policy change is a pure function over recorded rows. The policy
layer holds a _rule version_ (e.g. `policy-6024 v3`); re-evaluation
for a rule change is:

```sql
-- rows the new rule would treat differently, before any re-ingest
SELECT p.repo_id, p.license_spdx, p.d6_applicable, p.zone
FROM ingest_repo_provenance p
WHERE p.ingest_generation = (
        SELECT MAX(ingest_generation) FROM ingest_repo_provenance
        WHERE repo_id = p.repo_id)
  AND policy_eval(p.license_spdx, p.d6_applicable, p.zone,
                  :new_rule_version)
      <> policy_eval(p.license_spdx, p.d6_applicable, p.zone,
                     :current_rule_version);
```

`policy_eval` is the sibling #6024 policy expressed as a pure
function of the three recorded inputs — which is only possible
because §2 records inputs, not verdicts. The query's output is the
work list: repos to purge, re-tier, or re-ingest under the new rule.
Deletion duty (Cluster 6: source deletion should purge the corpus
copy) reads the same table in reverse — repos whose latest
generation is a tombstone generation (`license_spdx` NULL +
`zone` unchanged + a `detection_source = manual` row citing the
deletion sync) are purge candidates.

Re-detection (detector or taxonomy upgrade) is the same shape:
rows with `detector_version < :current` are re-run through the §3
cascade _against their recorded evidence first_ (the content-hashed
notice file), and only re-fetched when evidence is missing — a
taxonomy change never requires re-crawling the web.

## 6. Decision / go-no-go

**Go** on the §2 record with the §3 cascade and the §4 flag
semantics, as a registry-resident table under #3125. Specifically:

- The design's deliverable is the field set, the cascade, and the
  §5 query shape. Implementation (schema migration in the #3125
  registry, cascade wiring at ingest, `policy_eval` in the policy
  layer) is a separate follow-on issue, filed with this record.
- No D.6 _conclusion_ is recorded anywhere; `yes` stays unemitted
  until #6024's lawyer answer lands.
- Zone assignment rules themselves belong to the sibling #6024
  policy packet; this record only reserves the field and the four
  Cluster-6 values.
- Open dependency: #5979's SPDX-expression format recommendation
  is the authority for `license_spdx` syntax; if #5979 lands a
  different format, I4's parser is the one place that changes.

## 7. Non-goals

- Deciding the #6024 policy (what each zone/license combination
  permits) — sibling packet; this record is its memory, not its
  verdict.
- Retroactive capture for already-ingested repos beyond what the
  recorded-evidence path in §5 can reconstruct; a backfill pass is
  an implementation-phase decision with its own issue if wanted.
- Per-chunk or per-comment license fields — licenses attach to
  repos (and, via D.6, are argued about comments); chunk-level
  license facts would be a different design.
