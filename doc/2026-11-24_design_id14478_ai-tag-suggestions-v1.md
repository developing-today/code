# Design spike: safe v1 for AI file management — tag suggestions only (#14478)

- **Date:** 2026-11-24
- **Status:** Design spike record (slice 1 of #14478). Implements nothing. No production code changed.
- **Issue:** [#14478](https://github.com/developing-today/code/issues/14478) (spike under parent [#6381](https://github.com/developing-today/code/issues/6381))
- **Parent reads:** #6381 (tag suggestions / NL search / metadata extraction / provider integration); [#6377](https://github.com/developing-today/code/issues/6377) (web/core boundary — CLOSED); [#1061](https://github.com/developing-today/code/issues/1061) (provider registry — CLOSED, delivered via PR #4965)
- **Overlap check:** [#13741](https://github.com/developing-today/code/issues/13741), [#13742](https://github.com/developing-today/code/issues/13742), [#13743](https://github.com/developing-today/code/issues/13743) are creator-tooling phase-14 slices under #6309 — unrelated to AI file management; no overlap, no dependency.
- **Code read for this record:** `pkgs/id/book/reference/data-stores.md` (file = name → immutable blob hash + TagStore metadata), `pkgs/id/ARCHITECTURE.md` (FileOps / TagStore / `tag` command), `pkgs/id/crates/id-native-shell/src/providers.rs` (`client.file.read`: user-selected root, 32 KiB bound, `disclosure_acknowledged` gate), `pkgs/id/crates/id-caps/src/world_caps.rs` (capability manifest + ceilings registry, not a model catalog).

## Decision

**NO-GO for AI inference in v1; GO for one bounded design direction: AI-assisted tag suggestions.**

Choose exactly one capability from #6381's four (tag suggestions, natural-language search, metadata extraction, provider integration): **tag suggestions**. Do not include natural-language search, metadata extraction, or any provider/plugin framework in v1. This matches the exclusion list on #14478 (no production implementation, no file mutation, no provider/plugin framework, no broad architecture changes, no new model/provider integration).

## 1. Target subsystem and the one v1

Target `pkgs/id` FileOps and TagStore. A user-visible file is a human-readable name/tag reference to an immutable Iroh blob hash, with arbitrary key/value tags (`pkgs/id/book/reference/data-stores.md`; `pkgs/id/ARCHITECTURE.md`, Data Model / Metadata Tags).

The v1 suggestion surface, when it ever exists, shows candidate tags for **one explicitly named file** plus its current tags. It does not rename, replace, rewrite blob contents, crawl directories, or batch-tag a library. Accepted tags use the existing TagStore operation (`set_tag` / `set_if_absent` semantics — existing tags never overwritten); suggestions stay ephemeral until the user reviews and explicitly accepts each tag.

Out of scope for v1: natural-language search queries, automatic metadata extraction pipelines, user-configurable AI settings pages, and any AI provider plugin system.

## 2. Data and safety boundary

A future inference request may contain **only** one explicitly selected file's bounded content plus its name and current tags. Never crawl a directory, scan all files, infer from peer/world data, or transmit by default.

The existing native-shell `client.file.read` is a useful minimum-disclosure pattern but **not** authorization to send blob data to a model: it is read-only and opt-in per world, constrained beneath a user-selected root, bounded (32 KiB default, rate-limited), and refuses unless the user acknowledges that returned bytes are journaled (`pkgs/id/crates/id-native-shell/src/providers.rs`, `disclosure_acknowledged` / `REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED`). Any future inference call needs its own explicit per-use disclosure/consent and auditable data handling on top of this.

Failure semantics (no exceptions): missing content, missing provider, timeout, malformed output, or quota/privacy refusal means **no suggestion and no metadata change**. No autonomous writes, deletes, or replacements — ever in v1.

## 3. Provider-registry interface (what it is and is not)

#1061 / PR #4965 delivered the ID capability registry (`pkgs/id/crates/id-caps/src/world_caps.rs`): it registers capability providers/manifests and enforces scopes/ceilings. It is **not** a model catalog, inference API, or permission to add a parallel AI plugin system.

V1 must **not** route file content through the capability registry as though it were a model provider. Revisit only after an existing, approved inference provider/API can be called through the project's provider-routing contract, with explicit per-use disclosure/consent and auditable data handling; any new capability must be declared in the existing registry and remain excluded by default until granted.

## 4. Risks / go-no-go

- **Primary risk:** disclosing private file contents to a remote model. Contained by: single-file scope, bounded content, explicit per-use consent, ephemeral suggestions, no default transmission.
- **Secondary risk:** tag hallucinations. Contained by per-tag user approval over existing `set_if_absent` TagStore semantics.
- **GO:** retain this one-capability design direction as the safe v1 candidate.
- **NO-GO:** implementation, model calls, or any follow-up implementation issue until provider choice and disclosure policy are established. Filing an implementation issue now would make the unresolved provider/privacy decision look settled.
- **No production code or user files changed by this record.**

## 5. After this record

The next slice (if any) is a second design note that names the approved inference provider/API and its disclosure/consent contract — still no code. Code lands only after that contract is reviewed and the provider/privacy decision is recorded as settled on #14478.
