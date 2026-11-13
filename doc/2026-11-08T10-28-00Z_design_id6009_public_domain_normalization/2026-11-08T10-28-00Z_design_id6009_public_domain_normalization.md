# Public-domain storage normalization — CC0-1.0 for content, 0BSD for code

- **Date:** 2026-11-08
- **Status:** design note (rule + UI copy); docs-only, no implementation.
- **Issue:** [#6009](https://github.com/developing-today/code/issues/6009), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955); parent taxonomy design [#5977](https://github.com/developing-today/code/issues/5977).
- **Why a note and not a section of #5977's spec:** the #5977 taxonomy spec does not exist yet. At `origin/main` `561f413226` the only landed licensing design record is [the combination checker](2026-11-06_design_id5985.md) (#5985), which _consumes_ #5977's facts rather than defining them, and no `CC0-1.0` / `0BSD` / public-domain normalization exists in `pkgs/id/src`, `pkgs/id/contract`, or `doc/`. This note is the rule #5977's spec must adopt on day one so it cannot be finalised without it.

## 1. The rule

**A user selecting "public domain" is never stored as the string "public domain".** It is stored as a real, machine-readable dedication, chosen by the artifact's kind:

| Artifact kind                                                | Stored SPDX identifier            | Why                                                                                                                                                                                                                                                                                                                                                              |
| ------------------------------------------------------------ | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Content — images, audio, video, documents, world data/assets | `CC0-1.0`                         | CC0 is the only widely recognised dedication engineered for jurisdictions where a bare dedication fails: waiver first, permissive fallback licence if the waiver is unenforceable, then a covenant not to assert.                                                                                                                                                |
| Code — libraries, DLLs, worlds, source                       | `0BSD` (preferred) or `Unlicense` | CC0 is expressly not recommended for software (FSF) and Fedora reclassified it as not-allowed for _code_ in 2022 because CC0 does not grant patent rights. `0BSD` is an OSI-approved permissive licence with no attribution requirement — the closest legal equivalent to a dedication — and `Unlicense` is the accepted alternative where an author prefers it. |

Two consequences, both normative:

- **The bare string is refused at the boundary.** A declared licence of `public domain`, `Public Domain`, `PD`, `public-domain`, or `CC0` on a code artifact does not get normalised silently at read time; the **write** path normalises it, so the registry never holds the ambiguous value. This is a storage rule, not a display preference.
- **Normalization is by artifact kind, not by licence family.** The same human intent maps to two different identifiers depending on whether the bytes are content or code. A single global mapping is wrong in one direction or the other, which is why the kind must be known at the point of storage.

## 2. What the registry and the combination checker see

**Only the normalized SPDX identifier.** No component downstream of storage may see "public domain" as a value:

- **Registry / manifests:** the licence field holds `CC0-1.0`, `0BSD`, or `Unlicense`. `license_id` — the fact-record field the combination checker consumes ([#5985 §1](2026-11-06_design_id5985.md)) — is therefore always one of those or another taxonomy id, never free text.
- **Combination checker:** rules key on the taxonomy entry and its `obligation_traits`. A stored `CC0-1.0` content artifact and a stored `0BSD` code artifact each resolve to their own entry, so the checker needs no "public domain" case and should not grow one.
- **Display:** the UI may render "Public domain" as the label for either identifier — that is presentation. Storage, comparison, audit records and export all carry the SPDX id.

## 3. The UI copy (one sentence)

> **"Public domain" is recorded as CC0-1.0 for content and 0BSD for code, so your dedication is understood everywhere — note that neither gives away patent or trademark rights, and a bare dedication is not valid in every country.**

Both caveats the issue names are in the sentence on purpose: the patent/trademark gap (why code gets `0BSD` rather than CC0) and non-portability (why the bare string is not kept). It states what happens, not merely that something happens.

## 4. What this does not change

[#5977](https://github.com/developing-today/code/issues/5977) keeps **public domain and CC0 as separate taxonomy entries**. This note decides storage normalization only:

- The taxonomy still holds a public-domain entry and a CC0 entry, with their own ladder levels and traits. Collapsing them would lose the distinction the brief requires ("it must distinguish public domain… because that matters for some countries").
- The mapping is one-way at the boundary: `public domain → CC0-1.0 | 0BSD` on write. Nothing maps `CC0-1.0` back to "public domain", and an author who explicitly selects CC0 for code keeps CC0 — the normalization fires only on the ambiguous selection.
- The warning ladder is unaffected: neither `CC0-1.0` nor `0BSD` is copyleft; both sit in the allowed band.

## 5. Open point for the taxonomy spec

Where the artifact-kind decision is made (registry schema field, upload flow, or classifier output) is #5977's to settle. This note fixes only that the kind is known **before** storage and that the mapping is total for the two kinds — a third kind added later must state its own normalization rather than inheriting one by default.

## References

- [#6009](https://github.com/developing-today/code/issues/6009) — this deliverable.
- [#5977](https://github.com/developing-today/code/issues/5977) — license taxonomy & classifier; the spec this rule is adopted into.
- [#5985](https://github.com/developing-today/code/issues/5985) and [its design record](2026-11-06_design_id5985.md) — the combination checker that consumes the normalized identifier.
- [#6008](https://github.com/developing-today/code/issues/6008) — license detection pipeline (declared vs detected).
- CC0 jurisdiction survey: <https://Wiki.CreativeCommons.org/images/6/69/CC0_Jurisdiction_Survey.pdf>
- Fedora's 2022 reclassification of CC0 as not-allowed-for-code: <https://news.ycombinator.com/item?id=30556049>
