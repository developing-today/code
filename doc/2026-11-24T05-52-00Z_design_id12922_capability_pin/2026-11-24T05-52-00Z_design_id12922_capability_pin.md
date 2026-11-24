# Design: invite `capability_pin` term — pin the capability baseline as issued

- **Date:** 2026-11-24
- **Status:** **design record**, ready for issue-owner review; slice 1 is already implemented (PR #13150), the join-time wiring remains (#12921).
- **Issue:** [#12922](https://github.com/developing-today/code/issues/12922), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
- **Design of record this completes:** [#5959](https://github.com/developing-today/code/issues/5959) §7 (`doc/2026-11-23T05-02-30Z_design_id_capability_change_notices/`, PR #12920). That record settles the pin policy; this record extracts it into a standalone design for #12922 meeting the sufficient-design standard, because AUDIT-7 Lane 6 (DESIGN-GAPS, #13871) found #12922 had no design document of its own — no problem, goals, alternatives, approach, slices, acceptance, rollout/rollback, verification, or open questions recorded anywhere except §7 of the #5959 record and the issue body.
- **Consumed as settled, not re-derived:**
  - The comparison semantics of [#5999](https://github.com/developing-today/code/issues/5999) (`doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/`), §6 contract: compare `requested ∪ platform_bound` mapped to capability **classes** against the user's accepted baseline; trigger iff a class in the new message set is not covered by the baseline under the subsumption order; removals, within-class moves, messageless and optional additions, and used-set-only changes are silent. §9.1 names exactly what it leaves open: the invite interaction, settled by #5959 §7.
  - The registry schema of [#5958](https://github.com/developing-today/code/issues/5958) (`doc/2026-11-08T09-15-00Z_design_id_capability_model/`, §4): one record per **release identity** carrying `requested`, `discovered_used` (with certification state and provenance), and per-instance `effective_grant` / `observed` in the world-instance record. The record is immutable and content-addressed, which is what makes the pin small (see §4).
  - The permission-UX contract of [#5960](https://github.com/developing-today/code/issues/5960) (`doc/2026-11-08T09-06-08Z_design_id_permission_ux/`): pre-connect disclosure in two never-merged groups (client-side vs server-side), per-capability denial with graceful degradation, load-bearing refusals, remembered decisions inspectable on the world's grant page. Its §1.2 names #5959 as the owner of the change-notice shape its delta prompt uses; the invite-pinned first-connect notice of this record is that shape applied to a first join.
  - The invite model of [#5751](https://github.com/developing-today/code/issues/5751) (`doc/2026-11-14T18-50-00Z_design_id_invite_delegation/`): invite terms carried in the signed event body (expiry, use cap, redeemer pin, delegation depth, target world), with a hard 512-byte body budget. The §2–§4 terms the pin joins were implemented by [#12983](https://github.com/developing-today/code/issues/12983) (PR #13046, merged, #12983 closed) as the v2 postcard body; before that, archer lane HH verified the pin had no landable slice because the terms object it attaches to did not exist in code.

## 1. Problem

Accepting an invite is a first connect, and a first connect has no baseline to compare against. The #5959 change notice exists for the returning user: it fires when the world grew since the user last accepted it. For the invited user there is no "last accepted it" — so a world that grew between invite issuance and redemption would connect the joiner to a capability set nobody warned them about. The invite is a trusted link (the inviter vouched for the world *as issued*), which makes the silent upgrade worse, not better: the user accepted on the strength of the inviter's judgement of a world that no longer exists in that form.

Concretely, what breaks today, with evidence:

- The invite terms carry no statement of what the world could do when issued. Verified by reading the terms object on `origin/main` before PR #13046: `InviteBody` in `pkgs/id/crates/id-directory/src/invites/event.rs` was the fixed-layout v1 format (inviter / ticket_digest / group / role / expires_ms only). The v2 terms (PR #13046) still carry no capability statement except the pin slice 1 added.
- The redemption path has no "before" to compare against. The first-connect flow of #5960 §1.2 discloses the *current* set; it cannot say "since this invite was created, the world can now also…", because nothing records the issued set.
- The failure mode is silent by construction. A first join that shows only the current disclosure lets an expansion between issuance and redemption pass as if it were the world the inviter vouched for. Drew's requirement for the notice work is "a warning or notice **before the user connects** and has unexpected results" (#5959 §2); for invite joins that requirement had no mechanism at all.

## 2. Goals and non-goals

**Goals:**

1. An invite may record the capability baseline as issued, as one optional term, small enough for the 512-byte invite-body budget.
2. On redemption, the joiner's baseline is seeded from the pin when the current set is covered by the pinned set — no blocking capability decision required for the covered classes, because accepting the invite *was* the decision.
3. When the world outgrew the pin between issuance and redemption, the #5959 change notice fires **before the first connect completes**, worded for the situation, with the §4 choices unchanged.
4. When the pinned record cannot be fetched, the pin is ignored and the join is an ordinary first connect — an unusable pin is not an error and not a prompt.
5. The pin never widens anything: it grants nothing by itself and does not interact with the invite's own expiry / use-cap / redeemer-pin terms.

**Explicit non-goals:**

- Two-way sync, database views, and formula re-execution (v1 non-goals of the parent #6513 programme, restated here so this record is not read as expanding them).
- Any change to the #5999 §6 comparison contract, the #5958 §4 registry schema, or the #5960 disclosure shape. This record consumes all three.
- Server-side capability decisions. The pin is a client-side, first-join concern; server-side classes are decided at serve time under #5960 §3, and the notice discloses only their outcome.
- Mandatory pins. Public bearer links normally carry no pin (see §3).
- A baseline store, connect-stage comparison, or notice surface of its own. Those are #12921's scope; this issue owns only the pin term and its redemption decision, and consumes #12921's store and surface at the join (see §5, slices 2–3).

## 3. Alternatives considered

| Alternative | Tradeoff | Verdict |
| --- | --- | --- |
| **Do nothing.** Leave invite joins as ordinary first connects with only the current-set disclosure. | Zero work. But the silent-upgrade hole stays open exactly where trust is highest — a link from a known inviter. The #5959 notice requirement ("before the user connects") is unmet for the whole invite path. | Rejected. |
| **Mandatory invite pins.** Every invite carries a pin; redemption without a fetchable pinned record fails. | Uniform behaviour, no "when is a pin present" reasoning. But it makes every public link a capability vouch and puts a hard failure mode (unfetchable pinned record) on the public-distribution path, where the inviter is not vouching for a capability set to strangers. | Rejected (#5959 §9). Optional pin, graceful fallback. |
| **Pin carries the message set verbatim.** The invite terms list the classes/messages as issued. | The redeeming client needs no fetch to know the pinned set. But it blows the invite body's hard 512-byte budget for many-class worlds, and the listed set can go stale against the class-table version. | Rejected (#5959 §7, §9). Identity + recompute is smaller and cannot disagree with the record it names. |
| **Prompt on any invite join, pin or not.** Treat every first join through an invite as needing a blocking capability decision. | Catches expansions without any new term. But it prompts even when nothing changed since issuance, which is prompt fatigue for the common case, and it cannot distinguish "the world grew" from "this is just what the world does" — the distinction the pin exists to draw. | Rejected. The pin is what makes the prompt *conditional on growth*. |
| **Record the issued set server-side at mint time; the redeemer asks the server.** | No invite-body bytes at all; the server already knows the world. But it trusts the served record as a claim, contradicting the #5959 §1 trust note (the client does not trust the served record; the declared set is computed from the release the client actually connects to, verified against `release_identity`). It also adds a server round-trip and a server-side record to a flow that is otherwise client-computable. | Rejected. The pin keeps the issued identity in the signed invite body, where minting already puts its claims. |
| **Chosen: optional pin of `{ release_identity, table_version }`, identity + recompute.** | Two hashes and a version; fits the budget with room to spare; the pinned set is recomputed from the immutable, content-addressed registry record, so it cannot disagree with the record it names; covered ⇒ seed, uncovered ⇒ notice, unfetchable ⇒ ordinary first connect. | **Chosen.** |

## 4. Chosen approach

### 4.1 The pin term

Invite terms gain one optional field:

```rust
pub struct CapabilityPin {
    /// The pinned release's identity: the content address of its registry record.
    pub release_identity: [u8; 32],
    /// The class-table version the pinned set was computed under.
    pub table_version: u32,
}
```

- It deliberately does **not** carry the message set: the pinned release's registry record (#5958 §4) is immutable and content-addressed, so the redeeming client recomputes the pinned message set from the identity. The pin is two hashes and a version, sized to fit the invite body's 512-byte hard budget (#5751) — a class list would not guarantee that for many-class worlds.
- Minting: the inviter's client computes and attaches the pin at issuance (`InviteTerms::with_capability_pin`), optional; recommended for per-recipient links to sensitive worlds, normally absent on public bearer links. It is signed into the body with the other terms (`sign_terms`); delegation carries it explicitly (`child_terms`).
- The pin never widens anything: it grants nothing by itself, changes no server-side decision, and does not interact with the invite's own expiry / use-cap / redeemer-pin terms. The two pins compose: a redeemer pin says *who* may join; a capability pin says *what they were invited to*.

### 4.2 The redemption decision (pure function)

`InviteBody::pin_redemption(lookup, current) -> PinRedemption`, where `lookup: &dyn ReleaseMessageSetLookup` resolves the pinned release's message set from its registry record (`None` when the record cannot be fetched) and `current` is the message set of the release the client would actually join:

| Outcome | Condition | Effect |
| --- | --- | --- |
| `NoPin` | Invite carries no pin | Ordinary first connect. |
| `PinUnusable` | Pinned record cannot be fetched | Pin ignored; ordinary first connect. Not an error, not a prompt. |
| `SeedBaseline(set)` | Current set covered by pinned set | Pin seeds the baseline, `B := P` narrowed to `N` (#5959 §5 path 3 + narrowing). The first-connect disclosure still shows — it is a reading surface (#5960 §1) — and per-capability deny controls still work, narrowing `B` further; but no separate blocking capability decision is required for the covered classes. |
| `NoticeBeforeConnect(uncovered)` | Current set outgrew the pinned set | The #5959 §3 notice fires **before the first connect completes**, worded for the situation ("Since this invite was created, \<world\> can now also:"), with the §4 choices unchanged. Carries the uncovered part of the current set — what the world can now also do. |

The decision is deliberately pure — a function of (pinned set, current set, fetch outcome) over the `ReleaseMessageSetLookup` seam — so it is testable ahead of the store it seeds and the surface that presents it. The baseline store and the notice surface that consume the decision are #12921's; this is the decision they consume.

### 4.3 Data model

No new stored record is introduced by the pin itself:

- The pin lives only in the signed invite body (v2 postcard form; v1 bodies read as unpinned).
- The seeded baseline is a #12921 baseline-store record (§1 of #5959: per user + world, `baseline_key` over release identity, history kinds `accept | narrow | pin | grant-page-edit | yolo-auto-accept`), written with history kind `pin`.
- The pinned release's registry record is #5958 §4's record, fetched by identity; nothing about it is stored by the pin path.

### 4.4 Interfaces / contracts

- `CapabilityPin { release_identity: [u8; 32], table_version: u32 }` — `pkgs/id/crates/id-directory/src/invites/event.rs:124` (landed, PR #13150).
- `ReleaseMessageSetLookup::message_set(&self, pin: &CapabilityPin) -> Option<BTreeSet<String>>` — `event.rs:141`. The real lookup (fetch the immutable record by identity, recompute its set under the pin's table version) is slice 2, in #12921's connect-stage scope.
- `PinRedemption { NoPin, PinUnusable, SeedBaseline(BTreeSet<String>), NoticeBeforeConnect(BTreeSet<String>) }` — `event.rs:152`.
- `InviteTerms::with_capability_pin` (`event.rs:248`), `InviteTerms.capability_pin` (`event.rs:192`), `InviteBody.capability_pin` (`event.rs:290`), `InviteBody::pin_redemption` (`event.rs:578`).
- Consumed contracts, unchanged: #5999 §6 comparison, #5958 §4 registry record, #5960 §1.2 first-connect disclosure and §3–§4 notice/choices, #6017 observability surface (carries the raw diffs and baseline history the notice links to).

### 4.5 Security and capability analysis

- **The pin grants nothing.** It is not a capability, not a grant, and not an authority token. Its only effect is at redemption, and only in the narrowing direction (seed at or below the current set) or as a prompt. A forged pin cannot widen a join: the worst it does is seed a baseline at the current set (which the first-connect disclosure already establishes) or trigger a notice.
- **Untrusted served record.** Per #5959 §1's trust note, the client does not trust the served registry record as a claim: the declared set is computed from the release the client is actually connecting to, whose identity the client verifies against `release_identity`; the served record is a cache of that computation, not its source. If the served record and the computed set disagree, the computed set wins; if the client cannot compute, the comparison resolves to a trigger carrying `unknown` (#5999 invariant 10). A server that serves a broader manifest for the same identity therefore produces a prompt, not a silent pass. The pin's lookup inherits this rule: the pinned set is recomputed, not believed.
- **Unfetchable ⇒ ignore, never fail closed into a block.** A pinned record that cannot be fetched degrades to an ordinary first connect. This is deliberate: the pin is an optional hardening term, and making it load-bearing would let a missing record (or a network partition) deny joins that the invite itself authorises. The residual risk — an expansion hidden by an unfetchable record — is bounded by the first-connect disclosure, which still shows.
- **No interaction with invite authority terms.** Expiry, use cap, and redeemer pin are enforced independently; the pin neither extends nor shortens them, and a pin on an expired or exhausted invite is moot because redemption is refused before the pin is consulted.
- **Privacy.** The pin names a release identity and a table version — public facts about a release, not about the invitee. It carries no user identity and no usage history.
- **Downgrade safety.** v1 bodies read as unpinned; old clients ignore the term; new clients treat an absent pin as `NoPin`. No migration is needed.

## 5. Build slices (dependency-ordered)

Each slice is sized for one lane/session, with its own acceptance criteria. Slices 2–3 are shared with #12921's connect-stage scope and are tracked there; they are listed here so the dependency order of *this* issue is legible in one place.

### Slice 1 — Pin term + pure redemption decision — DONE

- **PR:** [#13150](https://github.com/developing-today/code/pull/13150) (merged, `d220c7ba3cc5`; archer lane LA).
- **What landed:** `CapabilityPin` as an optional term on `InviteTerms` and `InviteBody` (v2 postcard form; v1 bodies read as unpinned); minting attach via `with_capability_pin`, signed in by `sign_terms`, carried explicitly by `child_terms`; redemption decision as the pure function `InviteBody::pin_redemption` over the `ReleaseMessageSetLookup` seam.
- **Acceptance (met):** all four test oracles from the issue body pass as unit tests in `event.rs` (covered ⇒ `SeedBaseline` narrowed; uncovered ⇒ `NoticeBeforeConnect` with the uncovered part; unfetchable ⇒ `PinUnusable`; shrunk world ⇒ `SeedBaseline` at the narrower current set; unpinned ⇒ `NoPin`); the every-term round-trip carries a pin and stays inside the 512-byte budget. `cargo test -p id-directory`: 287 passed / 0 failed (281 base + 6 new). `cargo fmt --check` clean.
- **Depended on:** #12983 (v2 terms type, PR #13046) — the pin had no terms object to live on before it.

### Slice 2 — Real `ReleaseMessageSetLookup` (fetch + recompute)

- **Scope:** implement the lookup against the #5958 §4 release record: fetch the pinned release's immutable record by identity, recompute its message set under the pin's `table_version`; return `None` when the record cannot be fetched (offline, unknown identity, unparseable record). No network in tests — recorded fixtures.
- **Acceptance:** the lookup returns the pinned set for a recorded fixture release; an unfetchable fixture returns `None`; the recomputed set matches the record's declared set under the pin's table version, and a table-version skew resolves to the fail-closed `unknown` trigger per #5999 invariant 10.
- **Depends on:** slice 1; the #5958 §4 record-resolution path, which is part of #12921's connect-stage scope (`Requires: #12921`).

### Slice 3 — Connect-stage application + notice before first connect

- **Scope:** call `pin_redemption` during the first join through a pinned invite. `SeedBaseline` ⇒ write the baseline store with history kind `pin`, narrowed to the current set; `NoticeBeforeConnect` ⇒ hold the connection before any capability is exercised and present the #5959 §3 notice worded for the invite case, with the §4 choices; `PinUnusable` / `NoPin` ⇒ ordinary first connect.
- **Acceptance:** the four issue oracles end to end over fixtures — covered ⇒ baseline seeded, no blocking prompt; uncovered ⇒ notice before the first connect completes; unfetchable ⇒ ordinary first connect; pin present with a since-shrunk world ⇒ baseline seeds at the narrower current set. No imported object is readable beyond the ruled resolution order (the #5959 §7 / #5960 check).
- **Depends on:** slice 2; #12921's baseline store (slice 1 landed, PR #13079) and notice surface (data side landed, PR #14049, `9658b5c5`; grant-page rendering + live connect wiring remainder tracked in #14050, `Refs #13999` still open). This slice is the #12921 remainder that consumes the pin; it closes #12922 when its acceptance passes.

### Slice 4 — Minting default for per-recipient links

- **Scope:** decide and implement the minting default (see open question Q1): per-recipient links to sensitive worlds pin by default, or pin only on explicit opt-in. Client help/docs updated to state the recommendation in the same shape as #5751's redeemer pin.
- **Acceptance:** the chosen default is implemented in the minting path and documented; public bearer links still mint unpinned by default; a minted per-recipient link for a sensitive world carries a pin that round-trips through redemption.
- **Depends on:** slice 1; the Q1 decision (decider: Drew — see §8).

### Slice 5 — Live-workspace verification leg

- **Scope:** a pinned invite against a real world that grew between issuance and redemption; observe the notice firing before the first connect completes and the baseline seeding on the covered path. Named separately because it needs a live Notion-style test workspace — a real world, a real invite, a real release growth — not fixtures.
- **Acceptance:** observed (not fixture-simulated) notice before connect on the uncovered path; seeded baseline on the covered path; no grant created by the import/join beyond the ruled order.
- **Depends on:** slice 3; a live test world with an editions/inventory answer (gated on the Drew-owned editions/evaluation-budget answers recorded in #6513's family for live legs — same gate shape as the Notion importer's live leg).

## 6. Rollout + rollback

- **Rollout order:** this design record (this PR) → slice 2 (lookup) → slice 3 (connect-stage application, tracked under #12921) → slice 4 (minting default, after Q1) → slice 5 (live verification). Slice 1 is already on `main`; nothing in slices 2–4 changes the wire format — the pin term is already in the v2 body, and the lookup/application are client-side consumption of it.
- **No flag day.** The pin is optional: v1 bodies read as unpinned, old clients ignore the term, new clients treat absence as `NoPin`. Worlds, invites, and baselines created before the pin need no migration; the baseline store is #12921's, and pin-seeded baselines are ordinary baselines with history kind `pin`.
- **Rollback:** each slice reverts independently. Reverting slice 3 restores ordinary first connects for pinned invites (the pin term stays in the body, inert). Reverting slice 1 would remove the term from new mints; already-minted pinned invites would redeem as unpinned — safe, because the pin only ever narrows or prompts. There is no stored pin state to migrate back: the only persistent effect of a pin is a seeded baseline, which is a valid baseline with or without the pin.
- **Blast radius of a bad pin:** bounded by §4.5 — a wrong pin seeds a baseline at or below the current set or prompts. It cannot grant a capability, extend an invite, or widen a join.

## 7. Verification plan

- **The four test oracles** (from the issue body, already tests in slice 1, re-run end to end in slice 3): pin covered ⇒ baseline seeded, no blocking prompt; pin uncovered ⇒ notice before first connect; pin unfetchable ⇒ ordinary first connect; pin present with a since-shrunk world ⇒ baseline seeds at the narrower current set.
- **Property tests:** the comparison's property tests are #5999 §7 invariants 1–5, 8, 9 over `(B, N, ≼)`; the notice-level tests are #12921's (prompt iff trigger; per-class denial excludes exactly that class from the remembered set; narrowing on shrink, re-prompt on re-addition after narrowing; fail-closed prompt on an uncomputable comparison). The pin adds no new comparison invariant — it supplies `B`'s initial value on one path.
- **Budget check:** the every-term round-trip test carries a pin and asserts the signed body stays inside the 512-byte budget (in slice 1; re-asserted if the terms type changes).
- **Design-level check (this record):** read back this document against the 9-point sufficient-design standard — all 9 sections present, slices dependency-ordered with per-slice acceptance criteria, open questions each naming a decider — and link it from #12922. That read-back is the acceptance of the queue packet that produced this record (task 21661).
- **Live leg:** slice 5 (see §5); the live-workspace verification is named separately and gated as §5 states.

## 8. Open questions

- **Q1 — Minting default for per-recipient links.** Should the minting client pin by default on per-recipient links (to sensitive worlds, or to all per-recipient links), or only on explicit opt-in? The record recommends pinning for sensitive worlds and leaves the default open. **Decider: Drew** (policy; record the decision on #12922 before slice 4). Until decided, slice 4 waits and slices 1–3 are unaffected — the pin works either way once attached.
- **Q2 — Final notice copy for the invite-pinned uncovered case.** The structure and wording slot are fixed here ("Since this invite was created, \<world\> can now also:" + one plain-language line per new class), but final copy passes #5960's wording review; if that review changes a class string, only the client table changes. **Decider: #5960 wording review** (owner of the class strings).
- **Q3 — What counts as "unfetchable" for the real lookup.** Timeout, offline, unknown identity, unparseable record — all return `None` today by the trait contract, but the connect stage needs a stated boundary (e.g. retry-then-give-up vs fail-fast) and an observability hook so pin-unusable joins are visible on #6017's surface rather than silent. **Decider: #12921 implementation lane** (record the boundary on #12921; this record's contract — `None` ⇒ ordinary first connect — does not change).
- **Q4 — Should pin-seeded baselines and pin-unusable joins be recorded on #6017's observability surface?** The baseline history (§1) already records pin-seeding with kind `pin`; whether the per-user diff source surfaces it, and whether pin-unusable joins get a counter, is #6017's call. **Decider: #6017 owner** (Drew if unowned).

## 9. Acceptance (issue-level)

- This design record exists as a dated `doc/` document meeting all 9 points of the sufficient-design standard, linked from #12922 (this PR satisfies it).
- Slice 1's acceptance already holds on `main` (PR #13150; see §5).
- Slices 2–3 land with their acceptance criteria passing (the four oracles end to end over fixtures; no object readable beyond the ruled resolution order).
- Slice 4 lands after the Q1 decision.
- #12922 closes when slice 3's acceptance passes and this record is linked — the remaining minting-default (slice 4) and live-verification (slice 5) legs are tracked as follow-ups on the issue, not as blockers to closing, because the pin term, the decision, and the join-time wiring are the issue's scope and the pin is usable without them (slice 4 is a default, slice 5 is a verification leg gated on a live workspace).

## References

- [#12922](https://github.com/developing-today/code/issues/12922) — this design's issue (pin the capability baseline as issued).
- [#5959](https://github.com/developing-today/code/issues/5959) — capability-change notices design; §7 is the design of record for the pin (PR #12920).
- [#5955](https://github.com/developing-today/code/issues/5955) — the worlds design program.
- [#5958](https://github.com/developing-today/code/issues/5958) — capability model and registry schema; §4 is the record the pin recomputes from (PR #7599).
- [#5999](https://github.com/developing-today/code/issues/5999) — capability-change diff semantics; §6 is the comparison contract (PR #7630).
- [#5960](https://github.com/developing-today/code/issues/5960) — permission UX: disclosure, grant modes, memory (PR #7594).
- [#5751](https://github.com/developing-today/code/issues/5751) — invite delegation; the invite model the pin extends (`doc/2026-11-14T18-50-00Z_design_id_invite_delegation/`).
- [#12983](https://github.com/developing-today/code/issues/12983) — #5751 §5 step-1 terms extension (v2 postcard body, 512-byte budget); PR #13046, closed.
- [#12921](https://github.com/developing-today/code/issues/12921) — capability-change notices implementation (baseline store, connect stage, notice surface); baseline store PR #13079, notice-surface data side PR #14049, remainder #14050.
- [#6017](https://github.com/developing-today/code/issues/6017) — capability observability surface.
- [#13871](https://github.com/developing-today/code/issues/13871) / [#13873](https://github.com/developing-today/code/issues/13873) — AUDIT-7 Lane 6 DESIGN-GAPS, which found this design missing and filed the queue packet (task 21661) this record answers.
- `doc/2026-11-23T05-02-30Z_design_id_capability_change_notices/` — #5959 design record.
- `doc/2026-11-08T09-15-00Z_design_id_capability_model/` — #5958 registry schema.
- `doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/` — #5999 diff semantics.
- `pkgs/id/crates/id-directory/src/invites/event.rs` — `CapabilityPin` (:124), `ReleaseMessageSetLookup` (:141), `PinRedemption` (:152), `InviteTerms.capability_pin` (:192), `with_capability_pin` (:248), `pin_redemption` (:578).

## Change log

- 2026-11-24: Initial design record for #12922 (queue task 21661, xaltha lane): extracts #5959 §7 into a standalone 9-point design; records slice 1 as landed (PR #13150) with file:line evidence; slices 2–3 dependency-ordered against #12921 (store PR #13079, notice data side PR #14049, remainder #14050); open questions Q1–Q4 each name a decider.
