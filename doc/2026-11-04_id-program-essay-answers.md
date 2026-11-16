# Answers to Drew's 2026-11-04 ID program questions

Questions from Drew's 2026-11-04 essay; answers grounded in the cited issues and tree at `origin/main` `af0847d0`.

## 1. #2819 — Native-spill write lease: what it is

When a world gets too hot for its hosted Cloudflare Durable Object (DO), execution can spill over to a native `id` node — but the DO keeps the journal of record. The write lease is the rule that decides, at any moment, exactly one of them is allowed to write. There is never a moment when both can accept writes.

The lease itself lives in the journal, the DO is the only grantor, and fencing is an epoch check on journal append. A lease is recorded as journal entries — `LeaseGranted{holder, epoch, term, granted_at_position}`, `LeaseRenewed`, `LeaseRevoked`, and `LeaseHandedOff{from, to, epoch}` — and the epoch is monotonic and never resets. If a stale holder tries to append with an old epoch, that append is rejected, and the attempt itself is journaled as an audit entry; it is never applied.

Handoff is ordered: the DO appends `LeaseHandedOff{epoch+1}` as its last act as writer, and the new holder appends only after replaying the journal through the handoff position. Commands in flight at the boundary are refused with a retryable not-holder error — never executed by the old holder, and never silently re-executed. The recommended defaults are a 30-second term with renewal at 10 seconds.

_Citations: #2819 (OPEN, "id: design: native-spill write lease protocol for hot worlds (from #2059)"): "the journaled single-writer lease is the whole safety design — split-brain (DO and native node both accepting writes) is the catastrophic failure. The lease protocol itself was not designed. This issue designs it." Design record comment (2026-11-01): "The lease lives in the journal itself, the DO is the only grantor, and fencing is an epoch check on journal append." Design doc `doc/2026-11-01_design_id2819.md`, PR #3857 merged; implementation phased in #3803 (closed) / #3804 / #3805. Footer: epic #2056, parent #2059, consumer #2046._

## 2. #2898 — Reconstruction is admission

Any time `pkgs/id` rebuilds authority-bearing state from stored or supplied data, that rebuild is treated as a fresh admission decision — not as a trusted replay. The rebuilding code validates the same invariants the live path enforces at creation, and it refuses data that violates them. It never partially applies such data, and it never silently repairs it.

Three corollaries follow. Validation is against state you hold, not claims the input makes about itself. Unnamed means refused. And the system shape — sequence 0 / participant 0 — is not borrowable by authority-carrying entries.

This one rule unifies six filings as "one defect with six addresses": Phase 1, journal restore validates authors/scopes; Phase 2, attestation agreement validity; and Phase 3, derive-don't-trust plus a closing audit.

_Citations: #2898 (OPEN, epic + kind/design), recorded rule: "reconstruction is admission. Anywhere pkgs/id rebuilds authority-bearing state from stored or supplied data, the rebuilding code validates the same invariants the live path enforces at creation, and refuses (never partially applies, never silently repairs) data that violates them." Phase 1: #2894 + #2890 — restore's `Committed` arm checked only sequence continuity, and the `Issued` arm was never scoped. Phase 2: #2863 (`players=[A,A]` computing `Agreed` from one key); #2868 (free-form winner naming a third party). Phase 3: #2882 (caller-supplied `replacing: bool` admitting a 65th schedule); #2876 (manifest serde without `deny_unknown_fields`). Corrections of record (2026-10-31): participant 0 IS a legitimate `Committed` author (rule: 0, or issued earlier and not revoked); the attestation outcome domain includes the "draw" sentinel. Design doc: `files/designs/code-id-reconstruction-validation.md`._

## 3. #3231 — One receipt contract, and #3232 — what's audited

There is supposed to be one contract for receiving a signed statement, and every kind of signed statement is supposed to follow it. `pkgs/id` has three signed-statement types. Two of them — `documents.rs` and `chain.rs` — both state that builder and decoder apply the same contract ("a signed document always passes them on receipt"). The third, `artifact.rs`, never applied that contract to itself: its receipt gates, `check()` and `parsed()`, each omit rules — one omission per field — which produced filings #3146, #3187, #3192, and #3196.

