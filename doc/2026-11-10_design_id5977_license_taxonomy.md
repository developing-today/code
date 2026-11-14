# Design: license taxonomy & classifier — the enumerated set, the warning ladder, and the tag format

- **Date:** 2026-11-10
- **Status:** DECIDED (queue task 14719). Design only; no implementation is included in this change.
- **Issue:** [#5977](https://github.com/developing-today/code/issues/5977), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S82–S89 of the 2026-11-04 brief).
- **Inputs:** the combination-checker design [#5985](https://github.com/developing-today/code/issues/5985) (the license-fact interface this taxonomy must feed — `license_id`, `license_family`, `obligation_traits`, `facts_source`); the public-domain normalization decision [#6009](https://github.com/developing-today/code/issues/6009) (adopted verbatim in §4); the licensing policy design [#5984](https://github.com/developing-today/code/issues/5984) (owns what deployments _do_ with flags; this spec only classifies); the detection pipeline design [#6008](https://github.com/developing-today/code/issues/6008) (produces declared/detected facts).

## Decision summary

| Question             | Decision                                                                                                                                                                                                                                                                               |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| License value type   | An enumerated taxonomy (§2), never a free-text string. Every entry has an id, a family, an obligation-trait set, and a warning-ladder level                                                                                                                                            |
| Warning ladder       | Five levels (§3): `allowed` → `informational` → `warning` → `strong-warning` → `unsupported-by-default`. The **default support ceiling is MPL**: levels up to and including MPL's `warning` are supported by default; everything more copyleft is flagged and not supported by default |
| Public domain vs CC0 | Separate entries (§4). A bare public-domain dedication is a _declared_ value only; storage normalizes it per #6009 (CC0-1.0 for content, 0BSD for code)                                                                                                                                |
| Families             | MIT, BSD, and CC are families, enumerated entry by entry (§2) — "BSD" alone and "MIT" alone are never stored values where a variant exists                                                                                                                                             |
| Tag format           | §6: manifests carry an SPDX-style license expression over enumerated ids; the registry stores the #5985 license-fact record per artifact                                                                                                                                               |
| Adoption vs upload   | §7: the taxonomy is neutral. The hard-reject of GPL/AGPL for _our own dependencies_ (#5984) and the undecided treatment of _user-uploaded_ GPL content are policy facts recorded here without conflation — neither changes any ladder level                                            |

## 1. What the classifier produces

For every artifact (code, image, document, world, library — everything uploadable), the classifier emits one **license-fact record** in exactly the shape #5985 consumes:

| Field               | Source in this spec                                                                                                                                                                                  |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `license_id`        | An entry id from §2 (an SPDX identifier where one exists, else a taxonomy-local id)                                                                                                                  |
| `license_family`    | The entry's family from §2 (vocabulary: `permissive`, `public-domain-like`, `weak-copyleft`, `strong-copyleft`, `network-copyleft`, `noncommercial`, `share-alike-content`, `unlicensed`, `unknown`) |
| `obligation_traits` | The entry's multi-valued trait set from §2 (same vocabulary as #5985 §1; traits are not mutually exclusive)                                                                                          |
| `facts_source`      | `declared`, `detected`, or `declared+detected-agree` / `-disagree` (§5; the pipeline is #6008's)                                                                                                     |

A classifier that cannot identify a license emits the `unknown` entry (§2) — never a guess, never an empty value. Missing facts are a rule input for the combination checker, not an error in the taxonomy.

## 2. The enumerated set

Ladder levels are defined in §3. "Supported" means supported-by-default under the MPL ceiling.

### 2.1 Permissive — the allowed set (Drew's named five families, expanded)

| Entry id                                                     | Family             | Traits                         | Ladder                                                                                                                 |
| ------------------------------------------------------------ | ------------------ | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `MIT`                                                        | permissive         | permissive                     | allowed                                                                                                                |
| `MIT-0` (MIT No Attribution)                                 | permissive         | permissive                     | allowed                                                                                                                |
| `MIT-advertising` (pre-1999 MIT with the advertising clause) | permissive         | permissive                     | informational — an ancestor, distinct from `MIT`; the advertising obligation must be visible, not silently read as MIT |
| `0BSD` (BSD Zero Clause)                                     | permissive         | permissive, public-domain-like | allowed                                                                                                                |
| `BSD-2-Clause`                                               | permissive         | permissive                     | allowed                                                                                                                |
| `BSD-3-Clause`                                               | permissive         | permissive                     | allowed                                                                                                                |
| `BSD-3-Clause-Clear`                                         | permissive         | permissive                     | informational                                                                                                          |
| `BSD-4-Clause`                                               | permissive         | permissive                     | informational — the advertising-clause ancestor; not what "BSD" means in the allowed set                               |
| `Apache-2.0`                                                 | permissive         | permissive                     | allowed                                                                                                                |
| `BlueOak-1.0.0`                                              | permissive         | permissive                     | allowed                                                                                                                |
| `ISC`                                                        | permissive         | permissive                     | informational — recognized permissive, outside Drew's named set, so it is distinguished rather than silently allowed   |
| `Zlib`                                                       | permissive         | permissive                     | informational                                                                                                          |
| `Unlicense`                                                  | public-domain-like | public-domain-like, permissive | informational — a dedication with a fallback license; see §4                                                           |

The allowed set means exactly: `MIT`, `MIT-0`, `0BSD`, `BSD-2-Clause`, `BSD-3-Clause`, `Apache-2.0`, `BlueOak-1.0.0`, and `MPL-2.0` (§2.3) at its warning level. Anything else permissive-shaped is `informational`, not `allowed` — the system distinguishes each of Drew's named licenses individually, per the requirement.

### 2.2 Public domain and CC0 (see §4 for the full treatment)

| Entry id                                                      | Family             | Traits             | Ladder                                                                                                                      |
| ------------------------------------------------------------- | ------------------ | ------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| `LicenseRef-PublicDomain` (taxonomy-local; no SPDX id exists) | public-domain-like | public-domain-like | informational — a _declared_ value only; never a stored value (§4)                                                          |
| `CC0-1.0`                                                     | public-domain-like | public-domain-like | allowed for content; informational for code (the patent/trademark carve-out and the Fedora/FSF positions recorded in #6009) |

### 2.3 Weak copyleft — MPL is the ceiling

| Entry id                              | Family        | Traits        | Ladder                                                                                                                            |
| ------------------------------------- | ------------- | ------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `MPL-2.0`                             | weak-copyleft | weak-copyleft | **warning** — the highest ladder level supported by default. File-level copyleft is the most copyleft the default policy supports |
| `LGPL-2.1-only` / `LGPL-2.1-or-later` | weak-copyleft | weak-copyleft | strong-warning — beyond the ceiling: library-level copyleft with link obligations MPL does not impose                             |
| `LGPL-3.0-only` / `LGPL-3.0-or-later` | weak-copyleft | weak-copyleft | strong-warning                                                                                                                    |
| `EPL-2.0`                             | weak-copyleft | weak-copyleft | strong-warning                                                                                                                    |

### 2.4 Strong and network copyleft — each step flags harder (Drew's ladder)

| Entry id                              | Family           | Traits                            | Ladder                                                                                                                                                          |
| ------------------------------------- | ---------------- | --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GPL-2.0-only` / `GPL-2.0-or-later`   | strong-copyleft  | strong-copyleft                   | strong-warning                                                                                                                                                  |
| `GPL-3.0-only` / `GPL-3.0-or-later`   | strong-copyleft  | strong-copyleft                   | strong-warning, ranked above GPL-2.0 within the level (patent grant and anti-tivoization terms are combination-relevant facts the GPL-2.0 entry does not carry) |
| `AGPL-3.0-only` / `AGPL-3.0-or-later` | network-copyleft | strong-copyleft, network-copyleft | **unsupported-by-default** — the top of the ladder: network use triggers source obligations, which is a different operational fact from distribution            |

### 2.5 Creative Commons entries for non-code

| Entry id          | Family              | Traits                        | Ladder                                                                                                                                                                     |
| ----------------- | ------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CC0-1.0`         | public-domain-like  | public-domain-like            | allowed (see §2.2)                                                                                                                                                         |
| `CC-BY-4.0`       | permissive          | permissive                    | allowed                                                                                                                                                                    |
| `CC-BY-SA-4.0`    | share-alike-content | share-alike                   | warning — the allowed shape for share-alike content; derivatives carry the same license                                                                                    |
| `CC-BY-NC-4.0`    | noncommercial       | noncommercial                 | strong-warning — NC entries are flagged _differently_ from copyleft: the restriction is on field of use, not on derivatives; policy (#5984) owns what servers do with them |
| `CC-BY-NC-SA-4.0` | noncommercial       | noncommercial, share-alike    | strong-warning (both traits present — #5985's rules key on traits independently)                                                                                           |
| `CC-BY-ND-4.0`    | permissive          | permissive, no-derivatives    | strong-warning — no-derivatives is a reuse restriction a consumer must see before combining                                                                                |
| `CC-BY-NC-ND-4.0` | noncommercial       | noncommercial, no-derivatives | strong-warning                                                                                                                                                             |

### 2.6 The non-license slots

| Entry id                                                           | Family           | Traits                            | Ladder                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------ | ---------------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `LicenseRef-Unlicensed` (all rights reserved / no license granted) | unlicensed       | unlicensed                        | unsupported-by-default **for sharing and combination** — this is the _absence_ of a license, the opposite pole from public domain, and its own slot for exactly that reason. The brief supports it existing (the policy child decides what an uploader's own unlicensed artifact may do); the taxonomy only refuses to confuse it with anything else |
| `LicenseRef-Proprietary` (a named commercial/custom license)       | unlicensed       | unlicensed                        | strong-warning — terms exist but are not in the enumerated set; the entry records that a human must read them                                                                                                                                                                                                                                        |
| `LicenseRef-Unknown`                                               | unknown          | unknown                           | warning — identified as unidentified; the combination checker treats it as a rule input, and detection (§5) is expected to shrink this population over time                                                                                                                                                                                          |
| `SSPL-1.0`                                                         | network-copyleft | strong-copyleft, network-copyleft | unsupported-by-default — AGPL-derived, with a service-source-offering obligation broader than AGPL's; enumerated by name so it is never mistaken for `AGPL-3.0`                                                                                                                                                                                      |
| `BUSL-1.1`                                                         | noncommercial    | noncommercial                     | unsupported-by-default — source-available, not open source: the use grant excludes production use until the entry's change date, at which the artifact's fact record is re-issued under its change license (MPL-2.0 / Apache-2.0 in the common parameterizations)                                                                                    |
| `LicenseRef-CommonsClause`                                         | noncommercial    | noncommercial                     | unsupported-by-default — not a license but a _rider_: it is never a sole entry. A Commons-Clause artifact's manifest expression is `<base license> AND LicenseRef-CommonsClause` (§6 semantics: both atoms' facts apply), so the no-sale restriction is recorded alongside — never instead of — the base license's own family and traits             |

## 3. The warning ladder

Five levels, in order. The ladder is a property of the _entry_, not of a deployment; policy (#5984) decides what a server does at each level.

| Level | Name                   | Meaning                                                                                                                                        | Supported by default                                         |
| ----- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| 0     | allowed                | In Drew's named set or the CC allowed shapes; no flag                                                                                          | yes                                                          |
| 1     | informational          | Recognized and unproblematic, but distinguished (variants, ancestors, recognized permissives outside the named set, dedication-shaped entries) | yes                                                          |
| 2     | warning                | Real obligations a consumer must see: MPL's file-level copyleft, CC-BY-SA's share-alike, an unknown license                                    | yes — **MPL sits here; this is the default support ceiling** |
| 3     | strong-warning         | Beyond the ceiling: LGPL/EPL, GPL-2, GPL-3, all NC and ND content, proprietary terms                                                           | no                                                           |
| 4     | unsupported-by-default | AGPL, source-available non-licenses (SSPL/BUSL/Commons Clause), and unlicensed content _for sharing_                                           | no                                                           |

Two ladder rules:

- **Ceiling rule.** "By default we support nothing further copyleft than MPL" is level 2's upper edge: levels 0–2 are supported, 3–4 are flagged and unsupported unless the deployment's policy (admin-configurable, #5984) says otherwise. The taxonomy records the ceiling; it does not enforce it.
- **The `no-derivatives` trait (interface addition, flagged for #5985).**
  The ND entries carry a `no-derivatives` trait that is _not_ in #5985's
  v1 trait vocabulary. It is added here deliberately: without it, an ND
  asset's fact record is indistinguishable from plain `CC-BY` (or from
  `CC-BY-NC` for NC-ND), and the combination checker — which keys on
  traits, not ladder levels — would never warn on a derivative use.
  #5985's checker must treat `no-derivatives` as a gating trait
  (derivative combinations warn + exception, alongside its R3/R7
  handling); this record flags the addition explicitly rather than
  folding ND into a trait it does not mean.
- **Rank within a level.** Where the brief orders entries inside one level (GPL-2 below GPL-3 below AGPL), the order above is the display and reporting order. Combination rules key on traits, not on rank.

## 4. Public domain vs CC0 — separate entries, normalized storage (#6009 adopted)

Public domain and CC0 are **not the same object**, and the taxonomy keeps them as separate entries (§2.2):

- A **public-domain dedication** is a bare act whose validity depends on jurisdiction: in monistic author's-rights systems (Germany the standard example) copyright cannot be waived at all and moral rights are inalienable, so the dedication simply fails there; several other jurisdictions lack a dedication concept or treat it as revocable.
- **CC0** is engineered around exactly that failure: a waiver, then a fallback permissive license where the waiver fails, then a covenant not to assert. That is why CC0 works as a portable stored value and a bare dedication does not. CC0's own caveats are recorded with the entry: it does not license patent or trademark rights (the reason Fedora reclassified CC0 as not-allowed _for code_ in 2022 while keeping it fine for data/content, and the FSF does not recommend it for software).

**Storage normalization, adopted verbatim in substance from #6009:**

- A user selecting "public domain" is stored as **`CC0-1.0` for content** and **`0BSD` (or Unlicense) for code** — never as a bare "public domain" string.
- The declared value is preserved in the fact record's provenance (`facts_source = declared`, declared id `LicenseRef-PublicDomain`) so the normalization is auditable, while the registry and the combination checker see **the normalized SPDX identifier only**.
- The one sentence of UI copy #6009 requires (plain language, shown at selection): _"Public domain can't be given away everywhere — some countries don't allow it — so we record this as CC0, a dedication with a backup license that works in those places too."_ (For code, "as 0BSD" replaces "as CC0".)

## 5. Detection: declared, detected, inherited

- **Declared**: the uploader asserts an entry from §2 at upload. The picker offers entries, not text.
- **Detected**: file headers, `LICENSE` files, and package metadata are scanned (#6008 owns the pipeline). A detected id that disagrees with the declared id yields `facts_source = declared+detected-disagree` — both ids are recorded; neither silently wins.
- **Inherited**: a library's dependencies carry their own fact records. The combination checker (#5985) consumes the per-artifact records at combination events; the taxonomy's job is only that every artifact in the tree _has_ a record, with `unknown` where detection found nothing.

## 6. Tag format — manifests and registry

- **In a manifest** (world, library, package): a single field

  ```toml
  license = "MIT"
  ```

  whose value is an **SPDX-style license expression** over §2 entry ids: one id, or `A OR B` for dual licensing, or `A AND B` for multi-license artifacts. Taxonomy-local ids (`LicenseRef-*`) are valid expression atoms. No other syntax (no `+` suffix — the `-only` / `-or-later` entry ids carry that distinction; no free text; no URLs).

- **In the registry**: the manifest expression is stored verbatim as the
  artifact's `license_expression`, plus **one license-fact record per
  atom** in the expression (§1's four fields each), keyed by artifact id
  and content hash. The registry stores the _normalized_ id (§4) as each
  atom's `license_id`; the declared id survives only in provenance.
  Expression semantics for consumers (the combination checker above
  all): an `OR` expression is a _choice_ — obligations are those of the
  one branch the consumer elects, and the election is recorded at
  combination time; an `AND` expression is a _conjunction_ — every
  branch's obligations apply, evaluated per branch record, never by
  unioning trait sets into a fictitious single license. A consumer
  must never flatten an expression into one fact record: unioning an
  `OR`'s traits would make an optional branch's obligations mandatory,
  and dropping an `AND` branch would lose conditions the artifact
  actually carries.

- **Versioning of the taxonomy itself**: entries are append-only. An entry is never renamed or re-leveled in place; a correction adds a new entry and deprecates the old one, because stored fact records cite entry ids and levels historically.

## 7. Adoption-side vs upload-side (carried, not conflated)

Two different questions wear the word "GPL" in this program, and this spec keeps them apart:

- **Adoption side (decided, #5984):** for _our own dependencies_ — code the platform itself takes in — GPL/AGPL is **hard-rejected**. That is a procurement rule about what we ship, recorded here so implementers of dependency tooling can cite it.
- **Upload side (undecided, #5984):** what happens when a _user uploads_ GPL-licensed content of their own is **not decided** in #5984 and is not decided here. The taxonomy classifies it (`GPL-3.0-*`, strong-warning, §2.4) and the ladder flags it; whether a given server warns, allows, or refuses is that server's policy under #5984.

Neither fact moves any entry's ladder level. A level is a property of the license; both policies are properties of a relationship to the artifact.

## 8. Non-goals

- Policy: what any deployment does at any ladder level (#5984).
- Combination rules: which pairs may combine and how overrides work (#5985).
- Detection mechanics: scanners, header grammars, metadata parsers (#6008).
- Legal determinations about any specific artifact. The taxonomy classifies texts by their stated license; it does not adjudicate whether a given dedication or claim is effective for a given work.
