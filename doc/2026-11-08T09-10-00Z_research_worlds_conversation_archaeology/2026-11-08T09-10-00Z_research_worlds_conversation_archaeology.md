# Conversation archaeology: prior Drew discussions of Second Life and programmable worlds, tied into the worlds program

**Short answer:** the worlds program of the 2026-11-04 brief ([#5955](https://github.com/developing-today/code/issues/5955), statement S108) is not a new direction. Its founding dictation is Drew's own 2026-10-07 message in OpenCode session `ses_ef03b7d73ffewFUtEk0FCMTiz5`, which already named the context map, server-side capability "DLLs", derivation of capabilities from usage, and a native Roc tier — and the same session is where the Second Life / late-'90s-and-2000s virtual-world inspiration set enters. Two halves of that dictation were built (the compile-time capability ceiling against the `world_caps.rs` catalog, and derivation-from-usage in its runtime form: `CapLedger::decide_for` in `world_caps.rs` records observed usage and grants a known capability on first use under `grant_on_use`); what the current program designs is the remaining static half — source-derived requested/discovered-used sets — and the user-function DLLs. Four interpretation flags (C1–C4) that the earlier discussion left open were **decided by Drew on 2026-11-04 23:57 CST**; those rulings are recorded here in full, and one item — the C1 uploads regime — remains explicitly open.

This record formalizes material that until now existed only as issue comments. It adds no new archaeology: every recovered thread below is quoted or paraphrased from the comments cited, with their dates and session references preserved. Nothing flagged in tension is resolved here.

## Provenance of this record

This is the landed form of the deliverable specified in [#5990](https://github.com/developing-today/code/issues/5990) ("Doc landed and linked from the tracking issue"). The substance was recovered earlier and posted as comments; this record gathers it into the repository so it survives independently of the issue thread.

| Item | Where it lived | Status here |
|---|---|---|
| Founding-dictation tie-in, findings **F1/F2** | [#5955 comment 6650340031](https://github.com/developing-today/code/issues/5955#issuecomment-6650340031) | Reproduced in substance, with the same date and session reference |
| Findings **F3–F12** | Referenced by that comment as living in a scratch file (`workspace/scratch/worlds-capabilities-brief-2026-11-04/conversation-tie-ins.md`) | **Not in the GitHub record** — see "Known gap" below. Not reconstructed here |
| Drew rulings **C1–C4** | [#5955 comment 6650960763](https://github.com/developing-today/code/issues/5955#issuecomment-6650960763) | Reproduced in full, with per-child pointer comments verified |
| Per-child ruling comments | #5984, #5977, #5958, #5957, #5962, #5964, #5996, #5966, #6001 | Verified present (see "Child pointer comments") |

### Known gap: the F3–F12 bodies

The F1/F2 comment says the full tie-in record — "findings F1–F12, settled list, open list, contradictions C1–C4" — lives at `workspace/scratch/worlds-capabilities-brief-2026-11-04/conversation-tie-ins.md`. That path is a scratch location, not a repository path, and it is not present on the machine this record was written from. The bodies of F3–F12 therefore cannot be reproduced from the GitHub record today, and this record does not invent them. What is reproduced is what the GitHub record actually holds: F1/F2 in substance, the C1–C4 rulings in full, and the child pointer comments that encode each ruling.

Recovering F3–F12 is a separate, explicitly out-of-scope task for this record (it would be re-doing the archaeology, which #5990 forbids). If the scratch file still exists on a machine that has it, a follow-up should append the findings here rather than rewrite this record — dated records are appended to, not rewritten.

## The recovered threads

### F1 — the founding dictation (2026-10-07, session `ses_ef03b7d73ffewFUtEk0FCMTiz5`)

The 2026-11-04 brief is a restatement and extension of the ID program's founding dictation, which ran across 2026-10-06 → 10-07 in that session. In Drew's own words (2026-10-07):

> "ensure any number of worlds could work, native roc tier too, should allow on the fly compilation of files and a variety of capability which amount to server side dll that the program expects to be available and is only available if the server was told of the cap or derived capabilities by looking at usage. I expect it might be a model like extra classes/functions stored In context map or something so that the interface to compile against is semi stable…"

What got built from it: the compile-time ceiling against the `world_caps.rs` catalog, and runtime derivation-from-usage (`CapLedger::decide_for`, `grant_on_use`). What did **not** get built: the static, source-derived requested/discovered-used sets, and the user-function DLL half. Those are exactly what the current program designs.

### F2 — the inspiration set enters in the same session (2026-10-07)

The virtual-world reference set Drew remembered also enters in that same session, spelled "secondlife", alongside: There.com, ActiveWorlds, Habbo, Gaia, Goblins, E-lang, Cap'n Proto RPC, Veilid and Anoma. This is the answer to the brief's "We've had discussions about Second Life and a variety of other games with similar concepts, a lot in the early 2000s and late '90s" — the discussion is the founding dictation itself, and the named worlds are the vocabulary it was reasoned in.

## The rulings: C1–C4 (Drew, 2026-11-04 23:57 CST)

The working interpretations previously recorded across the program are superseded where they differ from these rulings. Each ruling is recorded as decided; none is re-litigated here.

### C1 — licensing: adoption RESOLVED, uploads OPEN

- **Adoption regime (our own code's dependencies) — CLOSED, firm.** "for our code, we won't be using GPL." The 2026-10-24 hard filter — NOT AGPL, NOT GPL — stands for anything our code depends on. Not being revisited.
- **Uploads regime (user content) — OPEN, undecided.** Drew does not particularly want to support GPL, sees use for it, and is not ruling it out. There is neither a ban nor a support commitment. The earlier "warn-and-allow" framing remains **provisional only** — a framing for design discussion, not ratified policy.
- **Owner of the open half:** [#5984](https://github.com/developing-today/code/issues/5984). The classifier work ([#5977](https://github.com/developing-today/code/issues/5977)) must keep the uploads outcome configurable — not hard-coded as either a ban or as settled warn-and-allow — until #5984 resolves it.

### C2 — capability posture: RESOLVED, correcting the earlier interpretation

The earlier working interpretation — that the brief *supersedes* the shipped no-filesystem / no-network default — **is wrong**.

- **The default does not move.** The current completely isolated seccomp policy (no fs open, no sockets) remains the default for worlds and servers, and is a standing invariant for server design.
- **Additive new work:** an **opt-in** file + network capability tier, **off by default on servers**.
- **Publishing:** first-party only for now.
- **Architecture requirement:** built so it *could* be enabled for anyone later; general enablement is a future decision gated on testing and refining how the platform wrangles and controls such capabilities. Invariant/bounds design must cover containment for that tier when enabled, without weakening the default isolation.

### C3 — user-code handling: AFFIRMED as a platform pillar

User-code handling (server-compiled DLLs, hot-swap into running worlds without a world recompile) and fuel budgets (C4) are, in Drew's words, "very big parts of making this a platform that other people can build on top of, interact with, and reason about." Top importance, not side features.

**Iteration doctrine (applies to C3 and C4 in full):**

- Thorough research first; test what is found; actively look for holes and gaps.
- Expect to rework and rebuild multiple times before landing the right solution — iteration is the plan, not a failure mode.
- Initial implementations may be locked to us in a test environment and need not be user-friendly in any way — **but** everything built must retain at least the capability to become highly user-friendly later. No architectural dead ends on UX.

C3 substance stands as interpreted: user code is source-only and server-compiled, which is what reconciles hot-swap with the Oct 31 native trust boundary (see [#5996](https://github.com/developing-today/code/issues/5996) for the alternatives exploration).

### C4 — fuel budgets: AFFIRMED as a platform pillar

Per-function metering / fuel budgets — with user-code handling (C3) — are "very big parts of making this a platform that other people can build on top of, interact with, and reason about."

- Cross-host fairness needs **fuel-style budgets, not wall-clock timeouts**; wall-clock is acceptable only single-host/cooperative, per the original interpretation.
- The two-level metering adaptation (burst bucket + hard per-tick ceiling, per-function accounting) is [#6001](https://github.com/developing-today/code/issues/6001), with the metering research on [#5966](https://github.com/developing-today/code/issues/5966).
- The iteration doctrine of C3 applies here in full.

**The one explicitly open item after these rulings: the C1 uploads regime, owned by #5984.**

## Classification against the #5955 design children

Each recovered thread is flagged against the design children it touches, in the three categories the deliverable asks for: *consistent*, *superseded by the 2026-11-04 brief*, or *in tension — needs Drew*.

| Recovered thread | Design child | Classification | Basis |
|---|---|---|---|
| F1 founding dictation: capability ceiling + runtime derivation built; static discovered-used set + user DLLs not | [#5958](https://github.com/developing-today/code/issues/5958) capability model (requested vs discovered-used sets) | **Consistent** | The static (source-derived) discovered-used set is the part of F1 that never got built — the runtime form (grant-on-use) exists; #5958 designs the static set |
| F1 founding dictation: server-side capability "DLLs" | [#5964](https://github.com/developing-today/code/issues/5964) dynamic user functions, server-compiled DLLs | **Consistent** | F1 names "server side dll" as the model; #5964 is its design |
| F1 founding dictation: on-the-fly compilation, semi-stable compile interface | [#5956](https://github.com/developing-today/code/issues/5956) Roc library imports; [#5996](https://github.com/developing-today/code/issues/5996) hot-swap alternatives | **Consistent** | "on the fly compilation" + "interface to compile against is semi stable" is the same problem both children address |
| F1 founding dictation: "native roc tier" | Roc worlds architecture record | **Consistent** | `doc/2026-10-07T00-20-00Z_architecture_roc_worlds/` is the record of that same session's design |
| F2 inspiration set (Second Life, There.com, ActiveWorlds, Habbo, Gaia, Goblins, E-lang, Cap'n Proto RPC, Veilid, Anoma) | Whole program | **Consistent** | The vocabulary the program is reasoned in; no child contradicts it |
| C1 uploads regime, earlier warn-and-allow framing | [#5984](https://github.com/developing-today/code/issues/5984) licensing policy; [#5977](https://github.com/developing-today/code/issues/5977) license taxonomy | **Superseded by the 2026-11-04 ruling** (warn-and-allow is provisional only, not policy) | Drew ruling 2026-11-04 23:57 CST |
| C2 earlier interpretation that the brief supersedes the shipped default | [#5958](https://github.com/developing-today/code/issues/5958), [#5962](https://github.com/developing-today/code/issues/5962), [#5957](https://github.com/developing-today/code/issues/5957) | **Superseded by the 2026-11-04 ruling** (default unchanged; opt-in tier is additive) | Drew ruling 2026-11-04 23:57 CST |
| C3 user-code handling as a top-importance pillar | [#5964](https://github.com/developing-today/code/issues/5964), [#5996](https://github.com/developing-today/code/issues/5996) | **Consistent** (affirmed) | Drew ruling 2026-11-04 23:57 CST |
| C4 fuel budgets as a top-importance pillar | [#5966](https://github.com/developing-today/code/issues/5966), [#6001](https://github.com/developing-today/code/issues/6001) | **Consistent** (affirmed) | Drew ruling 2026-11-04 23:57 CST |
| C1 uploads regime outcome | [#5984](https://github.com/developing-today/code/issues/5984) | **In tension — needs Drew** (explicitly left open by the ruling) | Drew ruling 2026-11-04 23:57 CST |

**Nothing above is resolved by this record.** The "in tension — needs Drew" row is the ruling's own open item, recorded as open.

## Child pointer comments

The deliverable asks that every design child with a flagged tension carry a comment pointing at the relevant recovered passage. Spot-checked 2026-11-08: all nine children named by the C1–C4 ruling comment carry their pointer, and none is missing.

| Child | Ruling | Pointer comment |
|---|---|---|
| [#5984](https://github.com/developing-today/code/issues/5984) licensing policy | C1 (primary record for uploads) | [6650953359](https://github.com/developing-today/code/issues/5984#issuecomment-6650953359) |
| [#5977](https://github.com/developing-today/code/issues/5977) license taxonomy & classifier | C1 classifier implications | [6650953863](https://github.com/developing-today/code/issues/5977#issuecomment-6650953863) |
| [#5958](https://github.com/developing-today/code/issues/5958) capability model | C2 (primary record) | [6650954322](https://github.com/developing-today/code/issues/5958#issuecomment-6650954322) |
| [#5957](https://github.com/developing-today/code/issues/5957) demo library + app | C2 demo implications | [6650954848](https://github.com/developing-today/code/issues/5957#issuecomment-6650954848) |
| [#5962](https://github.com/developing-today/code/issues/5962) server-side invariants | C2 invariant implications | [6650956299](https://github.com/developing-today/code/issues/5962#issuecomment-6650956299) |
| [#5964](https://github.com/developing-today/code/issues/5964) dynamic user functions | C3 (primary record) | [6650956743](https://github.com/developing-today/code/issues/5964#issuecomment-6650956743) |
| [#5996](https://github.com/developing-today/code/issues/5996) hot-swap alternatives | C3/C4 doctrine | [6650957236](https://github.com/developing-today/code/issues/5996#issuecomment-6650957236) |
| [#5966](https://github.com/developing-today/code/issues/5966) per-function gas metering | C4 (primary record) | [6650957831](https://github.com/developing-today/code/issues/5966#issuecomment-6650957831) |
| [#6001](https://github.com/developing-today/code/issues/6001) two-level metering | C4 pillar affirmation | [6650958335](https://github.com/developing-today/code/issues/6001#issuecomment-6650958335) |

## Sources

- Issue [#5990](https://github.com/developing-today/code/issues/5990), statement S108 of the 2026-11-04 brief; deliverable and "done looks like" text.
- Program issue [#5955](https://github.com/developing-today/code/issues/5955), including [comment 6650340031](https://github.com/developing-today/code/issues/5955#issuecomment-6650340031) (F1/F2 tie-in) and [comment 6650960763](https://github.com/developing-today/code/issues/5955#issuecomment-6650960763) (C1–C4 rulings).
- Child pointer comments listed in "Child pointer comments" above.
- `doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md` — the Roc worlds architecture from the same founding session.
- `doc/2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting/2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting.md` — the upload-regime license analysis that the C1 open item feeds.

## Change log

- 2026-11-08: Initial landed record for issue #5990; formalizes existing #5955 tie-in and ruling comments without redoing the archaeology. F3–F12 bodies recorded as a known gap (they live only in a scratch file outside the repository).