The contract has three rules:

1. **Receipt is construction.** `parsed()` refuses a known-kind body that no in-tree producer could build; `check()` refuses an envelope no legitimate signer could build. Unknown kinds are untouched — stored, never applied.
2. **A signed list names each thing once.** Duplicates are refused, never deduped on receipt, because signed bytes are evidence.
3. **Text in a signed body follows the crate's text rules.** NFC; `world_id` `MAX_WORLD_ID_BYTES=64`, no Cc/Cf characters; requests ≤ `MAX_FRIEND_REQUESTS=3`, each exactly 32 lowercase hex.

#3232 is Phase 3 of #3231, and it is an audit only — disposition, not fixes. It produces a field-by-field table for `artifact.rs`: every field of every known kind (`friend_remove`, `home_declared`, `document`, `tag`) plus every envelope field (`author`, `seq`, `prev`, `kind`, `body`, `after`, `hash`, `signature`), checked against the producer or sibling-validator rule governing it. Each field is dispositioned as (a) enforced at receipt, (b) deliberately unvalidated, with a one-line reason, or (c) a gap, which becomes a named follow-up — it is NOT fixed inside the audit. The audit also writes the rule into `artifact.rs`'s module doc.

> **Caveat:** #3232 shows CLOSED as of 2026-11-04T17:03:48Z, but its comments record the audit as gated / not started, with #3187 as the last gate. Closed state alone is not evidence the audit completed.

_Citations: #3231 (OPEN, epic); #3232 (Phase 3 of #3231)._

## 4. #534 — Phase 12: which program, what the pipeline does

Phase 12 is the ID program's own phase — `pkgs/id` platform flexibility, planned in `.opencode/plans/phase-12-platform-flexibility.md`. It is not a phase of some other program.

The pipeline in #534 is a research-then-design-then-implement gate. Each item needs a design — or, for D1-026, a research package — before anyone implements it. Each item is run through the research workflow, the design is committed, and only then is an implementation issue opened.

There were 11 items, F12.1–F12.17. #534 closed on 2026-11-04: all 11 boxes qualify as done by the issue's own definition, and the remaining work lives in the 8 open implementation issues.

_Citations: #534 (CLOSED, epic, "id: Phase 12 designs to run through the research workflow"): items "need a design, or for D1-026 a research package, before anyone implements them. Run each through the research workflow, commit the design, then open an implementation issue." Items: manifest #807 (closed); roles v2 → #835 (open); provider registry #1061; first providers #1062; audit log #1063; ABI spec #1064 (closed); guest SDKs research #1065; program upgrade #1066; fair scheduling #1067; portals #1068; module trust #1069 (closed). Research package: `doc/2026-11-03T00-05-09Z_research_phase-12-package-534/` (PR #4672)._

## 5. #558 — `id world dev`: what the loop is and isn't

`id world dev` is a local command-line developer loop for world-program authors: watch the source, rebuild it, hot-swap it into a scratch world, replay fixtures on each change, and run a deterministic fuzz harness against declared invariants — with edit-to-running in under 3 seconds.

It is not a WYSIWYG editor, and it is not publish or rollback tooling for live worlds. The design uses an offline, disposable scratch world. A candidate is promoted only when fixture replay and migration pass; failures preserve the last-good scratch world — and that preservation is the only rollback-like element in the design.

The real implementation gate is #1066 (F12.9 upgrade/migration), which is still open.

_Citations: #558 (OPEN, "id: F12.15 `id world dev`: watch, rebuild, hot-swap, fixture replay, fuzz harness"): "Watch the source, rebuild, hot-swap into a scratch world via F12.9 migration, replay fixtures on each change, and run a deterministic fuzz harness against declared invariants; edit-to-running under 3 s." Design record: `doc/2026-11-01_design_id558.md`, PR #3823. Implementation issue #5541._

