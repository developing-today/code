# Design: world governance — moderation-as-library, per-world parameterization, pre-approval

- Date: 2026-11-08
- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Issue: [#5973](https://github.com/developing-today/code/issues/5973), child of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955) (statements S75–S76 of the 2026-11-04 brief).
- Specified against: `origin/main` `83b7a024b8`, read 2026-11-08. Every substrate claim below is cited `path:line` at that revision.
- Consumes, never re-defines: the capability-model registry schema of [#5958](https://github.com/developing-today/code/issues/5958) (`requested` / `discovered_used` / `effective_grant` / `observed`) and the audience vocabulary of [#5960](https://github.com/developing-today/code/issues/5960) (`doc/2026-11-08T09-06-08Z_design_id_permission_ux/`; the issue is closed, the record is landed). Where this record names a capability, it names a **provider-registry entry** in the sense of `world_caps.rs`, not a new kind of thing.

## Decision in brief

Moderation is **world code over the ordinary capability surface**, not a platform feature. Concretely:

1. **There is no moderation capability set to invent.** The provider registry has 13 entries — `time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`, `world.info`, `loc.position`, `loc.watch` (`pkgs/id/crates/id-core/src/world_caps.rs`, `define_provider_registry!` block) — and **none of them is a moderation power**. A moderation library does not get a new privileged vocabulary; it gets the same providers any world code gets, plus the authority that already exists for the actions it performs.
2. **The two powers a moderation library actually needs already exist and are already separated from capabilities.** Revocation is `WorldCommand::Revoke` / `revoke_by(participant_id, by)` (`pkgs/id/src/world/handle.rs:653`), journaled as `JournalEntry::Revoked { participant_id, by }` where `by: None` means admin and `Some(participant)` means a moderator (`pkgs/id/src/world/mod.rs:1222`). Isolation, cross-world writes and file scope are gated by `require_moderation` (`pkgs/id/src/world/actor.rs:1083`), which accepts `Authority::Admin` **or** an account whose directory group level is `>= Level::Moderator` (`pkgs/id/src/directory.rs:1112`). These are *authority*, not *capability*: they are not in the `wants` document and a world program cannot request them.
3. **Therefore the moderation-library interface is a narrow, explicit seam**: a library declares the moderation *actions* it may take; the world's governance configuration binds those actions to participants; the platform executes each action through the existing authority path and journals it. The library never holds a capability that bypasses the authority check.
4. **Parameterization is configuration of one library, never a fork** — moderator/reviewer sets and thresholds are per-world settings of the same code, which is what makes "literally the same thing from a different world, with different moderators and a different threshold" true rather than aspirational.
5. **Pre-approval binds a content hash and the capability set**, not a name or a version string — an approval that does not name what it approved is a rubber stamp.

The rest of this record settles the four things the issue asks for: the interface (§2), the authority model and its relation to writer-set authority (§3), parameterization (§4), pre-approval (§5), and the boundary with the boards line (§6).

---

## 1. Verified current state

Read at `83b7a024b8`. This section is the evidence base; every later decision cites it.

### 1.1 Participant scopes — the moderator bit exists and is admin-only

`WorldScopes` is a `u8` bitset (`pkgs/id/src/world/mod.rs:158`). Bits:

| Scope | Bit | Meaning (`mod.rs:161-176`) |
|---|---|---|
| `JOIN` | `0b001` | join the world, receive snapshots/events |
| `CHAT` | `0b010` | send chat |
| `INPUT` | `0b100` | submit game input |
| `DELEGATE` | `0b1000` | mint capabilities with a subset of this set |
| `HISTORY` | `0b1_0000` | read chat without sending |
| `BUILDER` | `0b10_0000` | install and upgrade the world's program — *admin authority bit* |
| `MODERATOR` | `0b100_0000` | **revoke participants** — *admin authority bit* |

Two facts matter for governance:

- `ALL` deliberately **excludes** `BUILDER` and `MODERATOR` (`mod.rs:179-182`): "they are issued only by the admin, never implied by a ceiling or handed out by a delegator." `ALL_BITS` (`mod.rs:186`) includes them only so capability round-trips do not widen `ALL`.
- `from_role("moderator")` yields `GUEST | MODERATOR` (`mod.rs:217`), so a moderator keeps the ordinary guest powers and adds revoke. `ROLES` lists seven presets (`mod.rs:223-231`).

### 1.2 The moderator authority is enforced in two different ways

- **Directory-level**: `require_moderation` (`actor.rs:1083`) permits `Authority::Admin` or a signed key whose account is `directory.is_moderator(account)` — i.e. holds `Level::Moderator` or better in *some* group (`directory.rs:1112`). It currently guards exactly three operations: `set_isolation` (`actor.rs:1103`), `set_cross_world_write` (`actor.rs:1115`), and `set_file_scope` (`actor.rs:1159`).
- **Participant-scope-level**: revocation carries a `by: Option<u64>` moderator id into the journal (`handle.rs:653`, `mod.rs:1222-1230`), so the audit trail records *which participant* revoked.

These are two independent notions of "moderator" that happen to share a name. §3.2 resolves that.

### 1.3 The roles record already deferred exactly this work

`doc/2026-10-17T08-00-00Z_feature_world_roles_directory/…md` states: "Roles that would need new authority (a builder who installs modules, **a moderator who kicks**) stay future work: those are admin-token powers today, and handing them to a capability needs its own design." The `MODERATOR` bit and `BUILDER` bit have since landed, but the *governance* design that says who may hold them, and how a world delegates moderation without handing out admin, is what this record supplies.

### 1.4 There is no moderation provider, and adding one would be the wrong move

The catalog is closed at 13 entries, each a `ProviderManifest` with `name`, `version`, `kind`, `argument_schema`, `bounds`, `required_scope`, `default_ceiling` (`world_caps.rs:202-232`). `RequiredScope` has only `Join`, `Chat`, `ChatRead` (`world_caps.rs:190-200`) — there is no "moderator" scope a provider could require. A `moderation.hide` provider would therefore have to either (a) be requestable by any world code that holds it, which makes it a capability — exactly the thing §1.2 says moderation is not — or (b) be gated by a scope the `RequiredScope` enum cannot express. Both are wrong; §2.2 explains the alternative.

### 1.5 Writer-set authority is a *view* concept, and its issue is closed

`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/` establishes the rule: "ingest is a validity gate, never an authority gate; every object that verifies is stored; a view counts only its writer set." The issue the packet cites for it, [#5545](https://github.com/developing-today/code/issues/5545), is **CLOSED** (verified 2026-11-08) — its scope was view projection and the discovery query over writer-set objects, not moderation. The writer-set *concept* remains live in `pkgs/id/crates/id-core/src/membership/layer.rs` (one-level group expansion, "writer-set spec §D4"). See §3.3 for the correct relation and §8 for the packet drift this creates.

---

## 2. The moderation-as-library interface

### 2.1 The shape: actions, not capabilities

A moderation library is a world-code library that, in addition to its ordinary providers, declares a **governance action set** and receives **governance events**. The four hooks the issue names map onto that shape:

| Hook | Direction | Who provides the mechanism | Who decides the policy |
|---|---|---|---|
| **submission approval** | library → platform (verdict) | platform: the submission gate and its journal | the world's governance config (§4) |
| **report intake** | platform → library (event) | platform: report transport, rate limits, dedup | the library's rules + config thresholds |
| **quarantine action** | library → platform (action) | platform: the existing hide/withhold path over world state | the library, within its bound authority |
| **appeal** | participant → library → platform | platform: appeal transport and journal | the library's rules; the config names reviewers |

The split is the one the issue asks for and it is the *only* split that survives §1.2: **mechanism is platform-provided, policy is world-provided.** A world cannot re-implement report transport (it would need network and storage it does not have — see the 13-provider catalog), and the platform cannot decide what a world considers abusive.

### 2.2 Why actions are not providers

A provider is a *request the program makes of the server* (`world_caps.rs:1-30`: "A program stays a pure function. What it needs from the server is data it returns (`wants`)"). Moderation actions differ in three ways that make them a separate surface:

1. **They act on other participants, not on the caller.** Every existing provider's effect is scoped to the requesting program or its caller (`required_scope` is `Join`/`Chat`/`ChatRead`). An action that revokes participant 7 on the strength of participant 3's report is not expressible as "the program asked for X".
2. **They must be attributable to a human.** The journal records `by: Option<u64>` (`mod.rs:1222`). A program has no participant id; the *moderator* does. So the action is executed **as** a moderator, on a library's recommendation.
3. **They are irreversible-ish and auditable.** Revocation cascades to everything delegated from the revoked capability (`handle.rs:646`). That is a governance act, and §3.4 requires it be reviewable after the fact.

**Decision.** Governance actions are a distinct command family beside `WorldCommand` (`Revoke` already lives there), not provider-catalog entries. They are declared in the library's manifest and bound to participants by config; the `RequiredScope` enum is not extended.

### 2.3 The declared action set (v1)

Deliberately small — four actions, each with an existing or trivially-adjacent substrate:

- `revoke(participant)` — exists today (`handle.rs:653`); the moderator authority path.
- `withhold(object)` — the world-state counterpart of writer-set "counting": the object stays stored (§1.5 rule 1, "stored ≠ counted"), the view stops counting it. This is the quarantine primitive.
- `restrict(participant, until)` — narrower than revoke: withdraw `CHAT` and/or `INPUT` for a bounded window without ending the session. Requires a new journal entry; named as an implementation slice (§9).
- `annotate(subject, note)` — a journaled moderator note attached to a participant or object; the audit primitive appeals read.

`ban` is **not** in v1: a ban is a revoke plus a re-entry refusal, and re-entry refusal is an invite-layer decision that belongs with the identity/invite design ([#5751](https://github.com/developing-today/code/issues/5751)). Naming it here would invent a capability the substrate does not have.

### 2.4 What a moderation library legitimately needs — the capability set

This is the question the issue asks directly, and the answer is the interesting one: **a moderation library needs almost no capabilities, and that is a feature.**

A moderation library's real inputs are governance events (reports, submissions, appeals), which arrive through the governance channel, not through `wants`. What it may legitimately hold:

| Need | Provider | Why it is legitimate | Ceiling note |
|---|---|---|---|
| read the content it moderates | `records.read` | a moderator must see what is reported | already a provider; bounded page size (`MAX_RECORDS_RESULT_PAGE_BYTES`) |
| know who is present | `players`, `players.list` | presence is the subject of most reports | default-ceiling included |
| read recent chat for context | `chat.history` | report context | requires `CHAT` or `HISTORY` (`mod.rs:301-314`) |
| time-bound a restriction | `time.now`, `time.tick` | `restrict(…, until)` needs a clock | default-ceiling included |
| notify the world | `chat.say` | "this post was removed" | requires `CHAT` |
| **not** network, **not** file, **not** process | — | a moderation library that can reach the network is an exfiltration surface; the 2026-11-04 prior-art finding (#6005) is that executable shared moderation is the failure mode | — |

**Decision.** A moderation library's capability footprint is a *subset* of the ordinary provider set, and the registry's `requested`/`discovered_used` pair ([#5958](https://github.com/developing-today/code/issues/5958)) reports it like any other library's. There is no `moderation.*` capability, so there is nothing for a malicious pack to request that a benign one would not. This is the strongest available answer to "what capability set does a moderation library need": **the same one a chat bot needs, and nothing more.**

---

## 3. Authority: who may moderate, and how that relates to writer-set authority

### 3.1 The three authorities, kept apart

| Authority | Where it lives | Granularity | Can world code hold it? |
|---|---|---|---|
| **Capability** (provider grant) | `wants` + ledger, per world | per provider | yes — that is the point |
| **Participant scope** | `WorldScopes` bits on a join capability | per participant | no — issued by invite/admin |
| **Directory level** | `Level` in a group (`directory.rs:33-46`) | per account, per group | no — directory membership |

Moderation lives in the second and third rows. The first row is what a library *is*; the other two are what it may *do*.

### 3.2 Resolving the two "moderator" notions

§1.2 found `WorldScopes::MODERATOR` (participant scope, revoke) and `Level::Moderator` (directory level, isolation/cross-world/file-scope) — same name, different mechanisms, no stated relation. That is a live ambiguity, not a design.

**Decision — the directory level is the *source*, the participant scope is the *grant*.** A world's governance config (§4) names the accounts/participants who may moderate. Issuing a join capability with `WorldScopes::MODERATOR` is how that authority reaches a specific session, and it stays admin-issued (never in `ALL`, never delegable — `mod.rs:179-182` unchanged). `Level::Moderator` in the directory remains the *account-level* authority for server-wide operations (isolation, file scope) and is not implied by holding a world's moderator scope, nor the reverse.

Consequence worth stating plainly: **a world host who is not a directory moderator can still appoint world moderators.** That is the whole point of "moderation is world code" — the world's own governance is not hostage to the server's group directory. The reverse is *not* true: a directory moderator does not automatically get revoke power in a world they do not administer, because revoke is a participant-scope act and scopes are per-join.

### 3.3 Relation to writer-set authority

Writer-set authority answers "**whose writes count in this view**" (#2030 / `doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`). Moderation answers "**may this participant keep acting at all**". They are orthogonal axes and must not be merged:

- A moderator **cannot** make an object uncounted by fiat in a way that rewrites history: the writer-set rule is "no silent history rewriting; set changes are signed, versioned, reviewable events" (§0 rule 5 of that record). So `withhold(object)` sets a *moderation* projection input, journaled, alongside the writer set — it does not mutate the writer set.
- A writer set **cannot** be used as a moderation tool. Removing someone from a writer set changes whose writes count; it does not stop them writing, chatting, or joining. Using it as a ban would be exactly the "authority gate at ingest" that record forbids.
- Where they meet: a view names its inputs — `(view config, writer-set version, frontier, resolution)` (that record §0 rule 3). **This record adds `moderation state` to that tuple**, so two people comparing counts can see whether a withhold is why they differ.

**Packet correction.** The packet cites [#5545](https://github.com/developing-today/code/issues/5545) as the live "writer-set authority" reference; #5545 is closed and was about view projection, not authority. The correct live reference is the writer-set design record above plus `membership/layer.rs`. Recorded in §8 as packet drift.

### 3.4 Audit invariants

Every governance action must satisfy, and implementation must test:

1. **Attributed** — journaled with the acting moderator's participant id (`Revoked { by: Some(..) }` is the existing shape).
2. **Replayable** — the action is a journal entry, so replay reconstructs the same state (the existing `Revoked` entry already documents "Replay revokes the same tree").
3. **Appealable** — every action that removes something has a `subject` and a reason string, so an appeal can name what it appeals.
4. **Attributable to a rule** — the journal entry names the library and config version that recommended it, so "why was I removed" has an answer that is not "the host felt like it".

---

## 4. Per-world parameterization

### 4.1 Config, not forks

The requirement is that the *same* moderation library runs in two worlds with different moderators and different thresholds, and that this be configuration. The parameter surface, v1:

```text
governance {
  library        <name>@<version>        # which moderation library
  moderators     [<account|participant>…] # who may act (§3.2 grant source)
  reviewers      [<account|participant>…] # who may approve submissions (§5)
  thresholds {
    report_quarantine   <n>              # reports before automatic withhold
    report_window       <duration>       # window the count is over
  }
  exemptions     [<…>]                   # local carve-outs, never in the shared core
}
```

Three properties this buys, each traceable to a requirement:

- **Same library, different moderators** — `moderators` is config, so the library binary is identical across worlds.
- **Same library, different threshold** — `report_quarantine`/`report_window` are config, so "automatic quarantine after N reports" is a number, not a code branch.
- **Local judgment is not surrendered** — `exemptions` sits *outside* the shared core, so adopting a shared pack never overrides a world's own carve-outs. This is the format property [#6005](https://github.com/developing-today/code/issues/6005) requires of the data-only pack (§6).

### 4.2 Format

The issue names the KDL profile ([#5753](https://github.com/developing-today/code/issues/5753)). **KDL is not yet in the tree** — `#5753` is open and parked, and no `kdl` dependency exists in `pkgs/id`. So this record specifies the parameter *surface* above and **does not commit to a serialization**; when #5753 lands, the surface maps onto its profile. Committing now would invent a format the programme has not chosen. The block above is illustrative shape, not a wire format.

### 4.3 Where the config lives

Per the world manifest apply/diff line (`doc/2026-11-03T00-05-09Z_research_phase-12-package-534/concepts/C001-…world-manifest-apply-diff.md`), governance config is part of the world's manifest, so changing moderators is a manifest change: diffable, reviewable, and journaled like any other manifest apply. That is what makes §3.4 invariant 4 achievable — the config has versions.

---

## 5. Pre-approval semantics

### 5.1 What a "design" is here

"Pre-approve specific designs" is ambiguous unless *design* is pinned. The three candidates the issue lists, resolved:

- **A world object submission** — content a participant wants in the world (a build, a post, a map). **This is what pre-approval binds in v1.**
- **A library** — approved as *code to attach*. Approval here is a capability-footprint decision, and it is the capability model's job ([#5958](https://github.com/developing-today/code/issues/5958)), not governance's. Governance may *reference* an approved library but does not re-approve it.
- **A user function** — a dynamic function (S-brief of #5955). Same answer as libraries: it is a capability-footprint decision.

So: **pre-approval in this record means submission approval.** Library and user-function approval are named as out of scope and delegated to [#5958](https://github.com/developing-today/code/issues/5958).

### 5.2 What approval binds

**Decision — approval binds a content hash and a capability set, and nothing weaker.**

- **Content hash**: the approval names the exact bytes approved. A name, a version string, or an author is not a binding — any of them can be reused for different content.
- **Capability set**: the approval names the `requested` set the approved content may run with. Approving content but not its footprint lets an approved submission widen itself after approval.
- **Not bound**: the submitter's identity (an approval is about content, not people), and the *time* (an approval does not expire, but it does not transfer to new content either).

Consequence: re-submitting changed content requires re-approval, because the hash changes. That is the intended friction — it is what makes "pre-approved" mean something.

### 5.3 Who approves, and what approval is not

Approvers are the config's `reviewers` (§4.1), distinct from `moderators`: a reviewer decides *may this enter*, a moderator decides *must this leave*. Worlds may set them to the same people; the design does not conflate them.

Approval is **not** a capability grant. An approved submission does not thereby receive providers; it runs under the world's ordinary grant policy. Approval is a *gate*, and the gate is the `submission approval` hook of §2.1.

---

## 6. Boundary with the boards moderation line

[#5680](https://github.com/developing-today/code/issues/5680) §4 and its split [#5861](https://github.com/developing-today/code/issues/5861) design **board-level** moderation: board roles (owner, moderator, member, viewer), board hook events (post created, post reported, embed state changed), report routing to board vs platform, and board audit/appeals. #5680 explicitly says "do not invent a parallel system: … consistent with #835's hook model."

**This record is the world-level generalization, and the relation is one-directional:**

- **Boards consume this interface; they do not restate it.** A board's "remove post" is this record's `withhold(object)`; a board's "ban a poster" is `revoke(participant)`; a board's audit log is §3.4's journal. #5861 should name these actions rather than define its own.
- **Boards add a role ladder this record does not.** Board roles (owner/moderator/member/viewer) are a *board-internal* permission model layered on top; the world governance config says who may moderate the world, the board says who may moderate *this board*.
- **What does not generalize downward**: board-specific hook events (embed state changed) are board surface, not governance. They do not enter §2.3's action set.

**Cross-pollination note for #5861**: the four v1 actions in §2.3 are the vocabulary to adopt; if #5861 needs a fifth (e.g. `lock(board)`), it should be filed as an extension of this action set, not a parallel one.

**Relation to [#6005](https://github.com/developing-today/code/issues/6005)** (moderation pack format): #6005 names #5973 as its parent topic issue and says its pack is "a data format #5973's interface design can adopt". The fit is exact — §4.1's config surface *is* the parameter block a pack carries, and §4.1's `exemptions` is the "kept separate from the shared core" property #6005 requires. #6005 owns the serialization; this record owns the semantics of each field.

---

## 7. Open questions (deferred with reasons)

- **Report transport.** How does a report reach the world from a participant? No provider exists for it. Deferred: it is a new provider or a new channel, and choosing between them is a capability-model decision ([#5958](https://github.com/developing-today/code/issues/5958) owns the vocabulary). Default if forced: a governance event channel parallel to `wants`, not a provider.
- **Appeal adjudication.** Who hears an appeal when the moderator *is* the accused's accuser? §3.4 makes appeals *possible* (attribution + reason); it does not design the adjudication ladder. Deferred: needs a decision about whether appeals can escalate to the server admin, which is a Drew-level policy question (§8).
- **Cross-world moderation.** A shared pack used by two worlds: does a revoke in one inform the other? Deferred — this is the feed-protocol territory of #6006 (closed) and #6005, and answering it here would duplicate them.
- **`restrict` journal shape.** Named as an implementation slice (§9), not designed here.

## 8. Packet drift and escalations

- **Packet cites #5545 as live "writer-set authority"; #5545 is CLOSED** (verified 2026-11-08). Corrected in §3.3 to the writer-set design record + `membership/layer.rs`. Not blocking — the concept is live, only the issue reference is stale.
- **Packet says "no governance design doc landed"** — confirmed: `git ls-tree origin/main doc/` shows no governance/moderation design record before this one.
- **Escalation candidate (not escalated in this task):** §7's appeal-adjudication question ("can an appeal escalate past the world to the server admin?") is a policy decision that is genuinely the owner's. It is recorded here with the default (no escalation in v1; the world's decision is final) rather than escalated, because v1 is coherent without it.

## 9. Implementation slices (not built here)

Per the programme rule ([epic #2056](https://github.com/developing-today/code/issues/2056)), a spec is not a build order. Slices, each to be filed as its own issue when this record is accepted:

1. **Governance action command family** — `WorldCommand` variants for `Withhold`, `Restrict`, `Annotate` beside `Revoke`; journal entries with `by`, `reason`, `library`, `config_version`.
2. **Governance config in the world manifest** — parse §4.1's surface; journal config versions.
3. **Moderator grant path** — issuing `WorldScopes::MODERATOR` from the config's `moderators` list without widening `ALL` or enabling delegation (§3.2).
4. **Submission approval gate** — content-hash + capability-set binding (§5.2), journaled.
5. **Report intake channel** — pending §7's transport decision.
6. **View tuple extension** — add `moderation state` to the named view inputs (§3.3).

## Revision history

- 2026-11-08: Initial design record for #5973 (queue task 12862, lane bf419306-c4ef-470f-a553-beabf93239de, openchamber scheduled lane). Verified substrate at `83b7a024b8`; settled the moderation-as-library interface (§2), the authority model and writer-set relation (§3), parameterization (§4), pre-approval (§5), and the boards boundary (§6); recorded packet drift (#5545 closed) and six implementation slices.
