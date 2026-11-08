# Design note: reserved admin authority — what LambdaMOO's governance arc says id must keep off the ballot

- **Date:** 2026-11-08
- **Status:** design note for review; no implementation is included here.
- **Issue:** [#6004](https://github.com/developing-today/code/issues/6004), wave-2 follow-up (Lane B item 23) of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955). Parent topic: [#5973](https://github.com/developing-today/code/issues/5973) (world governance — per-world rules, pre-approval, moderation as shared libraries).
- **Source of the case:** `workspace/scratch/worlds-capabilities-brief-2026-11-04/prior-art-research.md`, Cluster 3 (LambdaMOO), Lane B item 23 of the follow-up list.
- **Siblings this note composes with, and does not re-decide:** [#5962](https://github.com/developing-today/code/issues/5962) (server-side invariants), [#6006](https://github.com/developing-today/code/issues/6006) → [`design_threat_list_feed_protocol`](../2026-11-05T13-36-21Z_design_threat_list_feed_protocol/2026-11-05T13-36-21Z_design_threat_list_feed_protocol.md) (feeds as advice), [`design_boundary_audit_appeals`](../2026-11-08T09-15-00Z_design_boundary_audit_appeals.md) (decision log, review queue, appeals), [`design_per_user_function_mapping`](../2026-11-08T10-26-00Z_design_id_per_user_function_mapping/2026-11-08T10-26-00Z_design_id_per_user_function_mapping.md) (fuel fairness), and the shipped capability substrate in `pkgs/id` (`WorldScopes::BUILDER` / `MODERATOR`, the admin token, world capability ceilings, `boards.rs` moderation log).

## The decision this note argues for

**Reserve a named, small set of authorities to the server/world admin from day one, and make the rest of governance delegable — because LambdaMOO ran the experiment of doing it the other way first, and had to take the authority back.**

LambdaMOO, 1990–1996, is the only documented full arc of a programmable world trying pure community governance:

1. **Wizards held everything.** Programmable objects, verbs, quota, and social order all sat with a handful of technical admins (the "wizards").
2. **1992, "LambdaMOO Takes a New Direction."** The wizards devolved *social* decisions to the player body — disputes, harassment, social norms — while keeping technical power (the database, the server, quota grants).
3. **1993, petition/ballot system.** Devolved decisions became formal: petitions, ballots, a voting mechanism with defined thresholds, plus a body of "archons"/arbitrators to hear disputes.
4. **The Mr. Bungle incident.** A participant used a scripted "voodoo doll" to make other avatars appear to perform acts they had not consented to — capability abuse through *delegated animation*, i.e. a script running with the apparent authority of another participant's avatar. The social machinery handled the aftermath; it could not undo the act, and the act was possible because the capability system let one participant's code drive another participant's presentation.
5. **1996, "LambdaMOO Takes Another Direction."** After the ballot system proved slow and unwieldy for anything time-sensitive, the wizards retook social authority. The devolution was reversed by the same people who had granted it.

Read as a design lesson rather than history, the arc says three things:

- **Governance on a programmable world is not one thing.** *Social* decisions and *capability* decisions were devolved together in 1992 and separated in practice afterwards. The failure was not that players could not govern; it was that the mechanism could not act at the speed of the harms, and that the capability layer had holes no vote could close.
- **Authority that can be retaken will be retaken, at the worst moment.** The 1996 reversal happened under pressure. A design that plans for admin authority from the start can make that authority *narrow, logged, and reviewable*; a design that discovers it under pressure makes it wide and unaccountable.
- **Quota was governance, not plumbing.** Wizards approved quota increases judged on the quality and utility of what a participant had already built. That is an admission control decision made by an admin, using the resource system as the lever, and it was one of the few levers that worked.

## 1. Authority reserved to the admin from day one

The test for "reserved" is: **can a world community's vote cause this to happen, if the admin disagrees?** If yes, it is delegable. If no, it is reserved and must be listed here explicitly, with the reason.

id already ships a version of this split, and the note's job is to make the split *principled and stated* rather than incidental:

| Authority | Reserved to | Why it cannot be delegated | What exists today |
| --- | --- | --- | --- |
| **Installing/replacing the world program** | world admin, or a `builder`-scoped participant the admin minted | The program *is* the world's behaviour; whoever installs it can change every rule at once, including the rules that would review the install. A vote to install is a vote to dissolve the constitution. | `WorldScopes::BUILDER` (install path only, carries no capability/network/file/process authority), `JournalEntry::BuilderInstalled` audit record, admin-only service operations |
| **Setting capability ceilings and grants** | world admin (ceiling also narrowed by the server admin) | A ceiling is the bound on what any grant can reach. If a world vote could raise its own ceiling, "ceiling" would mean "whatever the majority wants today" and every disclosure shown to a user would be a lie. | `WorldLimits.server_caps` (server ceiling, runtime config), `JournalEntry::CapsCeiling`, `WorldCommand::SetCapsCeiling`, admin-token-authorized |
| **The admin token itself** | server/world admin | The root of the authority chain. It is deliberately not a `WorldScope` and cannot be delegated, attenuated, or issued by a `delegator`. | `authorize_admin`, `MAX_ADMIN_TOKEN_BYTES`, `secret_eq` comparison |
| **Resource envelope (participants, fuel, bytes, tick floor)** | server admin (ceiling) + world admin (requested profile within it) | The envelope is what keeps *other* worlds on the same host alive. A world community that could raise its own envelope could starve its neighbours, and those neighbours never consented to that governance body. | `WorldLimits`, durable per-world resource profile, `WorldExecutionProfile::resolve(server_limits)` with the server ceiling winning |
| **Revoking a participant's capability tree** | admin, or a `moderator` the admin minted | Must be available inside one tick, at machine speed, to stop an ongoing harm. A ballot cannot be that. Note the *scope*: `moderator` can revoke, not install, not raise ceilings. | `WorldScopes::MODERATOR`, `WorldCommand::Revoke`, `JournalEntry::Revoked { by }`, `require_moderation` |
| **Host-level serving decisions (delisting, replication, serving policy)** | host operator | The host is a different principal with its own liability, and it may not even be a participant in the world. | `HostServingPolicy` in `boards.rs`, deliberate: delisting never claims global erasure |
| **Stopping the server / closing the world** | host operator | Same as above. | process-level |

Two rules fall out of the table and should be stated as invariants of the governance design:

1. **No governance body may widen its own authority.** Any decision that changes the ceiling, the grant set, the installed program, or the resource envelope requires the admin authority, even when a community process "proposed" it. A community proposal becomes effective only when an admin accepts it — which is exactly the shape the threat-list design already uses for feeds ("a feed is advice signed by its author, never authority over a world").
2. **Every exercise of reserved authority is logged as an entry that references what it changed.** The `BUILDER` and `MODERATOR` paths already write journal entries; the boundary-audit design already specifies an append-only signed decision log with per-scope visibility. Reserved authority should use that same log, so the 1996-style retaking of authority is *visible* when it happens rather than argued about afterwards.

### What the admin authority is *not*

- **Not a superuser capability inside world code.** The admin token authorizes service operations; it does not appear in the capability set a world program or a moderation library runs with. A moderation library that could present the admin token would collapse the table above.
- **Not a substitute for moderation.** Reserving install/ceiling/revoke does not make the admin a moderator of content. Content decisions are exactly what should be delegated (below), and the arc's lesson is about *mechanism speed and capability holes*, not about admins being better judges of content.
- **Not unaccountable.** Reserved means "cannot be outvoted", not "cannot be reviewed". The boundary-audit design's appeal flow applies: an adverse admin decision explains itself and can be overturned by a later signed entry, without rewriting history.

## 2. What is genuinely delegable to a world community

The complement of the table, and the part LambdaMOO got right for four years:

- **Content rules and their parameters.** What counts as spam, what is off-topic, what a report means, what thresholds trigger automatic quarantine, which reviewer set hears which class of report. These are the S75/S76 items and they are *decisions about content*, not about the machine.
- **Who moderates.** Elected/appointed reviewer sets, per-world. Rotation, recall, and term rules for those roles.
- **Appeals of content decisions.** A community body can overturn another moderator's content decision; the boundary-audit record shape supports that as new signed entries referencing the original.
- **Approval of participant submissions within an already-installed program** — a world's own in-world objects, designs, or library attachments, where approval binds a content hash and the capability footprint the artifact runs with, not the world program itself. This is the "pre-approve specific designs" half of S75 and it is safely delegable *provided* the approved artifact cannot install or re-ceiling the world.
- **Adopting, forking, and parameterizing moderation packs** — the library model of S76: same pack, different moderator set, different report threshold, different quarantine policy. Adoption and configuration are the community's; the pack's capability footprint is disclosed at adoption and bounded by the ceiling.

The delegable side has one hard condition: **a delegated decision must be able to act at the speed of the harm it addresses, or it must be backed by a reserved path that can.** LambdaMOO's ballot could not quarantine; id's delegated moderation can, because quarantine is a content action on an artifact, while revocation of the *participant* stays reserved. Keep those two separate and the 1996 failure does not repeat: the fast path exists, is narrow, and is logged.

### The Mr. Bungle lesson, mapped to id's surface

The incident was capability abuse through delegated animation: participant A's code caused participant B's avatar to appear to do things. It is the exact shape of a hole where *representation* is delegated without *consent scoping*.

Where id could have the same hole, and the rule that closes it:

- **Per-user function mapping ([#5967](https://github.com/developing-today/code/issues/5967) design).** A world maps over one bound function per user. The mapping must never let one binding's intent be attributed to another binding's entity. The failure table already routes contract violations to a crash row plus a flag; this note adds the governance reading: *attribution is reserved*, i.e. only the principal bound to a binding may cause intents attributed to it, and a world program that synthesizes another participant's intent is a platform defect, not a world rule.
- **Delegation (`WorldScopes::DELEGATE`).** A delegator mints a *narrower* capability for someone else. Attenuation is one-way and cannot carry `uses`; the admin authority bits (`BUILDER`, `MODERATOR`) are deliberately excluded from `WorldScopes::ALL` and cannot be delegated. That is the right shape and should be stated as the rule: delegated authority is always a strict subset, never a re-attribution.
- **Presentation.** If a world can render participant B differently to participant C (filters, sprites, cape sparkles per S67), that is presentation, not authority — but a filter that makes B appear to be a different participant, or appear to have said something, is the voodoo doll in modern dress. The presentation layer needs the same consent scoping as capabilities: a world may change how B *looks*, never who B *is* or what B *did*.
- **Moderation libraries.** A moderation library gets capabilities like any other code. It must not be able to act *as* a participant. Its legitimate set is: read content in the scope it moderates, hide/quarantine a content object (a signed statement, per the boards log model), open a report, and request a review — not revoke, not install, not re-ceiling. Those stay reserved.

## 3. Quota as a governance tool

LambdaMOO's quota is the most directly importable mechanism in the arc, because id already has the substrate and is currently using it only as a limit.

What LambdaMOO did:

- Default quota was **object count** per participant; a byte-based alternative existed in-database. Both are *resource* quotas.
- Increases were **granted by wizards**, judged on the **quality and utility** of what the participant had already built with their existing quota. The base allocation was automatic; the increase was an admission-control decision.
- The consequence is the design point: quota was **the main lever by which a world decided who gets to build more world**, and it was exercised by an admin as a *judgment*, not a formula. Participants could earn it; they could not vote it to each other.

What id has today, and what the gap is:

- `WorldLimits.participants` bounds active capabilities per world; the per-world resource profile bounds execution; `WorldExecutionProfile::resolve` keeps the server ceiling authoritative. These are all **envelope** limits — they bound the world as a whole, resolved by admin configuration, applied on next install/restart.
- What is missing is a **per-principal building quota**: a bound on the resources one participant's content or functions may consume, with a *grant* path that is an admin (or delegated steward) decision.

The design recommendation, in three parts:

1. **Two tiers, like LambdaMOO.** An automatic base allocation every participant gets (so joining and trying things never needs a decision), and a discretionary increase that someone must grant. The automatic tier is what keeps the world from being a gatekept club; the discretionary tier is what keeps it from being unusable.
2. **Quota grants are content decisions in the log, not envelope changes.** A grant names the principal, the new bound, and the evidence (what they built, why it merits more). It is a signed entry referencing the prior bound, reviewable and appealable exactly like a boundary decision. It does not touch the ceiling: the sum of granted quotas must still fit the world envelope, and if it does not, that is an envelope decision for the admin (and the server ceiling above them).
3. **Quota is never a punishment mechanism.** LambdaMOO's arc does not support revoking quota as discipline — the fast path for harm is revoke/quarantine, the slow path is review. Using quota as a penalty makes the resource system adversarial and invites the Core-War-style optimisation against the meter that the prior-art research already warns about.

**Interaction with governance, stated plainly:** a community can decide *policy* about quota (what kinds of building count as utility, how large the base allocation is within the envelope, who the stewards are who hear grant requests), but it cannot decide *the envelope*, and it cannot grant itself more than the admin's total. That is the same shape as §1 rule 1 and it is what makes quota a governance tool rather than a lever a majority can pull to consume the host.

## 4. The reserved-authority list, stated as a checklist

For #5973's design doc to be complete, it should carry this list verbatim (it is the explicit list #6004's "done looks like" asks for):

**Reserved to the world admin (cannot be outvoted by any world-community process):**

1. Install or replace the world program (and therefore the rules the community runs under).
2. Set or lower the world capability ceiling; issue or withdraw grants outside what the ceiling allows.
3. Hold and rotate the admin token; mint or withdraw `builder` and `moderator` authority.
4. Revoke a participant's capability tree.
5. Request or reduce the world's resource envelope (subject to the server ceiling, which is the server admin's).
6. Adopt or reject a community proposal; nothing a community process produces is effective without an admin-accepted entry.

**Reserved to the server/host operator (above the world admin):**

7. The server capability ceiling (`--world-allowed-caps`) and runtime limits.
8. Serving policy: what this host serves, replicates, or delists.
9. Whether the world runs at all.

**Delegable to the world community (parameters, not mechanism):**

10. Content rules and their parameters — report thresholds, quarantine policies, exempt lists.
11. Moderator/reviewer sets, their terms, and recall rules.
12. Appeals and reversals of content decisions.
13. Pre-approval of in-world submissions (content hash + capability footprint), excluding anything that installs or re-ceilings.
14. Adoption, forking, and per-world parameterization of moderation libraries.
15. Quota policy within the envelope, and the stewardship of who hears grant requests.

**Never delegable to anyone, including the admin, because they are platform invariants:**

16. Attribution — a participant's entity cannot be made to appear to act.
17. Capability ceilings that the platform has already promised a user in disclosure (an admin may narrow, never silently widen beyond disclosure).
18. The append-only integrity of the decision log (no one edits or deletes an entry; corrections reference).

Items 16–18 are the ones LambdaMOO could not enforce in 1993 and had to fix by retaking authority in 1996. Stating them as platform invariants on day one is the whole point of this note: it is cheaper to write them down than to lose a community to rediscovering them.

## 5. What this note does not decide

- The moderation-library interface itself (hooks, signatures, capability footprint enforcement) is #5973's own deliverable; this note only supplies the authority split it must respect.
- Pre-approval binding details (hash vs capability set vs both, and revocation of an approval) belong to #5973 and the library-typing child of #5955.
- The concrete quota data model, and whether grants are journalled like `CapsCeiling` or logged like board moderation, is an implementation decision for the slice that builds it.
- Feed-based moderation (threat lists) is [#6006](https://github.com/developing-today/code/issues/6006) and is already designed; this note's only addition is that a feed subscription is a *delegable* decision whose authority to enforce comes from the world's own pack, never from the feed author.

## References

- LambdaCore "Getting Started" (quota, programmer/wizard creation, and the warning that a "social wizard" is likely to do accidental damage): <https://www.moo.mud.org/moo-faq/lcore-getting-started.html>
- LambdaMOO case summary and sources: <https://vectree.io/pdf/c/lambdamoo>
- Prior-art research, Cluster 3 (LambdaMOO) and Cluster 4 (moderation-as-library), plus follow-up item 23: `workspace/scratch/worlds-capabilities-brief-2026-11-04/prior-art-research.md`
- Repo surfaces cited: `pkgs/id/src/world/mod.rs` (`WorldScopes::BUILDER`/`MODERATOR`, `WorldLimits`), `pkgs/id/src/world_limits.rs` (`WorldExecutionProfile`), `pkgs/id/src/world/actor.rs` (`Revoke`, `BuilderInstalled`, `require_moderation`, admin audit records), `pkgs/id/src/world_session/mod.rs` (`authorize_admin`, `secret_eq`), `pkgs/id/src/boards.rs` (`ModerationAction`, `HostServingPolicy`), `doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings/`, `doc/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles/`, `doc/2026-10-17T08-00-00Z_feature_world_roles_directory/`.