## 6. #788 — Native reference shell

This is a reference native client application — built with Bevy or Qt — that implements the F13.1 shell contract, for the things a terminal cannot do: 3D, audio, and native widgets. It also carries the WIT / component-model decision record.

Its acceptance test is equivalence: it renders the same world and emits the same input events as the terminal shell, with no host change, and it passes the F13.1 conformance check.

Current state: the shell contract exists, but no native client shell exists yet. A workspace member `crates/id-native-shell` — "Native reference shell for id (F13.6, design #788)", deliberately not a default member — now exists per the crate-split research (see §12).

_Citations: #788 (OPEN, XL, "id: Phase 13 F13.6 native client shells on the shell contract"). Shell contract: `pkgs/id/src/shell_contract.rs`, `contract/shell-contract.v1.json`, PR #842._

## 7. #790 — Topology: the specific cases

A client can connect to several worlds and servers, and to other clients, at once — and `id` is often client and server simultaneously. A client-only mode must always exist (cheap web load, SSH-only clients); SSH servers in `id` stay rare and opt-in. This issue is closed and implemented, across five specific cases:

1. **One client → many worlds on ONE server, concurrently.** Acceptance: one client holds live sessions to two worlds at once.
2. **One client → many servers at once.** A coordinator reuses a connection per endpoint; no connection spans servers; and same-named worlds on different endpoints cannot collide.
3. **Many clients → one world.** Joining appends nothing to the journal — with the caveat that first/last players' presence transitions may append a System event.
4. **Client + server simultaneously in one node.** One serve process hosts many worlds (`world_hub.rs`).
5. **Client-only mode.** Satisfied WITHOUT a `--client-only` flag: only an explicit `id serve` hosts a world, and SSH serving is opt-in.

_Citations: #790 (CLOSED + implemented): "Clients connect to several worlds and servers and to other clients at once, and id is often client and server simultaneously. A client-only mode must always exist (cheap web load, SSH-only clients); SSH servers in id stay rare and opt-in." Implemented: `WorldSessions` coordinator in `pkgs/id/src/world_net.rs` (PR #3896), PR #3862._

## 8. #835 — Moderator authority: the corrected model

