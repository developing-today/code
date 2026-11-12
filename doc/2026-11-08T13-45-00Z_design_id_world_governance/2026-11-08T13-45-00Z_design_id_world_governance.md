# Design: world governance — moderation-as-library, per-world parameterization, pre-approval

- Date: 2026-11-08
- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Issue: [#5973](https://github.com/developing-today/code/issues/5973), child of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955) (statements S75–S76 of the 2026-11-04 brief).
- Specified against: `origin/main` `69aa168d5d`, read 2026-11-08. Citations below are re-checked against this revision; the draft began at `83b7a024b8`.
- Coordinates with, never re-defines: the requested/used capability vocabulary owned by [#5958](https://github.com/developing-today/code/issues/5958), the release/instance fields (`requested`, `discovered_used`, `effective_grant`, `observed`) documented by [#6017](https://github.com/developing-today/code/issues/6017), and the audience vocabulary owned by [#5960](https://github.com/developing-today/code/issues/5960). Their records are cited in §1 and §5. Where this record names a capability, it means an ordinary provider-registry entry in `world_caps.rs`, not a new moderation privilege.

## Decision in brief

Moderation is **world code over the ordinary capability surface**, not a platform feature. Concretely:

1. **There is no moderation privilege in the ordinary provider catalog.** At this revision the catalog has 19 entries: 13 request/subscription providers (`time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`, `world.info`, `loc.position`, `loc.watch`) plus six platform modules (`Key`, `Screen`, `RocJson`, `RocGrid`, `RocCap`, `RocText`) in `world_caps.rs:346-502`. None grants moderation authority. The six modules do not turn the catalog into an authority channel.
2. **Provider grants and moderation authority stay separate.** The existing world `MODERATOR` participant scope is admin-issued and currently authorizes participant revocation (`pkgs/id/src/world/mod.rs:174-186`; `handle.rs:647-665`; `mod.rs:1222-1230`). Separately, directory `Level::Moderator` is accepted for server-level changes to isolation, cross-world writes, and file scope (`pkgs/id/src/world/actor.rs:1103-1179`; `pkgs/id/src/directory.rs:1112`). Neither set of powers is a `wants` capability, and the directory role is not a world-scoped moderator appointment.
3. **The library is a policy module, not an actuator.** It receives typed governance inputs and per-world configuration, then returns a typed decision or recommendation. A platform adapter validates the decision against the configured action scope and authority, performs the permitted world-scoped effect, and records the result. The library cannot mint privileges or bypass those checks.
4. **Parameterization is configuration of one library, never a fork** — moderator/reviewer sets and thresholds are per-world settings of the same code, which is what makes "literally the same thing from a different world, with different moderators and a different threshold" true rather than aspirational.
5. **Pre-approval binds an exact artifact and its requested capability set**, not a name or version string. It authorizes only that artifact for that world; it is not a capability grant.

The rest of this record settles the four things the issue asks for: the interface (§2), the authority model and its relation to writer-set authority (§3), parameterization (§4), pre-approval (§5), and the boundary with the boards line (§6).

---

## 1. Verified current state

Read at `69aa168d5d` (origin/main at this correction pass; the original draft pinned `83b7a024b8` and its citations were re-checked against the current revision). This section is the evidence base; every later decision cites it.

### 1.1 Participant scopes — the moderator bit exists and is admin-only

`WorldScopes` is a `u8` bitset (`pkgs/id/src/world/mod.rs:158`). Bits:

| Scope       | Bit          | Meaning (`mod.rs:161-176`)                                      |
| ----------- | ------------ | --------------------------------------------------------------- |
| `JOIN`      | `0b001`      | join the world, receive snapshots/events                        |
| `CHAT`      | `0b010`      | send chat                                                       |
| `INPUT`     | `0b100`      | submit game input                                               |
| `DELEGATE`  | `0b1000`     | mint capabilities with a subset of this set                     |
| `HISTORY`   | `0b1_0000`   | read chat without sending                                       |
| `BUILDER`   | `0b10_0000`  | install and upgrade the world's program — _admin authority bit_ |
| `MODERATOR` | `0b100_0000` | **revoke participants** — _admin authority bit_                 |

Two facts matter for governance:

- `ALL` deliberately **excludes** `BUILDER` and `MODERATOR` (`mod.rs:179-182`): "they are issued only by the admin, never implied by a ceiling or handed out by a delegator." `ALL_BITS` (`mod.rs:186`) includes them only so capability round-trips do not widen `ALL`.
- `from_role("moderator")` yields `GUEST | MODERATOR` (`mod.rs:209-219`), so a moderator keeps the ordinary guest powers and adds revoke. `ROLES` lists seven presets (`mod.rs:222-231`).

### 1.2 Existing moderation authorities are distinct and narrower than this design

- **Directory authority**: `require_moderation` (`actor.rs:1103`) permits `Authority::Admin` or a signed key whose account is a directory moderator (`directory.rs:1112`). It guards server-level changes to isolation, cross-world writes, and file scope (`actor.rs:1121-1179`); it does not appoint moderators within a world.
- **World participant scope**: `WorldScopes::MODERATOR` is an admin-issued bit for revoking a participant's world capability. `revoke_by` records the acting participant in `JournalEntry::Revoked` (`handle.rs:651-666`, `mod.rs:1220-1229`). It is not currently a configurable world-moderator roster.

These are separate existing authorities, not two halves of one role. §3 specifies the proposed world-scoped policy authority and the implementation gap explicitly.

### 1.3 The roles record already deferred exactly this work

`doc/2026-10-17T08-00-00Z_feature_world_roles_directory/…md` states: "Roles that would need new authority (a builder who installs modules, **a moderator who kicks**) stay future work: those are admin-token powers today, and handing them to a capability needs its own design." The `MODERATOR` bit and `BUILDER` bit have since landed, but the _governance_ design that says who may hold them, and how a world delegates moderation without handing out admin, is what this record supplies.

### 1.4 There is no moderation provider, and adding one would be the wrong move

The current catalog contains 19 entries (`world_caps.rs:219-247,346-502`), each manifesting a provider or platform module. `RequiredScope` has only `Join`, `Chat`, and `ChatRead` (`world_caps.rs:188-198`); it has no moderation authority. A `moderation.hide` provider would either be a grantable capability or require smuggling authority into the provider scope model. Neither is the design here; §2 makes the policy-module/authority-adapter split explicit.

### 1.5 Writer-set authority is a _view_ concept, and its implementation issue is closed

The writer-set design record `doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/` establishes: "ingest is a validity gate, never an authority gate; every object that verifies is stored; a view counts only its writer set." Issue [#5545](https://github.com/developing-today/code/issues/5545) is **CLOSED** because its v1 view-projection/discovery implementation landed in PR #6672; the issue and design remain the relevant source, not a live authority-grant mechanism. The follow-on writer-set v2 record (`doc/2026-11-08T11-10-00Z_design_id_writer_sets_v2/`, extends #5545) retains the same separation. See §3.3: writer-set membership never grants moderation authority.

---

## 2. The moderation-as-library interface

### 2.1 The shape: policy module plus platform adapter

A moderation library is world code that consumes typed governance inputs and per-world configuration, then returns a typed decision or recommendation. It does **not** execute moderation effects itself. The platform adapter owns event transport, authority checks, state mutation, and journaling. The four hooks named by the issue map onto that seam:

| Hook                    | Direction                                                    | Mechanism owner                                                                     | Policy / decision owner                                                            |
| ----------------------- | ------------------------------------------------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| **submission approval** | platform → library (request) → platform (decision)           | platform: submission intake, validation, and journal                                | library evaluates the world's rules; §5 binds the result to the exact artifact     |
| **report intake**       | platform → library (event)                                   | platform: world-scoped report transport, rate limits, deduplication                 | library rules plus per-world thresholds                                            |
| **quarantine**          | library → adapter (recommendation) → platform (effect)       | adapter validates action scope and authority, then applies a permitted world effect | library recommends; configured authority decides or a narrow automation rule fires |
| **appeal**              | participant → platform → library (recommendation) → reviewer | platform: appeal intake and journal                                                 | library may recommend; an independent configured reviewer decides                  |

The split is: **mechanism and authority are platform-provided; policy is world-provided.** The library may not directly invoke a privileged API, mint a role, hide content, or revoke a participant. An automated outcome is possible only when the world config explicitly grants that narrow effect to a named rule and the adapter records it as a policy action. By default, quarantine is a recommendation for an authorized reviewer, not an action the library can take.

World report transport is a platform primitive, not a provider the library implements with network or storage. It is distinct from the boards report path: #5861 decides board reports are host-local and not replicated (§6). There is no implicit forwarding between board, world, and platform report destinations.

### 2.2 Why effects are not providers

A provider is a request the program makes of the server (`world_caps.rs:1-30`: a program stays a pure function and receives only the data returned for its `wants`). Moderation effects differ in three ways that make them a separate surface:

1. **They act on other principals or shared world state.** Revoking a participant because of a report is not an ordinary request made by the library for itself.
2. **They require independent authorization.** A policy result is not proof that the caller may execute it. The adapter must check a current world-scoped actor grant or an explicit automatic-action rule before applying it.
3. **They are consequential and auditable.** Revocation cascades to the revoked participant's delegated capabilities (`handle.rs:646-666`); a world projection withhold changes what the world shows. Both need attribution, replay, and appeal semantics.

**Decision.** Governance effects are a distinct command family beside `WorldCommand` (`Revoke` already lives there), not provider-catalog entries. A library may declare its decision schema; the world config binds decision kinds to permitted actors and thresholds; the adapter validates and executes only those decisions. `RequiredScope` is not extended with moderator authority.

### 2.3 The declared world-effect set (v1)

The library returns recommendations naming one of these effect kinds; an authorized human or a specifically configured automatic rule decides whether one is executed:

- `approve(artifact_hash, requested_caps)` / `reject(artifact_hash, reason_code)` — submission-gate outcomes, not capability grants.
- `withhold(world_object, reason_code)` — stop counting the object in this world's moderation projection while retaining the signed/stored object and provenance. This is the quarantine primitive and requires a new world command and journal entry.
- `restrict(world_participant, scopes, until)` — withdraw `CHAT` and/or `INPUT` for a bounded period without ending the session; requires a new journal entry.
- `revoke(world_participant, reason_code)` — uses the existing world revocation path and is high impact; v1 requires an authorized human actor, not an automatic report threshold.
- `annotate(subject, reason_code)` — attach structured, appealable rationale to the world moderation record; it is not a free-text execution channel.

A global `ban` is not in v1. A world revoke does not prevent a principal joining another world; any re-entry policy is world-specific, while identity/invite mechanics remain with [#5751](https://github.com/developing-today/code/issues/5751). No action here applies to a board's signed graph or another world.

### 2.4 What a moderation library legitimately needs — its capability set

**Default: no additional ordinary provider capabilities.** Submission, report, and appeal arrive as bounded governance events with a world-local target reference, principal reference, trusted event time, relevant public/published presentation, and the governing config/policy version. The library's policy function evaluates that input; it does not need a clock, an all-world content reader, presence enumeration, chat posting, or external I/O merely to decide.

| Capability / input                                          | Default                                | Why / constraint                                                                                                                                                                                                                             |
| ----------------------------------------------------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Typed submission, report, appeal events                     | platform event channel, not `wants`    | Carries only data the world/report audience is entitled to see; includes trusted event time and target references                                                                                                                            |
| `records.read`                                              | **not granted by default**             | It is a broad ordinary provider today. A moderation library may receive it only by explicit world grant and only after the read path enforces the relevant audience/target scope; a bounded page size alone is not an authorization boundary |
| `chat.history`                                              | **not granted by default**             | Add only when a world explicitly needs chat context and the reader has `CHAT`/`HISTORY` (`mod.rs:300-314`); prefer a bounded, authorized context in the event                                                                                |
| `players` / `players.list`                                  | **not granted by default**             | Presence is not required for content policy; add only for a declared, world-specific rule                                                                                                                                                    |
| `time.now` / `time.tick`                                    | **not granted by default**             | Threshold windows use the trusted event timestamp; a runtime clock is not policy authority                                                                                                                                                   |
| `chat.say`                                                  | **not granted by default**             | Notices are a separate platform/UI effect; a moderator policy does not need chat-write authority                                                                                                                                             |
| `loc.position` / `loc.watch`                                | **not granted by the default profile** | Sensor providers are excluded from default ceilings and use a consent seam; moderation has no location need                                                                                                                                  |
| Network, arbitrary file, process, or moderation effect APIs | **not available**                      | No exfiltration or privilege escape; effects are returned as typed data and checked by the adapter                                                                                                                                           |

If a world explicitly grants an optional provider, the library's ordinary capability record remains subject to #5958's `requested` / `discovered_used` model and #6017's `effective_grant` / `observed` reporting. These are measurements and grants, not moderation authority. The legitimate default capability set is therefore **empty beyond the event input**; optional read/context providers are explicit, minimal, and audience-scoped.

---

## 3. Authority: who may moderate, and how that relates to writer-set authority

### 3.1 Authorities that must remain separate

| Authority                                        | Where it lives                                            | Granularity                         | Can the library hold it?                                                         |
| ------------------------------------------------ | --------------------------------------------------------- | ----------------------------------- | -------------------------------------------------------------------------------- |
| **Capability** (provider grant)                  | release `requested` plus world-instance `effective_grant` | per provider and world              | No; providers are inputs/effects of the program, not action authority            |
| **World participant scope**                      | `WorldScopes` bits on a join capability                   | per world participant/session       | No; issued by the platform and checked by the adapter                            |
| **Directory level**                              | `Level` in a group (`directory.rs:33-46`)                 | account-level server authority      | No; directory membership                                                         |
| **World governance appointment** (this proposal) | signed, versioned world governance config                 | per world, actor, and decision kind | No; the config and adapter—not library code—bind and enforce it                  |
| **Writer-set inclusion**                         | signed view/set objects                                   | per view and causal frontier        | No; it controls which valid writes count in that view, not who governs the world |

The library receives only its ordinary, explicitly granted inputs and returns policy data. It neither holds nor derives participant, directory, world-governance, or writer-set authority.

### 3.2 Proposed world-scoped authority

Today `WorldScopes::MODERATOR` is admin-issued and can revoke a participant; `Level::Moderator` authorizes certain server-level settings changes. Neither is a configurable per-world role roster, and they are not interchangeable.

**Decision.** A world governance config names stable principals and the actions each may authorize within that world. The config itself is signed by the world's governance owner/authorized manifest writers; changing it is a versioned governance change. A world-scoped moderator appointment does not imply directory `Level::Moderator`, and directory moderator status does not imply appointment in any world. At action time, the platform adapter checks the current config and actor proof, then records actor, action, target, reason code, library/policy version, and config version.

This creates an implementation gap that must stay visible: the existing admin-issued `WorldScopes::MODERATOR` path does not implement config-driven appointments. The implementation must add a world-scoped authorization path or explicitly reconcile the existing scope; it must not silently widen `WorldScopes::ALL`, let libraries issue moderator scopes, or infer authority from directory level. Until that path exists, this is a design, not implemented behavior.

### 3.3 Relation to writer-set authority

Writer-set authority answers **whose signed writes count in a particular view** (`doc/2026-10-30T13-15-00Z_design_id_writer_set_view_projection/`, extended by `doc/2026-11-08T11-10-00Z_design_id_writer_sets_v2/`). Moderation answers **which world-scoped effects are permitted and which objects this world's moderation projection counts**. They are orthogonal and must not be merged:

- **No ingest gate or history rewrite.** A validly signed object remains stored and attributed even when a world moderation decision withholds it from that world's projection. Moderation does not invalidate the signature, rewrite the writer set, or erase the object.
- **No authority through membership.** Adding a principal to a writer set does not appoint them as moderator or authorize governance-config changes. Removing a principal from a writer set changes what a view counts; it does not revoke their ability to write, chat, or join. Conversely, a world moderation appointment does not make that principal's writes count in a view.
- **Separate, named inputs.** The existing view inputs include view config, writer-set version, frontier, and resolution (writer-set v1 §0 rule 3). A world view that applies moderation must additionally name the applicable world-moderation projection/config version. It remains a separate signed input, not a hidden rewrite of the writer set. This addition applies to world views only; it does not alter the board projection in #5861.

**Source correction.** The packet and #5973 body point to [#5545](https://github.com/developing-today/code/issues/5545) as "writer-set authority." #5545 is closed because its v1 view-projection/discovery implementation landed in PR #6672; it is not a moderator-role grant. The v1 and v2 design records above are the relevant sources for the interaction specified here. The idea that ingest is validity-only and views decide counting remains current.

### 3.4 Audit invariants

Every governance outcome must satisfy, and implementation must test:

1. **Attributed** — human effects name the acting principal; automated effects name the policy/library version and rule that fired.
2. **Replayable** — the decision/effect is journaled with the world, config, and policy versions needed to reconstruct the same state.
3. **Appealable** — an outcome that removes or restricts something names its target and structured reason code; appeal input is separately retained under its privacy policy.
4. **Authority-checked** — the journal proves the adapter checked the actor's current world-scoped authority or the exact configured automatic-action rule.
5. **Projection-safe** — a withhold changes only the world moderation projection; it does not mutate signed content, the writer set, board state, or another world's state.

---

## 4. Per-world parameterization

### 4.1 One configured library/component, not a fork

The requirement is that the same library or implementation can serve different worlds with different reviewer/moderator sets and different report thresholds, purely by configuration. The parameter surface, v1:

```text
governance {
  library        <name>@<version>            # policy implementation, same code across worlds
  policy_rules   <policy_version_or_hash>    # exact rule bundle, independent of moderator names
  reviewers      [<principal-ref>…]          # who may decide submission approval
  moderators     [<principal-ref>…]          # who may authorize restrictive effects
  decision_authority {
    withhold     recommend | moderators     # automation is opt-in, never the default
    restrict     moderators
    revoke       moderators
    approval     reviewers
  }
  thresholds {
    report_quarantine  <n>                   # reports needed before a quarantine recommendation
    report_window      <duration>            # window over trusted event time
    appeal_window      <duration>
  }
  exemptions     [<rule-or-subject-ref>…]    # local carve-outs, outside the shared core
  report_route   world-local                 # v1; no implicit forwarding to board/platform
}
```

Four properties this buys, each traceable to a requirement:

- **Same policy, different people.** Moderator and reviewer sets are configuration; the library code and policy rules are identical across worlds.
- **Same library, different threshold.** `report_quarantine`/`report_window` configure when a _recommendation_ exists; they do not by themselves authorize an effect.
- **Automation is explicit and narrow.** Only `withhold` may be set to `recommend` or `moderators`; v1 never lets a threshold alone revoke a participant. A report count is evidence, not a verdict.
- **Local judgment is not surrendered.** `exemptions` is outside the shared core, so adopting a shared policy bundle does not override the world's carve-outs. This is the property [#6005](https://github.com/developing-today/code/issues/6005) requires of a data-only pack.

### 4.2 Format

The issue names the KDL profile ([#5753](https://github.com/developing-today/code/issues/5753)). That work is **open and blocked**, and there is no `kdl` dependency in `pkgs/id` at this revision. This record therefore specifies the parameter _surface_ above and **does not commit to a serialization**. When #5753 lands, the surface maps onto its profile. The block above is illustrative shape, not a wire format.

### 4.3 Where the config lives, and how it becomes authoritative

Governance config belongs to the world manifest, consistent with the world-manifest apply/diff design (`doc/2026-11-03T00-05-09Z_research_phase-12-package-534/concepts/C001-d1-019-f12-1-world-manifest-apply-diff.md`). Changing reviewers, moderators, thresholds, or policy rules is therefore a manifest change: diffable, reviewable, and journaled. The config has versions, which is what §3.4 invariant 2 needs.

Two limits keep "config, not a fork" from becoming "config can grant anything":

1. **Config can narrow, not invent.** It may choose reviewers/moderators and thresholds for the effect kinds the library exposes. It cannot add a new privileged effect, extend a decision beyond that world, or grant a capability absent from the installation's ordinary grant policy.
2. **Config changes are authority checked.** Only a principal the current governance config/manifest recognizes as an authorized writer may change it. Bootstrapping the first governance config is an explicit world-creation/admin step, not something a moderation library can do for itself.

---

## 5. Pre-approval semantics

### 5.1 What "a design" means here

"Pre-approve specific designs" needs its object pinned. The issue's three candidates resolve differently:

- **A world object submission** — content or a build a participant wants in this world: a map, a post, a model, or an attachable artifact. **This is the object pre-approval binds in v1.**
- **A library** — approved as code to attach. That is the capability-footprint/attachment decision owned by the capability model ([#5958](https://github.com/developing-today/code/issues/5958), with observability in [#6017](https://github.com/developing-today/code/issues/6017)). Governance may _reference_ an approved library and may require that approval before attaching it, but does not re-approve its capability set.
- **A user function** — a dynamic function (the S-brief of #5955). Same answer as libraries: capability-footprint approval, not this submission gate.

So **pre-approval in this record means world submission/artifact approval**. Library and user-function capability approval are explicitly out of scope and delegated; their designs remain unsettled.

### 5.2 What approval binds

**Decision — an approval binds an exact artifact, its requested capability footprint, and the approving world's governance/policy versions. Nothing weaker.**

- **Artifact identity:** a content hash over the exact bytes or canonical content-addressed object the world will accept. A name, author, version string, or mutable reference is not a binding; any of them can point at different content later.
- **Requested capability footprint:** the exact `requested` set kind/ceiling/contract required by the approved artifact, as its ordinary capability record. Approving content without its footprint lets an approved thing widen itself after approval.
- **World and policy context:** the approval names the world and the governance config/policy version under which it was granted. A different world's approval, or a later policy change that redefines the gate, must not silently inherit it.
- **Approver authority snapshot:** who approved, under which reviewer authority, with a verifiable reference. Approval is an authorization event, not a rename of the artifact.

**Not bound:** the submitter's identity as a condition of content quality, and perpetual validity. An approval does not transfer to changed content; re-submitting different bytes requires re-approval. Expiry/revocation policy is an explicit governance choice, not hidden in the hash.

**Decision — approval is necessary, not sufficient for execution.** An approved artifact still runs under the world's ordinary capability grant. Approval itself grants no provider, expands no ceiling, and is not an input to `effective_grant` except as a policy gate the world may consult.

### 5.3 Who approves, and what approval is not

Approvers are the config's `reviewers` (§4.1), distinct from `moderators`: a reviewer decides _may this artifact enter this world_, a moderator decides _must this effect remove or restrict something_. Worlds may appoint the same principals to both, but the decision kinds remain distinct.

Approval is **not** a capability grant, a signature by the submitter, a transfer of ownership, or a waiver of the artifact's own read/audience rules. It is one named gate, evaluated by the library and recorded by the adapter. The world retains its ordinary rights to withhold an approved object later through §2.3/§3.4.

---

## 6. Boundary with the boards moderation line

The boards line has now settled its own decision: [#5680](https://github.com/developing-today/code/issues/5680) is the board-surface parent, and [#5861](https://github.com/developing-today/code/issues/5861) closed with the merged decision record `doc/2026-11-08_design_id5861.md`. This section cross-pollinates that record; it does not restate or override it.

### 6.1 Two authority domains, deliberately not joined

Board moderation operates on the **signed public board graph and its default projection**. World governance operates on **a world instance's participants and world-state projection**. They share vocabulary and audit principles, but not principals, actions, or storage:

- **A board view-ban is not a world revoke.** #5861 §3 defines view-ban as hiding that key's posts in that board's default projection; it explicitly cannot prevent publishing or erase signed history. World `revoke(participant)` ends a world capability and cascades delegated world capabilities, but grants nothing on any board.
- **A board role is not a world moderator appointment.** #5861 §2 uses owner-signed, board-scoped `RoleAttestation`s. This record's world appointments are world-scoped config entries. Neither transfers to the other, and #5861's I2 already states that a role/action on one board grants nothing on another board, host, or world.
- **A board hide is not a world withhold.** Both are projection-only, but over different projections with different inputs, owners, and journals. They may use similar audit language; they must not share a mutable authority path.

### 6.2 What this record takes from #5861

#5861's corrections are adopted here, not contradicted:

- **Hook restraint.** #5861 I4/§4 bars board hooks from hiding, removing, restricting, banning, freezing, locking, granting roles, or deciding appeals; an authorized human signs a separate action. §2.1 adopts the same principle for world policy: the library recommends/decides policy, and an authority-checked adapter executes only permitted effects. This record's opt-in, non-default automation is the narrow exception; it is a configured world action, never an arbitrary hook.
- **Projection, not deletion.** Withhold follows the same principle as a board hide: the signed/stored object remains; only the named projection changes.
- **Append-only, attributable outcomes.** §3.4 mirrors #5861's audit invariants: no in-place rewrite, no silent history rewriting, and every action carries authority evidence.
- **Separate report destinations.** #5861 §5 makes board reports host-local/private and distinct from platform reports, with no auto-copying. This record's world report route is likewise world-local/private in v1, and it does not forward implicitly to a board or platform destination.
- **No global arbiter by default.** #5861 §6 has no platform-wide board arbiter; this record likewise leaves appeal escalation as an explicit world policy decision (§7), not an assumed power.

### 6.3 What the boards line may take from this record

- The **policy-module plus authority-adapter split**: a hook/policy component can recommend or evaluate, while an authorized actor/action performs the effect. That is compatible with #5861's hooks-cannot-act invariant.
- The **approval binding shape** (§5.2) for board submission/curation flows: exact artifact, requested footprint, and board/policy context. Board-specific approval remains #5680's to design.
- The **per-world parameterization distinction** (§4.1): a shared policy bundle plus local reviewer/moderator lists, thresholds, and exemptions. The same discipline applies to board policy packs; the board record owns their serialization.

### 6.4 What does not generalize between them

- **No shared action vocabulary.** #5861's Hide/Restore, label, pin, lock, freeze, and view-ban are board projection actions. §2.3's world effect set is separate. Neither record should force the other's action names into its schema.
- **No shared scope.** A world moderator cannot moderate a board, and a board moderator cannot revoke a world participant merely by holding a board role.
- **No shared storage or automatic replication.** Board graph events are board-object events; world governance records belong to the world instance. Cross-domain automation would need a new, explicit design and consent.
- **Board-specific hook events** (embed state changes, post presentation changes) do not enter §2.3.

### 6.5 Relation to #6005 (moderation pack format)

[#6005](https://github.com/developing-today/code/issues/6005) names #5973 as its parent topic and describes a data-only pack. The fit is precise if the boundary stays data/policy:

- #6005 owns the pack's serialization, schema, versioning, provenance, and signed-feed/override mechanics.
- This record owns the semantics a pack's fields must mean when applied to a world: library identity, policy-rule version, reviewer/moderator authority bindings, thresholds, decision-authority policy, exemptions, and report routing.
- A pack is **data, not an execution surface**: installing/adopting one changes configuration only. It cannot install code, grant a capability, or create a moderation power the platform does not already expose. This is both the prior-art lesson in #6005 and the #5861 hooks-cannot-act lesson applied to packs.

---

## 7. Open questions (deferred with reasons and defaults)

- **World report transport and privacy.** World reports need a platform intake path with retention, audience, and anti-abuse limits. Deferred: this is a world-instance transport and data-retention design, not a provider and not a board report. Default for v1: world-local/private intake with structured fields, trusted receipt time, bound retention, and no implicit forwarding. Filed as an implementation slice (§9).
- **Appeal adjudication and escalation.** Who hears an appeal when the moderator is the accuser? §3.4 makes outcomes attributable and appealable; the adjudication ladder is not designed here. Default for v1: an independent configured reviewer; no automatic escalation to a server admin, and no platform-wide arbiter. Owner policy may later add a server-level escalation.
- **Cross-world moderation signals.** Whether a shared pack or policy decision in one world informs another is feed-protocol territory (#6005, and the closed #6006). Default: no automatic cross-world effect; any sharing is advisory data adopted by each world's own config.
- **Automated withhold:** v1 allows a configured threshold to create a _recommendation_, and permits automatic execution only when `decision_authority.withhold` explicitly opts in with a bounded action (for example, withhold from the default projection pending review). Revocation is never automatic. The exact automatic-action journaling shape is an implementation slice.
- **Legacy admin-issued moderator scope:** whether config appointments will reuse `WorldScopes::MODERATOR`, add a distinct world-governance authorization, or migrate the scope requires implementation review; this record deliberately does not prescribe it beyond the non-widening invariants in §3.2.

## 8. Source and packet drift

- **#5545 is CLOSED, not live authority.** Verified 2026-11-08. Its v1 implementation (PR #6672) landed, and it covered view projection/discovery, not moderator authority. §3.3 uses the v1/v2 writer-set design records instead. This restates the issue body's wording, not just the packet's.
- **#5861 is CLOSED-completed**, with `doc/2026-11-08_design_id5861.md` merged via PR #7857. This record's §6 is written against that landed record, not against the issue as if it were still open. Its view-ban and hooks-cannot-act decisions are treated as binding for boards.
- **#5680 remains OPEN** as the board-surface parent; #5861 resolves the moderation part. This record does not claim #5680 is closed.
- **#5753 is OPEN and blocked**, not merely parked; the KDL serialization decision remains deferred for that reason (§4.2).
- **Catalog count corrected.** The provider registry has 19 entries (13 request/subscription providers plus six platform modules), not 13. The conclusion that none is a moderation authority is unchanged.
- **#5958/#6017 division of fields.** #5958 owns requested/used capability semantics; #6017 documents the release/instance field set (`requested`, `discovered_used`, `effective_grant`, `observed`) and the capability event log. This record cites both rather than attributing the four-field schema to #5958 alone.
- **No governance design record preceded this one.** The merged `doc/2026-11-08_design_id5861.md` is a board-moderation record, not a world-governance record; it does not close #5973's scope.
- **Escalation candidate, not escalated:** appeal escalation past the world is genuinely an owner policy question. The default is recorded in §7 (no automatic escalation; no platform-wide arbiter), and v1 is coherent without it.

## 9. Implementation slices (not built here)

Per the programme rule ([epic #2056](https://github.com/developing-today/code/issues/2056)), a design is not a build order. Slices, each to be filed as its own issue when this record is accepted:

1. **World-governance authorization and config versioning** — signed manifest/config schema for §4.1, actor/action checks, version journaling, and the reconciliation plan for the legacy admin-issued `WorldScopes::MODERATOR` path (§3.2).
2. **World effect command family** — `WorldCommand` variants and journal entries for `Withhold`, `Restrict`, and `Annotate` beside `Revoke`, with target, actor, reason code, library/policy version, and config version. No automatic `Revoke`.
3. **Policy-module seam and adapter validation** — typed governance-event input, typed recommendation output, and adapter enforcement that rejects effects outside the configured action scope.
4. **World report intake** — bounded, private, world-local transport and retention with trusted timestamps and no implicit forwarding (§7).
5. **Submission approval gate** — exact artifact hash plus requested-footprint/world/policy binding (§5.2), with journaled reviewer authority.
6. **World-view projection input** — name the world moderation projection/config version as a separate view input for world views; do not alter board projections (§3.3).
7. **Appeal review workflow** — independent-reviewer assignment, structured outcomes, and retention for private appeal context (§7).

## Revision history

- 2026-11-08: Initial design record for #5973 (queue task 12862, lane bf419306-c4ef-470f-a553-beabf93239de). Settled moderation-as-library as a policy-module/authority-adapter seam, per-world parameterization, artifact pre-approval, and the boards boundary.
- 2026-11-08 (task 12862 correction pass): updated the pinned revision to `69aa168d5d`; corrected the catalog count to 19; separated world moderation from board view-ban and directory authority; aligned §6 with the merged #5861 record; changed the library default capability footprint from "a few providers" to "event input only"; tightened approval binding and automatic-action limits; refreshed relation statuses (#5545 and #5861 closed, #5680 and #5753 open/blocked) and the #5958/#6017 attribution.
