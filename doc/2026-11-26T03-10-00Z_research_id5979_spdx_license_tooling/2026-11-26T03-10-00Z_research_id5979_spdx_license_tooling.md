# id: SPDX identifiers and license-detection tooling for the classifier (#5979)

Research packet for [#5979](https://github.com/developing-today/code/issues/5979),
part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
It supplies the evidence that three decided or drafted designs left open:

- the license taxonomy and classifier, [#5977](https://github.com/developing-today/code/issues/5977)
  ([`doc/2026-11-10_design_id5977_license_taxonomy.md`](../2026-11-10_design_id5977_license_taxonomy.md)),
  which fixed the enumerated set and the SPDX-style tag format;
- the detection pipeline, [#6008](https://github.com/developing-today/code/issues/6008)
  ([`doc/2026-11-05T17-24-09Z_design_id_license_detection_pipeline/`](../2026-11-05T17-24-09Z_design_id_license_detection_pipeline/2026-11-05T17-24-09Z_design_id_license_detection_pipeline.md)),
  which left scanner choice, expression grammar, confidence semantics and
  budgets "until #5979 supplies evidence";
- the public-domain normalization, [#6009](https://github.com/developing-today/code/issues/6009).

Status: research and recommendation only. No code, and no change to any
decided design. Where a finding bears on a decided record, §8 lists it as a
flag for that record's owner; the taxonomy is append-only (#5977 §6), so
none of the flags edits an entry in place.

Evidence date: 2026-11-26. The SPDX data was read from the
`spdx/license-list-data` repository's `main` branch (745 license ids, 86
exceptions; latest release tag `v3.29.0`). Every other claim names its
source in §10.

## 1. Recommendation

1. **Identifiers.** Every non-local entry in #5977 §2 is a current,
   non-deprecated SPDX License List id. Store SPDX ids in their reference
   case, and record the license-list version that classified each fact
   record (§2).
2. **Expression format.** Accept the SPDX 2.3 expression grammar on input,
   store one canonical form, and keep the raw input beside it (§3). The
   canonical form is: reference-case ids, upper-case `AND` / `OR` / `WITH`,
   no `+` (rewritten to the `-or-later` id where one exists, refused where
   none does), no `DocumentRef-`, and `LicenseRef-` atoms only from the
   taxonomy.
3. **Detection.** Two tiers (§4, §7). Tier 1 runs in process on every
   upload: the Rust `spdx` crate (expression parser plus its `detection`
   feature for full license texts) and a REUSE-convention reader for
   `SPDX-License-Identifier:` headers, `LICENSES/` and `REUSE.toml`. Tier 2
   is ScanCode as an optional, out-of-process deep scan that a server
   enables for published libraries and worlds. licensee is not recommended:
   it is Ruby and its corpus is choosealicense.com's short list, so most of
   the taxonomy's variants can only come back as no-match.
4. **Thresholds.** Start strict, so a near-variant falls to
   `LicenseRef-Unknown` rather than to its base license, which is #5977's
   "never a guess" rule. Settle the number with the benchmark in §7, not
   by adopting a tool's default (cargo-deny's is 0.8).
5. **Manifest field.** In KDL manifests, `license` is one node with one
   string argument holding the canonical expression, plus optional
   repeatable `license-file` nodes (§6).

## 2. SPDX identifier table for the #5977 set

Checked against `license-list-data` `main` on 2026-11-26. "OSI" and "FSF"
are the list's `isOsiApproved` and `isFsfLibre` flags; they are facts about
the texts, not inputs to the ladder.

| #5977 entry                   | On the SPDX list | SPDX full name                                                       | OSI | FSF libre |
| ----------------------------- | ---------------- | -------------------------------------------------------------------- | --- | --------- |
| `MIT`                         | yes              | MIT License                                                          | yes | yes       |
| `MIT-0`                       | yes              | MIT No Attribution                                                   | yes | no        |
| `MIT-advertising`             | yes              | Enlightenment License (e16)                                          | no  | no        |
| `0BSD`                        | yes              | BSD Zero Clause License                                              | yes | no        |
| `BSD-2-Clause`                | yes              | BSD 2-Clause "Simplified" License                                    | yes | yes       |
| `BSD-3-Clause`                | yes              | BSD 3-Clause "New" or "Revised" License                              | yes | yes       |
| `BSD-3-Clause-Clear`          | yes              | BSD 3-Clause Clear License                                           | no  | yes       |
| `BSD-4-Clause`                | yes              | BSD 4-Clause "Original" or "Old" License                             | no  | yes       |
| `Apache-2.0`                  | yes              | Apache License 2.0                                                   | yes | yes       |
| `BlueOak-1.0.0`               | yes              | Blue Oak Model License 1.0.0                                         | yes | no        |
| `ISC`                         | yes              | ISC License                                                          | yes | yes       |
| `Zlib`                        | yes              | zlib License                                                         | yes | yes       |
| `Unlicense`                   | yes              | The Unlicense                                                        | yes | yes       |
| `CC0-1.0`                     | yes              | Creative Commons Zero v1.0 Universal                                 | no  | yes       |
| `MPL-2.0`                     | yes              | Mozilla Public License 2.0                                           | yes | yes       |
| `LGPL-2.1-only` / `-or-later` | yes              | GNU Lesser General Public License v2.1 only / or later               | yes | yes       |
| `LGPL-3.0-only` / `-or-later` | yes              | GNU Lesser General Public License v3.0 only / or later               | yes | yes       |
| `EPL-2.0`                     | yes              | Eclipse Public License 2.0                                           | yes | yes       |
| `GPL-2.0-only` / `-or-later`  | yes              | GNU General Public License v2.0 only / or later                      | yes | yes       |
| `GPL-3.0-only` / `-or-later`  | yes              | GNU General Public License v3.0 only / or later                      | yes | yes       |
| `AGPL-3.0-only` / `-or-later` | yes              | GNU Affero General Public License v3.0 only / or later               | yes | yes       |
| `CC-BY-4.0`                   | yes              | Creative Commons Attribution 4.0 International                       | no  | yes       |
| `CC-BY-SA-4.0`                | yes              | Creative Commons Attribution Share Alike 4.0 International           | no  | yes       |
| `CC-BY-NC-4.0`                | yes              | Creative Commons Attribution Non Commercial 4.0 International        | no  | no        |
| `CC-BY-NC-SA-4.0`             | yes              | … Non Commercial Share Alike 4.0 International                       | no  | no        |
| `CC-BY-ND-4.0`                | yes              | … No Derivatives 4.0 International                                   | no  | no        |
| `CC-BY-NC-ND-4.0`             | yes              | … Non Commercial No Derivatives 4.0 International                    | no  | no        |
| `SSPL-1.0`                    | yes              | Server Side Public License, v 1                                      | no  | no        |
| `BUSL-1.1`                    | yes              | Business Source License 1.1                                          | no  | no        |
| `LicenseRef-PublicDomain`     | no (local)       | —                                                                    | —   | —         |
| `LicenseRef-Unlicensed`       | no (local)       | —                                                                    | —   | —         |
| `LicenseRef-Proprietary`      | no (local)       | —                                                                    | —   | —         |
| `LicenseRef-Unknown`          | no (local)       | —                                                                    | —   | —         |
| `LicenseRef-CommonsClause`    | no (local)       | Commons Clause is on neither the license list nor the exception list | —   | —         |

Findings from the table:

- **No entry needs a new id.** All 29 SPDX-backed entries exist and none is
  deprecated. The five `LicenseRef-` entries are correctly local: SPDX has
  no id for a bare public-domain dedication, for "no license", for "unknown"
  or for the Commons Clause.
- **Deprecated forms that detection and import will meet.** The list keeps
  `GPL-2.0`, `GPL-3.0`, `LGPL-2.1`, `AGPL-3.0` and `GPL-2.0+` as deprecated
  ids. The list itself names `GPL-2.0` "GNU General Public License v2.0
  only" and `GPL-2.0+` "… v2.0 or later", so canonicalization is mechanical:
  a bare deprecated GNU id becomes `-only`, a `+` form becomes `-or-later`,
  and the declared spelling survives in provenance.
- **`MIT-advertising` is the Enlightenment (e16) license.** Its text is the
  MIT grant plus a requirement that the notice appear in "marketing &
  publicity materials" and that "acknowledgment shall be given … that this
  Software was used". #5977 glosses the entry as "pre-1999 MIT with the
  advertising clause"; the classification (permissive, informational,
  visible advertising obligation) holds, but the gloss misnames the text's
  origin. See §8.
- **Public-domain-shaped ids SPDX does list.** Besides `CC0-1.0`,
  `Unlicense` and `0BSD`, the list has `CC-PDDC` (CC Public Domain
  Dedication and Certification), `CC-PDM-1.0` (Public Domain Mark) and
  `PDDL-1.0` (Open Data Commons dedication). Detection will meet them on
  imported content. Under #6009 they are declared values to normalize, not
  stored values; see §8.

## 3. Expression grammar

SPDX 2.3 (Annex D) defines the grammar #5977 §6 calls "SPDX-style":

- **Atoms.** A license id, `LicenseRef-<idstring>`, optionally prefixed
  `DocumentRef-<idstring>:`. An idstring is letters, digits, `-` and `.`.
- **Operators.** `+` (suffix: this version or later; no whitespace before
  it), `WITH` (attaches an exception from the SPDX exception list), `AND`,
  `OR`, and parentheses. Default binding, tightest first: `+`, `WITH`,
  `AND`, `OR`, so `A OR B AND C` is `A OR (B AND C)`.
- **Case.** In 2.3, operators are case-sensitive and ids match
  case-insensitively, but the reference case should be used. SPDX 3.0 makes
  the whole expression case-insensitive and adds `AdditionRef-` as a
  user-defined right-hand side for `WITH`.

What the canonical stored form should do with each feature, given #5977:

| Feature                   | Canonical form                                                            | Why                                                                                     |
| ------------------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Id case                   | Reference case from the list (`Apache-2.0`, not `apache-2.0`)             | Matches PEP 639's "case-normalized" storage rule and SPDX's own advice                  |
| Operator case             | Upper case                                                                | Valid under both 2.3 (required) and 3.0 (any case)                                      |
| `+`                       | Rewritten to the `-or-later` id where the list has one; otherwise refused | #5977 §6 forbids `+`; the `-only` / `-or-later` ids carry that distinction              |
| Deprecated ids            | Rewritten as in §2; declared spelling kept in provenance                  | Same normalize-and-keep-provenance rule #5977 §4 uses for public domain                 |
| `DocumentRef-`            | Refused                                                                   | It points into another SPDX document; a manifest has none to point at                   |
| `LicenseRef-`             | Only the taxonomy's own local ids                                         | Anything else is free text in disguise, which #5977 forbids                             |
| `WITH`                    | Parsed and kept; see below                                                | Real dependencies use it                                                                |
| `/` (crates.io legacy OR) | Read as `OR` on import only, never accepted in a manifest                 | Cargo documents `/` as deprecated; the `spdx` crate's lax mode reads it as `OR`         |
| Parentheses               | Kept where they change meaning, dropped where they do not                 | One canonical string per meaning, so equality of stored expressions means equal meaning |

**`WITH` needs a decision the taxonomy has not made.** #5977 enumerates no
exceptions, but exceptions occur in exactly the dependency trees this
platform imports: `Apache-2.0 WITH LLVM-exception` is common in the Rust
WebAssembly stack, and `GPL-2.0-only WITH Classpath-exception-2.0` in Java.
Both exceptions are on the SPDX exception list. Until #5977 adds
exception-bearing entries, the honest reading is: keep the raw expression,
and emit `LicenseRef-Unknown` for the `WITH` atom, so the combination
checker sees an unidentified license rather than the base license without
its exception. Flagged in §8.

SPDX's special values `NONE` and `NOASSERTION` appear in SPDX documents and
SBOMs, not in manifests. On import, `NONE` (the author asserts no license)
maps to `LicenseRef-Unlicensed` and `NOASSERTION` maps to
`LicenseRef-Unknown`.

## 4. Detection tooling

| Tool                           | Language, form                  | Evidence it reads                                                                                                     | What it reports                                                                                                                                       | Behaviour on variants and near-misses                                                                                                                                      |
| ------------------------------ | ------------------------------- | --------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `spdx` crate (0.13.x)          | Rust library                    | Expressions (strict or lax parse); with the `detection` feature, whole texts and SPDX license headers                 | Parsed expression tree; candidate license with a confidence                                                                                           | Bound by the caller's threshold. Matching against license texts can only tell a variant from its base if the threshold is strict enough to separate them                   |
| cargo-deny                     | Rust CLI; uses the `spdx` crate | `Cargo.toml` `license` and `license-file`, LICENSE texts                                                              | Pass/fail against an allow list; `clarify` pins an expression to license-file hashes                                                                  | `confidence-threshold` defaults to 0.8. A crate without machine-readable license data counts as unlicensed; the old `unlicensed` setting was removed                       |
| licensee                       | Ruby gem (GitHub's detector)    | License files, package-manager files (gemspec, npm/bower, Cargo, Cabal, CRAN, Dist::Zilla, NuGet), README references  | One license key with a confidence                                                                                                                     | Exact match, then Sørensen–Dice similarity with a 98% default threshold. Corpus is choosealicense.com's list, so variants outside it come back unmatched rather than wrong |
| ScanCode toolkit               | Python CLI                      | Every file's text, against a large index of license texts and rules (hash, Aho-Corasick and sequence matchers)        | `license_detections` with `detected_license_expression` and `detected_license_expression_spdx`; per-match `score`, `match_coverage`, `rule_relevance` | Weak results are reported as `license_clues`, not detections. Licenses absent from SPDX come out as `LicenseRef-scancode-*`. Heaviest of the four by far                   |
| REUSE (spec 3.3, `reuse` tool) | A convention plus a linter      | `SPDX-License-Identifier:` and `SPDX-FileCopyrightText:` headers, `.license` sidecar files, `REUSE.toml`, `LICENSES/` | Per-file declared expression and copyright; lint for missing or unused licenses                                                                       | Not a detector: it reads declarations exactly. Nothing to mis-detect, and nothing found where nothing was declared                                                         |

What this means for #6008:

- **Declared evidence comes first and is cheap.** REUSE headers, a
  manifest's `license` node and package metadata are declarations, not
  detections. They give `facts_source = declared` without any text
  matching. REUSE 3.3's `REUSE.toml` (version 1, `[[annotations]]` with
  `path`, and a `precedence` of `closest`, `aggregate` or `override`) is the
  one standard way to declare licenses for files that cannot carry a
  comment, such as images and data files. That fits worlds and libraries,
  which mix code and assets. DEP5 is deprecated in 3.3 and mutually
  exclusive with `REUSE.toml`.
- **Text matching is the detected side.** Only the `spdx` crate is
  in-process Rust; licensee and ScanCode would each add a language runtime
  to the server. ScanCode's per-file, per-rule evidence is the richest, and
  its clue/detection split is the false-positive behaviour #5977 wants.
  Its cost makes it a deep-scan tier, not a per-upload step.
- **The evidence fields #6008 left open** follow from the tools' outputs:
  tool name and version, SPDX license-list version, matcher kind (declared
  header, manifest, full-text match, rule match), the file path, the score,
  and for full-text matches the coverage. ScanCode's `score` and
  `match_coverage` and the crate's confidence are not on one scale, so the
  fact record should carry the tool next to the number and never compare
  scores across tools.

## 5. Public domain in SPDX

SPDX has no identifier for a bare "public domain" statement. A document
that wants to say "public domain" must use a `LicenseRef-` with the text,
which is what #5977 §2.2 does with `LicenseRef-PublicDomain`. The listed
public-domain-shaped ids are `CC0-1.0`, `Unlicense`, `0BSD`, `CC-PDDC`,
`CC-PDM-1.0` and `PDDL-1.0`. This supports #6009's choice: storing `CC0-1.0`
for content and `0BSD` for code keeps every stored value an SPDX id that
tools outside the platform understand, and leaves "public domain" only in
provenance.

## 6. How registries handle the license field

| Registry  | Field                                                                       | Accepted values                                                                                                                           | Validation                                                                                                                                    | "No license" and custom terms                                                                                                             |
| --------- | --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| crates.io | `license` or `license-file` in `Cargo.toml` (one required)                  | An SPDX expression (Cargo's reference says it is read as SPDX 2.1); `/` as OR is deprecated                                               | Publish rejects an unknown or invalid expression (secondary source; Cargo's own check was historically only presence)                         | `license-file` points at the terms when no SPDX id fits                                                                                   |
| npm       | `license` in `package.json`                                                 | An SPDX expression, or the npm-only values `UNLICENSED` and `SEE LICENSE IN <file>`; `validate-npm-package-license` rejects `LicenseRef-` | Warns with a suggestion on near-miss ids                                                                                                      | `UNLICENSED` means no rights granted. It is one letter from the SPDX id `Unlicense`, a public-domain dedication with the opposite meaning |
| PyPI      | `License-Expression` and `License-File` (core metadata 2.4, PEP 639, Final) | An SPDX expression (2.2 grammar or later), `LicenseRef-` allowed                                                                          | PyPI MUST reject uploads whose `License-Expression` is not a valid, case-normalized expression, and for 2.4 MUST reject missing license files | License classifiers are deprecated; custom terms use `LicenseRef-` plus a `License-File`                                                  |

Import mappings this implies (for #6008's importers):

| Seen in a registry            | Taxonomy atom                                     | `facts_source`                                   |
| ----------------------------- | ------------------------------------------------- | ------------------------------------------------ |
| An SPDX expression            | Canonicalized per §3                              | declared                                         |
| npm `UNLICENSED`              | `LicenseRef-Unlicensed`                           | declared                                         |
| npm `SEE LICENSE IN <file>`   | `LicenseRef-Proprietary`, then detect `<file>`    | declared, then declared+detected-agree/-disagree |
| crates.io `license-file` only | Detect the file; `LicenseRef-Unknown` if no match | detected                                         |
| Cargo `A/B`                   | `A OR B`                                          | declared                                         |

The `UNLICENSED` / `Unlicense` pair is the costliest confusion in this table:
read the wrong way, an all-rights-reserved package becomes public domain.
Importers should match `UNLICENSED` exactly and before any case-insensitive
id lookup.

## 7. Detection plan and benchmark for #6008

**Tier 1, every upload, in process.**

1. Read declarations: the manifest `license` node, REUSE headers, `.license`
   sidecars, `REUSE.toml`, and package metadata for imported packages.
2. Parse each declaration with the `spdx` crate in strict mode; on failure
   retry in lax mode, record that the input needed it, and canonicalize
   (§3).
3. Run the crate's text detection on `LICENSE*`, `COPYING*` and
   `LICENSES/*` files only, not on every file.
4. Compare declared with detected and set `facts_source` as #5977 §5 says.

**Tier 2, optional, out of process.** A server that publishes libraries or
worlds to others can enable a ScanCode pass per content hash, with its
`detected_license_expression_spdx` and per-match evidence stored beside the
tier-1 record. It never overrides a tier-1 declaration; disagreement is
recorded as `declared+detected-disagree`.

**Benchmark to settle the threshold and #6008's budgets.** A corpus built
from the SPDX texts of every #5977 entry plus its nearest variants:

- `MIT` vs `MIT-0` vs `MIT-advertising`;
- `BSD-3-Clause` vs `BSD-3-Clause-Clear` vs `BSD-4-Clause`;
- the six CC 4.0 entries against each other;
- `GPL-*-only` vs `-or-later` (identical texts; only headers differ, so
  text detection must return the version without the qualifier and leave
  `-only` / `-or-later` to declared evidence);
- each text with a changed copyright line and reflowed whitespace.

The pass mark is zero cross-variant matches: a variant may come back as
itself or as `LicenseRef-Unknown`, never as its neighbour. Pick the lowest
threshold that meets that, and measure time per file at it for the budgets.

## 8. Flags for other records

None of these changes a decided entry in place.

1. **#5977, `MIT-advertising` gloss.** The SPDX id names the Enlightenment
   (e16) license, not a pre-1999 MIT. The ladder level and family still fit;
   only the description is wrong. Under the append-only rule this is a note,
   since the id and level do not change.
2. **#5977, exceptions.** Consider exception-bearing entries, at least
   `Apache-2.0 WITH LLVM-exception` (the exception only adds permissions),
   so `WITH` atoms in real dependency trees stop classifying as unknown.
3. **#5977, source-available ids detection will meet.** The list has
   `Elastic-2.0`, `PolyForm-Noncommercial-1.0.0`,
   `PolyForm-Small-Business-1.0.0` and `FSL-1.1-MIT`. Today they would
   classify as `LicenseRef-Unknown`; enumerating them next to `BUSL-1.1`
   would let the ladder flag them by name.
4. **#6009, other public-domain ids.** `CC-PDDC`, `CC-PDM-1.0` and
   `PDDL-1.0` need a normalization line, as a bare dedication has. A Public
   Domain Mark labels a work as already free of copyright; it is not a
   grant, so it fits the declared-only treatment.
5. **#6008, open items.** §4 and §7 answer scanner choice, expression
   grammar and evidence fields. The budgets stay provisional until the §7
   benchmark runs.

## 9. Non-goals

- Choosing ladder levels or policy for any license (#5977, #5984).
- Combination rules (#5985).
- Legal views on whether any dedication or license is effective.
- Implementing the pipeline; that is #6008's.

## 10. Sources

- SPDX License List data, `spdx/license-list-data` (`json/licenses.json`,
  `json/exceptions.json`, `text/MIT-advertising.txt`, tags up to
  `v3.29.0`), read 2026-11-26.
- SPDX specification 2.3.1, Annex D, license expressions:
  <https://spdx.github.io/spdx-spec/v2.3.1/SPDX-license-expressions/>
- SPDX specification 3.0, license expressions annex:
  <https://spdx.github.io/spdx-spec/v3.0-dev/annexes/spdx-license-expressions>
- REUSE Specification 3.3 (2024-11-14), source in `fsfe/reuse-website`:
  <https://reuse.software/spec-3.3/>
- PEP 639, Improving License Clarity with Better Package Metadata (Final):
  <https://peps.python.org/pep-0639/>
- Cargo manifest reference, `license` and `license-file`.
- `validate-npm-package-license` README and npm/npm PR #11215.
- cargo-deny `[licenses]` configuration:
  <https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html>
- `spdx` crate 0.13.4 documentation (`ParseMode`, `detection` feature):
  <https://docs.rs/spdx/latest/spdx/lexer/struct.ParseMode.html>
- licensee API documentation: <https://www.rubydoc.info/gems/licensee/Licensee>
- ScanCode toolkit, license detection updates:
  <https://scancode-toolkit.readthedocs.io/en/latest/explanation/scancode-license-detection-updates.html>