The corrected model is neither new tiers nor creator-granted roles. Phase 12 (PR #801, F12.3) already added the `BUILDER` / `MODERATOR` scope bits, `invite --role builder|moderator`, and the journal-only audit entries `BuilderInstalled` and `Revoked{by}`. What is missing is the wiring: every caller passes `by: None`, so in practice a builder cannot install and a moderator cannot revoke over the wire, and the audit entries never appear. #835 is that missing session-frame wiring.

The powers are asymmetric. An install-module frame accepts a `BUILDER` holder, which leads to `install_program_by`; a revoke frame accepts a `MODERATOR` holder, which leads to `revoke_by`. A builder still receives no network, file, or process authority — and that is tested.

Frames are authorized by each session's live capability. The actor derives the audit identity; no client-supplied actor ID is trusted. Builders install Wasm only.

_Citations: #835 (OPEN). Implementation issue #5543._

## 9. Writer-set projection and the discovery query (#5545) — definitions

**Writer set.** A writer set is a signed, versioned, view-owner-owned object (kind `idset.writer`, shaped `{set_id, version, owner, scope?, entries, excludes}`) that says whose writes count. Its foundational rule is that ingest and authority are separate: every object that verifies is stored, and a view counts only its writer set.

Entries come in three forms: `Key{author, include_history}`, `Group{id, role_min, include_history}` (with one-level expansion), and `Public{include_history}`. Exclusions are evaluated after entries, and exclusions win. Identity is the 32-byte Ed25519 verifying key. Version is the owner's chain `seq_num`.

**View projection.** A view is a tuple — `{view_id, config_seq, owner, set:(set_id, set_seq), frontier, resolution, subject_scope, kind_scope, exceptions}` — and projection is a pure function of that whole tuple, or it is undefined. A claim is counted only if it was applied under the pinned resolution, within the frontier, not rescinded, in-scope, and its author was admitted by the set version in force when the claim was written (`seen_by`), or retro-imported via `include_history`.

The output is `Projection{counted, uncounted, provenance}` — never the same type for the two sides. Uncounted reasons are the closed enum `WhyNotCounted`: `AfterFrontier`, `RescindedSince`, `AuthorRemoved`, `HistoryNotImported`, `AuthorNeverAdmitted`, `OutOfScope`.

**Discovery query.** The discovery query is subject-first, like search: `discover(subject_or_terms, view) -> Discovery{counted, uncounted GROUPED BY AUTHOR with {author, petname?, claim_count, claims}, total_authors (Exact/AtLeast), next_cursor}`. The uncounted side is returned at all — that is what makes "uncounted" mean findable rather than discarded. Results are bounded by `MAX_DISCOVERY_ROWS`.

Tree state today: `idset.rs` (35,988 B) is types and encoding only. Expansion, projection, and counting are later slices, deliberately absent there; `WhyNotCounted` / `expand_retro` exist only in the design doc.

_Citations: #5545 (OPEN). Design doc `doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`, Rule §0: "ingest is a validity gate, never an authority gate; every object that verifies is stored; a view counts only its writer set."_

## 10. #3455 — The Cf contract

Confirmed: the primary meaning is exactly as titled — a value cannot act on the terminal. A plain-text field is row-safe only if it contains no character that can act on the terminal: no structure character (Cc controls, U+2028/U+2029) and no format character (category Cf, the crate's one shared set). Every plain renderer in `pkgs/id` accounts for both classes. The disposition differs by surface kind — row renderers escape, stripping sanitizers strip, ingest validators refuse — and no surface re-derives the character set.

The nuance is that the contract is broader than control sequences. Bidi overrides and isolates (U+202A–U+202E, U+2066–U+2069) visually reorder following fields, and zero-width characters (U+200B, U+FEFF, U+00AD) make look-alike rows with different bytes — the Trojan-Source class, on live CLI output. The existing guards tested only `char::is_control()` (Cc), which misses all of that.

Mechanically: there is one shared set — `petname.rs`'s `is_format_char`, promoted to a single `pub(crate)` definition. At row renderers the disposition is to ESCAPE (`\uXXXX`, content preserved, JSON-parseable), not strip. It is "the printer, not ingest": there is no new ingest validation, and `--json` is unchanged.

Explicitly NOT covered by #3455: ANSI/OSC terminal sequences — "no filed instance," listed in the design doc's open questions.

### Your extensions (not yet covered)

The following are Drew's extensions, and they are not yet covered by the #3455 contract as filed:

- HTML/JS-bearing output modes that some applications require;
- safe-plain rendering offered as a user choice;
- servers ingesting arbitrary data without executing it.

These are flagged as a candidate follow-on design, not as part of #3455.

_Citations: #3455 (CLOSED, sub-issues 4/4 complete, "Design: a value cannot act on the terminal — one format-character (Cf) contract for pkgs/id plain output"). Contract text: "A plain-text field is row-safe only if it contains no character that can act on the terminal: no structure character (Cc controls, U+2028/U+2029) and no format character (category Cf, the crate's one shared set). Every plain renderer in pkgs/id accounts for both classes; the disposition differs by surface kind — row renderers escape, stripping sanitizers strip, ingest validators refuse — and no surface re-derives the character set." Phases: #2839, #3111, #3089, plus the closing inventory._

## 11. #2146 — Programs-as-websites: progress and what remains

The problem: `id serve` could host a world only via the World Explorer's `/world` page, behind an interactive guest join. There was no way to give a program its own web presence. #2146 is closed as design delivered, and the decision was: v1 is a hosted world addressed by WorldName (selector `program:<world-name>`); bindings select Explorer or `Program(WorldName)`; the web token is listener-wide; the guest capability — not the web token — authorizes the session; and rendering is a trusted text / portable-screen presenter, with "no guest HTML, CSS, script."

What has landed, at `origin/main` `af0847d0`: in `serve.rs`, `WebContentTarget::{Explorer, Program(String)}` plus `WebBinding` plus `resolve_web_bindings` (#2145), per-listener routers, and persisted-publication boot restore; in `web/routes.rs`, `create_program_router` (program surface only, Explorer absent); in `templates.rs`, `render_program_page`; in `publications.rs`, the #2147 slice-1 registry (`Placement::{Listener, PathMount}`, `canonicalize_mount_path`, the shadow rule) — whose module doc says it is "deliberately the part with no sockets in it. Later slices wire it to live listeners… and to a CLI; this slice wires the boot half"; and in `web/src/world.ts`, the presenter.

What remains: Phase 4, live path-mount routing — there is no mount dispatch in `routes.rs` yet. And Phase 5, which is #2147, still OPEN: runtime publish/unpublish without restart.

One open question the design left for Drew: anonymous public program sessions, versus always invite-capability-gated. The design's recommendation is invite-gated for v1.

_Citations: #2146 (CLOSED as design delivered, PR #3836, `doc/2026-11-01_design_id2146.md`); #2147 (OPEN)._

## 12. Crate split — priority, current state, next steps

This work is prioritized: Drew said so on 2026-11-01, under umbrella issue #3447, which carries `priority/high` + `expedite` labels. Drew's priority order within the work is (1) optimal runtime performance of the shipped binary, and (2) build time. The split is also the mechanism for feature-leveled variants — client-only builds stripping most of the server — once the seams exist.

Current state: `pkgs/id` is ALREADY a workspace. Its members are `crates/id-core` (the extracted leaf/kernel, PR #3471) and `crates/id-native-shell` (the #788 shell, not a default member); `default-members` = root + `id-core`. But the root package is still the shipped binary plus facade, and `lib.rs` still declares 78 modules — the monolith is not yet split.

The plan is 11 crates — `id-kernel`, `id-caps`, `id-store`, `id-events`, `id-groups`, `id-sync`, `id-worlds`, `id-directory`, `id-web`, `id-cli`, plus the facade — in 7 one-PR steps, each under a 5-point verification protocol. Implementation issue #5546 is CLOSED as a superseded duplicate of umbrella #3447 plus the step issues.

Next steps, all OPEN:

- #4403 — step 1 remainder (extract `id-caps`)
- #4405 — step 2 (`id-store`, then `id-events`)
- #4406 — step 3 (`id-sync`)
- #4407 — step 4 (`id-groups`)
- #4408 — step 5 (`id-worlds`)
- #4409 — step 6 (`id-directory`, then `id-web`)
- #4411 — step 7 (`id-cli` + facade slimming)

_Citations: #3447 ("pkgs/id: split the monolith crate into sub-crates along module seams (prioritized by Drew 2026-11-01)"); Drew 2026-11-01: "prioritize the crate splitting work." Design: `doc/2026-11-01T16-53-00Z_design_id_crate_split/`._

## 13. KDL prior art — what the hunt found

The finding is negative: NO standalone old KDL brainstorming doc exists in the code repo's `doc/`, in the hatch-personal notes, or in the zettel corpora.

What does exist:

- `pkgs/id/README.original.md`, around line 38 — `kdl` listed among structured-data formats for hash-based ids;
- `pkgs/roc/out-in.kdl` — KDL brainstorming of Roc syntax;
- `pkgs/screen/README.md` — listing `kdl` beside `sparql`/`turtle`/`json-ld`;
- the 2022 repo `developing-today/hygge` — a rudimentary KDL builder/serializer, in Rust, using the `kdl` crate v3.

Memory summaries place the deep late-2023 KDL / graph-DB / RDF brainstorming in Drew's ChatGPT conversations — so it was likely never a repo file. The new design issue for this is #5688.

> **Caveat:** the laptop tree itself was unreachable during the hunt (the laptop was off). The repo findings were verified via a sandbox clone at 2026-11-02 HEAD plus GitHub code search, which agree.
