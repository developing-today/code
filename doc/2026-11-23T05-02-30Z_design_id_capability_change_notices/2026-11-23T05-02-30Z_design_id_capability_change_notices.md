# Design: capability-change notices on reconnect — presentation, choices, re-baselining, invite pinning

- **Date:** 2026-11-23
- **Status:** **design decision record**, ready for issue-owner review; no implementation is included.
- **Issue:** [#5959](https://github.com/developing-today/code/issues/5959), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S17–S18 of the 2026-11-04 brief).
- **Gate history:** this design was explicitly gated on closure of the capability-model design [#5958](https://github.com/developing-today/code/issues/5958) (routing recorded on this issue 2026-11-17, task 19933). #5958 closed 2026-11-23 with its record on `main`; this record is written against that landed schema, not a draft of it.
- **Consumed as settled, not re-derived:**
  - The comparison semantics of [#5999](https://github.com/developing-today/code/issues/5999) (`doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/`), §6 contract: compare `requested ∪ platform_bound` mapped to capability **classes** against the user's accepted baseline; trigger iff a class in the new message set is not covered by the baseline under the subsumption order; removals, within-class moves, messageless and optional additions, and used-set-only changes are silent. §6 names exactly what it leaves to this record: presentation, wording delivery, the choice controls, placement in the connect flow, and the invite interaction.
  - The registry schema of [#5958](https://github.com/developing-today/code/issues/5958) (`doc/2026-11-08T09-15-00Z_design_id_capability_model/`, §4): one record per **release identity** carrying `requested`, `discovered_used` (with certification state and provenance), and per-instance `effective_grant` / `observed` in the world-instance record.
  - The permission-UX contract of [#5960](https://github.com/developing-today/code/issues/5960) (`doc/2026-11-08T09-06-08Z_design_id_permission_ux/`): pre-connect disclosure in two never-merged groups (client-side vs server-side), per-capability denial with graceful degradation, load-bearing refusals, remembered decisions inspectable on the world's grant page. Its §1.2 names this record as the owner of the change-notice shape its delta prompt uses.
  - The precedent survey [#5961](https://github.com/developing-today/code/issues/5961) (`doc/2026-11-12T20-58-00Z_research_id_permission_precedents/`): fixed plain-language warning strings over free text (Chrome), one-time grants as a first-class mode (Android "Only this time"), declining an update's new permissions blocks only that update (Firefox), and the fatigue evidence against prompt-on-any-change (Felt et al.).
  - The invite model of [#5751](https://github.com/developing-today/code/issues/5751) (`doc/2026-11-14T18-50-00Z_design_id_invite_delegation/`): invite terms carried in the signed event body (expiry, use cap, redeemer pin, delegation depth, target world), with a hard 512-byte body budget.

## Decision summary

| Question              | Decision                                                                                                                                                                                                                         | Where |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| Comparison baseline   | The accepted baseline `B` of #5999 §6, stored client-side per (user, world), keyed by `baseline_key` over the #5958 **release identity**                                                                                         | §1    |
| When the notice fires | During connect, after the release record is resolved and `N` computed, **before** the connection completes or any capability is exercised; fail-closed (`unknown`) when the comparison cannot be computed                        | §2    |
| Notice content        | Headline + one plain-language line per **new class only**, client table text, client-side and server-side lines in separate groups; the full current disclosure and the used-set view are one expansion away, never the headline | §3    |
| User choices          | **Connect once** / **Connect and remember this set** / **Don't connect** — no default button; per-class denial of new client-side classes composes with the first two                                                            | §4    |
| Decrease asymmetry    | Decreases never prompt; the baseline **narrows automatically** to the covered smaller set on a silent reconnect, so a later re-addition prompts as the expansion it is                                                           | §5    |
| Re-baselining rule    | `B` changes only by: an explicit remember-choice, first-connect decisions, an invite pin, automatic narrowing, or a grant-page edit. Never by connecting, by a server action, or by an update alone                              | §5    |
| Other audiences       | Server admins decide new **server-side** classes at serve time under #5960 §3 modes (users see disclosure only); authors get the declaration-defect channel at publish time, never a user prompt                                 | §6    |
| Invites               | Accepting an invite **may pin** the capability baseline as issued: an optional pin in the invite terms records the release identity + table version; on first join, an uncovered expansion prompts exactly like a reconnect      | §7    |
| Grant-on-use          | The notice does not cover it; where a world's policy is grant-on-use, the notice carries a standing honesty line saying so                                                                                                       | §3.3  |

## 1. The baseline, against the #5958 schema

The comparison of #5999 §6 needs three inputs. Against the landed registry schema they are:

- **`requested`** — the release record's `requested.caps` (§4.2 of the capability model), a property of the immutable release.
- **`platform_bound`** — the caps reachable through the platform the release was compiled against, identified by `release.platform.tree_hash`. The release record names the platform; the platform tree's reachable set is a fact of that tree hash, evaluated with the same `*` expansion the ledger uses (#5999 §1).
- **The class mapping** — the `class` field on `ProviderManifest` that #5999 §2.4 specifies and its follow-ups file. Until a name carries a class, it maps to `unknown`, which always discloses (#5999 invariant 4).

The **accepted baseline** is a client-side record, stored with the user's other ID state (the same store #5960 §5.1 puts remembered grants in, per user identity, following the user across devices — never per device fingerprint):

```json
{
  "world_id": "<world instance id>",
  "release_identity": "<blake3 hex>",
  "message_set": ["chat.write", "device.location"],
  "table_version": 3,
  "baseline_key": "H(world_id ‖ release_identity ‖ message_set ‖ table_version)",
  "history": [{ "at": "...", "kind": "accept | narrow | pin | grant-page-edit", "message_set": ["..."] }]
}
```

`release_identity` here is the capability model's content-addressed identity (platform tree hash + source closure hash + DLL closure hash + compiler digest + target). It plays the role #5999 §4.2 assigns to `module_content_hash`, in the precise form the landed schema gives it.

**Trust note (reconciling #5999 §1 with #5958 §4.3).** #5999 requires the compared entry to be the signed, content-addressed one, or the baseline is attackable; #5958 §4.3 deliberately leaves the registry record server-local and unsigned in v1. This record resolves the two the only way consistent with both: **the client does not trust the served record as a claim.** The declared set is computed from the release the client is actually connecting to, whose identity the client verifies against `release_identity`; the served record is a cache of that computation, not its source. If the served record and the computed set disagree, the computed set wins; if the client cannot compute (missing manifest data, unparseable class, version skew), the comparison resolves to a trigger carrying `unknown` (#5999 invariant 10). A server that serves a broader manifest for the same identity therefore produces a prompt, not a silent pass.

## 2. Placement in the connect flow

The check is a stage of connecting, not a screen before it and not a badge after it:

1. The user initiates a connect (or a reconnect) to a world.
2. The client resolves the world's current release record and instance record, verifies the release identity, and computes `N = messages(requested ∪ platform_bound)` under its current class table.
3. **Short-circuit:** if the baseline's `release_identity` and `table_version` both match the current computation inputs, the verdict is the recorded one — silent (idempotence, #5999 invariant 8). No recompute, no prompt.
4. Otherwise the client compares `N` against `B`:
   - **Covered** (`∀ c ∈ N, ∃ b ∈ B : c ≼ b`): the connection proceeds with no prompt. If `N` is strictly smaller, the baseline narrows (§5) and the narrowing is written to history.
   - **Not covered**: the connection is **held** — no world code runs, no capability is exercised, no presence is announced — and the notice (§3) is presented with the choices of §4. Only a choice completes or aborts the connection.
5. A first connect has no `B`: it follows the #5960 §1.2 flow (full disclosure, decision required for client-side lines), and the decisions taken there establish `B`. The notice of this record is the _returning_-user surface; §7 covers the one case where a first connect also produces a notice (an invite pin).

Drew's requirement is "a warning or notice **before the user connects** and has unexpected results" — step 4's hold is what makes that literal: the prompt is not advisory chrome around a connection that already happened, and declining it leaves the user exactly where they were (§4).

## 3. The notice

### 3.1 Content

```
<World name> can do more than when you last connected

Since you accepted this world, it can now also:

Using your client
  • Use this device's location.

Through the server side
  • Connect to any host on the network.        (disclosure only — decided by the server's admin)

[ See everything this world can do ]   [ What changed in detail ]
```

Rules:

- **Only the delta is the headline.** The lines are exactly `new_classes` from #5999 §6 — the classes in `N` not covered by `B` — one line each, in the client's fixed text for that class id. Listing everything the world can do, changed or not, is the install-time bundle the precedent survey shows users stop reading (#5961 §2); the full current disclosure (both groups, #5960 §1.1 shape, with the used-set annotations) is one expansion away, labelled as above.
- **The two groups are never merged** (#5960 §1): client-side lines carry decision controls (§4); server-side lines are disclosure only, marked as decided by the server's admin, because the connecting user cannot grant or deny someone else's machine (#5960 §3, #5961 §4 admin pre-consent pattern).
- **Plain language, never a hash.** The headline names the world and the direction of change; the lines name consequences. `baseline_key`, release identities, and raw capability names appear nowhere in the headline surface. The raw declared diff, the used set, and the over-declaration flag ride in "What changed in detail" and on the observability surface (#6017) — detail for the curious, never the trigger and never the message (#5999 §6 givens).
- **The used set is shown, never warned on.** In the expanded view, a requested-but-unused line carries #5960's annotation ("asks for this, has not used it"). A used set wider than the declaration is shown as what #5999 §5 settles it is — an author declaration defect — worded as a fact about the world's paperwork, not as a decision the user is being asked to make; the cure is the author's declaration change, which then arrives through the normal trigger if it adds a class.
- **Wording is client-side and fixed.** Class text comes from the client's class table keyed by class id (#5999 §2.4); the registry entry supplies ids, never sentences — a server that could word the warning could soften it. Final copy is #5960's to approve (its wording-review open question); this record fixes the structure, the grouping, and which strings exist, not their final phrasing.

### 3.2 The fail-closed form

When the comparison cannot be computed (§1), the notice appears with the single `unknown` line — "Do something this version of the app cannot describe." — and the same three choices. Silence requires understanding; an unparseable world is a prompting world (#5999 invariants 4 and 10).

### 3.3 The grant-on-use honesty line

Grant-on-use (`Decision::AllowAndGrant`) can grow the _granted_ set during a connection; no reconnect notice can cover that, and #5999 §4 rule 9 assigns the seam to this record. Where the world's policy is grant-on-use (already a reported fact in `CapsReport.policy`), the notice — and the first-connect disclosure — carries one standing line: "This world may ask for more while you are connected; those requests are decided as they happen, not by this notice." The notice never claims a guarantee it does not provide.

## 4. The user's choices

Exactly three primary choices, with the semantics #5999 §4.1 fixes and the controls this record settles:

| Choice                            | Connects?        | Effect on `B`                                                                    | Precedent                                                                           |
| --------------------------------- | ---------------- | -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| **Connect once**                  | Yes, holding `N` | Unchanged — the same change prompts again next time                              | Android "Only this time" (#5961 §1.2)                                               |
| **Connect and remember this set** | Yes, holding `N` | `B := N` — the only notice path that raises the baseline, always an explicit act | Remembered per-scope grants (#5961 §4)                                              |
| **Don't connect**                 | No               | Unchanged                                                                        | Firefox: declining an update's new permissions blocks only that update (#5961 §1.5) |

Rules:

- **No default button.** No choice is pre-selected and Enter accepts nothing; a default on a warning is an implicit-accept path, and implicit accept is the silent upgrade the requirement forbids (#5999 §4.1). Focus lands on the headline, not on a control.
- **Per-class denial composes with connecting.** Expanding a new client-side line offers a deny for that class, with #5960 §2 semantics: a denied optional class is inert and its features show as unavailable; the connection proceeds without it. "Connect and remember" after denials sets `B := N ∖ {denied classes}` — a denied class was never accepted, so if the world still requests it, it prompts again next time; that is the correct consequence of the denial, not a bug. Denying a class the world declares **load-bearing** is equivalent to Don't connect for this attempt, and the notice says so in those words before the user commits to it ("This world cannot run without …").
- **Server-side lines have no controls.** They are present so the user is informed, and their absence of buttons is itself labelled (§3.1), so the user does not read a disclosure as a decision they made.
- **Refusal costs nothing else.** Don't connect leaves the world installed, the baseline untouched, and prior remembered grants intact. There is no "reject this world" escalation attached to the notice; leaving is always possible and never punished (#5960 §2's shape, Firefox's precedent).

## 5. Re-baselining and the decrease asymmetry

`B` changes only through these five paths, each written to the baseline's history and inspectable on the world's grant page (#5960 §5.1):

1. **Explicit remember** on a notice (§4).
2. **First-connect decisions** under #5960 §1.2 (including per-capability denials, which exclude classes from the initial `B`).
3. **An invite pin** (§7).
4. **Automatic narrowing:** on a covered reconnect where `N` is strictly narrower than `B`'s coverage, `B := N`. No prompt — a decrease is never an interruption (#5999 rule 2).
5. **Grant-page edits:** a user revoking a remembered grant removes the corresponding class from `B`; if the world later requests it again, it is new and prompts.

And never through: connecting by itself, a server or admin action, a world update by itself, or the passage of time.

The issue asks whether there is a "remember for capability decreases only" asymmetry. The answer is stronger than the guess: **decreases are not remembered because they are never asked.** They narrow the baseline automatically (path 4), and the narrowing is load-bearing, not bookkeeping: if a world drops a class and the baseline kept it, the world could silently re-add that class later under the old acceptance. With narrowing, the re-addition is an expansion against the current baseline and prompts — which is the behaviour "no silent upgrades" requires. The asymmetry is therefore: **increases always prompt and change nothing until accepted; decreases never prompt and always narrow.**

**YOLO interaction** (#5960 §5.2): while client-side YOLO is on, the trigger is suppressed — that is what YOLO means — but the comparison still runs and the bookkeeping still happens: each connect under YOLO records `B := N` with history kind `yolo-auto-accept`, and the notice that would have fired is listed on the grant page as auto-accepted. Turning YOLO off therefore never resurrects a stale baseline, and the loud YOLO indicator remains the user's standing warning that this is happening.

## 6. Who else gets notified

The notice is a per-user, client-side surface. Two other audiences have their own channels; neither is this prompt:

- **Server admins — server-side classes are decided at serve time, not at someone else's connect.** When a release whose server-side message set is not covered by the admin's recorded decisions is first registered/served on a server, it appears on the admin surface as a pending item under the #5960 §3 grant modes: _approve generally_ covers future releases silently; a per-permission or per-website _ignore_ makes the class inert there; until decided, the world is served without the undecided server-side classes where they are optional, and not served where one is load-bearing (§5960 §4's server-side rule). Every such decision lands in the server's grant log with who/when/mode/scope. The connecting user's notice (§3) only discloses the outcome.
- **World and library authors — the declaration-defect channel.** When a publish-time derivation shows a release's `discovered_used` exceeding its declaration, or a dependency's footprint change moving a dependent world's declared message set, the author is notified at publish time through the registry surface: the release record's `discovered_used.provenance` and analyzer fields name what changed and which dependency moved. The author's cure is a declaration change; the user-facing consequence, if any, then flows through §2's trigger as a real class with real wording. Authors are never notified via their users' prompts, and users are never prompted about a defect that is the author's to fix (#5999 §5).

## 7. Invites: accepting an invite pins the baseline as issued

#5999 §9.1 leaves the policy to this record and #5751: should accepting an invite pin the capability set the invite was issued under? **Decision: yes — as an optional term of the invite, recorded as a pin; never as an obligation on the inviter.**

**The pin.** An invite's terms may carry `capability_pin: { release_identity, table_version }` — the release identity of the world as issued, and the class-table version the inviter's client computed under. It deliberately does **not** carry the message set itself: the pinned release's record is immutable and content-addressed, so the redeeming client recomputes the pinned message set from the pinned release identity. The pin is two hashes and a version — it fits the invite body's 512-byte hard budget (#5751) with room to spare, which a class list would not guarantee for many-class worlds.

**On redemption / first join through a pinned invite:**

1. The redeeming client fetches the pinned release's record by identity and computes the pinned message set `P`. (If it cannot — the pinned release is unavailable — the pin is unusable and the join falls back to an ordinary first connect; an unusable pin is not an error and not a prompt.)
2. It computes `N` for the release it would actually join.
   - **`N` covered by `P`:** the pin seeds the baseline, `B := P` narrowed to `N` (§5 path 3 + narrowing). The first-connect disclosure still shows — it is a reading surface (#5960 §1) — and per-capability deny controls still work, narrowing `B` further; but no separate blocking capability decision is required for the covered classes. Accepting the invite _was_ the decision, made with the inviter's vouch for the world as issued.
   - **`N` not covered by `P`:** the world grew between issuance and redemption. The notice of §3 fires **before the first connect completes**, worded for the situation ("Since this invite was created, <world> can now also:"), with the §4 choices unchanged. This is the case the pin exists for: without it, a first-join user has no "before" to compare against, and an expansion between issuance and redemption would be invisible — a silent upgrade delivered by a trusted link.
3. **A pin never widens anything.** It can only seed a baseline at or below the current `N`; it grants nothing by itself, changes no server-side decision, and survives nothing about expiry or use caps — those remain the invite's own terms (#5751), enforced independently.

Bearer public-distribution links will normally carry no pin — the inviter is not vouching for a capability set to strangers — and per-recipient links for sensitive worlds should carry one, the same recommendation shape as #5751's redeemer pin. The two pins compose: a redeemer pin says _who_ may join; a capability pin says _what they were invited to_.

## 8. Implementation shape

No code in this record; the shape an implementation issue is written against:

- **Client baseline store:** the §1 record, in the per-user ID state store beside #5960's remembered grants; the grant page lists baselines and their history next to the grants they interact with.
- **Connect pipeline:** the §2 stage sits between release resolution and connection completion; its inputs are the two #5958 records (release + instance), the client class table, and the baseline store. The comparison itself is #5999 §6's pure functions, unchanged.
- **Notice surface:** §3's layout contract; class strings from the client table; the expanded views reuse #5960's disclosure component and link the raw diff to #6017's surface.
- **Invite delta:** one optional field in the invite terms (§7), minted by the inviter's client at issuance, consumed at redemption; no wire change beyond the terms object #5751 already specifies.
- **Admin/author surfaces:** §6 rides #5960 §3's grant log and the registry's publish pipeline; no new notification transport is introduced by this record.

**Named test oracles** (for the implementation issues): the comparison's property tests are #5999 §7 invariants 1–5, 8, 9 over `(B, N, ≼)`; the notice-level tests are: prompt iff trigger fires; per-class denial excludes exactly that class from the remembered set; narrowing on shrink, and re-prompt on re-addition after narrowing; invite pin covered ⇒ no blocking prompt, invite pin uncovered ⇒ prompt before first connect; YOLO auto-accept records history and never leaves a stale baseline; fail-closed prompt on an uncomputable comparison.

## 9. Alternatives rejected

- **Prompt on any change, decreases included.** The fatigue failure (#5961 §2) and a direct contradiction of #5999 rule 2. Rejected.
- **Whole-set accept only, no per-class denial.** Simpler, but it breaks #5960 §2's per-capability denial contract and forces an all-or-nothing choice Chrome's model exists to avoid. Rejected; per-class denial composes instead (§4).
- **A default "Connect" button for speed.** A default on a warning is an implicit accept path. Rejected (§4).
- **Mandatory invite pins.** Would make every public link a capability vouch and put a hard failure mode (unfetchable pinned record) on the public-distribution path. Rejected; optional, with a graceful fallback (§7).
- **Pin carries the message set verbatim.** Blows the invite body's hard byte budget for many-class worlds and can go stale against the table version; the identity + recompute form is smaller and cannot disagree with the record it names. Rejected (§7).
- **Notify users about used-set growth directly.** #5999 §5 settles this: it is an author declaration defect, cured by a declaration change. Routing it to users as a warning would hide which capability became reachable and would fire on analyzer churn. Rejected (§6).

## 10. Follow-ups

- Implementation of §8 (baseline store, connect stage, notice surface) files as an implementation issue against `pkgs/id` citing this record and #5999 §6; the `class` field on `ProviderManifest` remains #5999's own follow-up and gates it.
- The invite `capability_pin` term files as a small implementation issue against the #5751 invite terms, citing §7.
- Final notice copy passes #5960's wording review; if that review changes a class string, only the client table changes — no rule in this record depends on the phrasing.
- #6017's observability surface consumes the baseline history (§1) as its per-user diff source; no schema addition is needed from this record.

## References

- [#5959](https://github.com/developing-today/code/issues/5959) — this design deliverable (S17–S18).
- [#5955](https://github.com/developing-today/code/issues/5955) — the worlds design program.
- [#5958](https://github.com/developing-today/code/issues/5958) — capability model and registry schema (the gate for this record; closed 2026-11-23, PR [#7599](https://github.com/developing-today/code/pull/7599)).
- [#5999](https://github.com/developing-today/code/issues/5999) — capability-change diff semantics; §6 is the contract §1–§2 consume (PR [#7630](https://github.com/developing-today/code/pull/7630)).
- [#5960](https://github.com/developing-today/code/issues/5960) — permission UX: disclosure, grant modes, memory (PR [#7594](https://github.com/developing-today/code/pull/7594)).
- [#5961](https://github.com/developing-today/code/issues/5961) — permission-prompt precedent survey (PR [#9031](https://github.com/developing-today/code/pull/9031)).
- [#5751](https://github.com/developing-today/code/issues/5751) — invite delegation; §7 adds one optional term to its model.
- [#6017](https://github.com/developing-today/code/issues/6017) — capability observability surface; carries the raw diffs and history this record's notice links to.
- `doc/2026-11-08T09-15-00Z_design_id_capability_model/` — registry schema (§4) this record's baseline is keyed against.
- `doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/` — classes, subsumption, trigger rules, and the §6 contract.

## Change log

- 2026-11-23: Initial design record for #5959 (queue task 19933, lane FG / archer): placement in the connect flow, notice content and fail-closed form, the three choices with per-class denial, the re-baselining rule with automatic narrowing, admin/author channels, and the invite capability pin. Written after the #5958 gate cleared (closed 2026-11-23T04:05Z).
