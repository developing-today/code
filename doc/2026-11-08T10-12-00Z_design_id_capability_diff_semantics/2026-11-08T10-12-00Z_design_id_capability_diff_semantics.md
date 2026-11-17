# Capability-change diff semantics — user-meaningful classes and re-approval triggers

- **Date:** 2026-11-08
- **Status:** proposed design for review; no implementation is included here.
- **Issue:** [#5999](https://github.com/developing-today/code/issues/5999), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955); parent topic [#5959](https://github.com/developing-today/code/issues/5959) owns the change-notice UX and consumes this record.
- **Inputs:** re-triage of #5999 by task 12425 (2026-11-08, verdict confirmed real-and-current); prior triage task 10365 (2026-11-05); the registry vocabulary in `pkgs/id/crates/id-core/src/world_caps.rs` at `origin/main` `cf39c3aba1` (re-read at `b6bd188624`, unchanged); sibling designs for the capability model ([#5958](https://github.com/developing-today/code/issues/5958)), the used-set derivation tool ([#5998](https://github.com/developing-today/code/issues/5998)), and the observability surface ([#6017](https://github.com/developing-today/code/issues/6017)).
- **Method:** every "current state" claim is tied to code at this checkout. Chrome's permission-warning model is the stated precedent in #5999 and is followed in substance, not copied field-for-field. Where this record spends a concept another child of #5955 owns (the declared-vs-derived split, the derivation tool, the notice UI, registry format), it names the seam and does not redefine that child's part.

## Decision to make

#5959 must warn a returning user when a world they already connected to now does something broader than what they accepted. That requires a *comparison*, and the comparison is the whole design: compare too coarsely and every update prompts (fatigue, the failure the notice exists to avoid); compare too finely and real escalations slip through as "just one more permission" (a silent upgrade, which Drew's requirement forbids outright).

This record settles, for #5959 to consume directly:

1. **What is compared** — which sets on each side of the diff.
2. **What the comparison is over** — user-meaningful capability *classes* (the "warning messages" of the registry), not raw capability names.
3. **The collapse/subsumption rules** — which additions are already covered by an accepted warning.
4. **The diff-trigger rules** — change kinds that require re-approval before reconnect, and change kinds that are silent.
5. **The sideways case** — a discovered-used set that changes while the requested set does not.

It does **not** design the notice's presentation, wording, placement, or the user's choice buttons; those are #5959's.

## Recommendation

Compare the **class set** the world's registry entry produces, computed from `declared = requested ∪ platform-bound`, against the **accepted class set** the user last explicitly approved for that world. Trigger re-approval **iff** a class in the new set is not covered by the accepted set under a subsumption lattice. Everything else — removals, moves within one class, messageless capabilities, optional additions, used-set-only changes — is silent (the last one is surfaced as an author-facing declaration defect, not a permission moment).

The class set is an **antichain of maximal classes under an explicit subsumption order**, so "expansion" has exactly one meaning: a class appears that no accepted class covers. That single definition is what makes the trigger implementable and testable without judgment calls.

## Current state (evidence)

| Existing fact | Design consequence |
| --- | --- |
| The registry is 19 statically registered providers with a `ProviderManifest` carrying `name`, `version`, `kind`, `argument_schema`, `bounds`, `required_scope`, `default_ceiling` (`pkgs/id/crates/id-core/src/world_caps.rs:202-216`, table at `:346-491`). | There is **no class field today**. The class assignment is a new manifest field (§2.4), so the table works for names not yet registered only through the `unknown` fallback, never by guessing. |
| `ProviderKind` is `Request`, `Subscription`, `PlatformModule` (`world_caps.rs:129-136`), and platform modules are checked by the compile gate (`check_imports`) rather than called as host requests (`world_caps.rs:196-200`; `pkgs/id/examples/roc-world/README.md`). | Platform modules are still ledger-visible names a world can be bound to — `Screen` and `Key` decide whether a world can render a UI and receive keyboard input — so they get real classes; the data-type modules are messageless (§2.2). |
| `RequiredScope` is `Join`, `Chat`, `ChatRead` (`world_caps.rs:190-199`), used by the compile gate and the ledger's actor check. | Actor scope is about *which participant* may cause a use; it is **not** the warning vocabulary and does not determine class. Two names can share a scope and differ in class. |
| The effective authority is `granted ∩ server_ceiling ∩ world_ceiling ∩ CATALOG`, with `*` expanded over allowed names except sensor caps (`world_caps.rs:1358-1380`; ceiling formula in `doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings`). | The diff must use the same expansion the ledger uses. `*` is not a special "everything" class; it is evaluated into classes like any other set. |
| Sensor caps are `loc.*` (`is_sensor_cap`, `world_caps.rs:1040-1042`): a wildcard never reaches them and grant-on-use never mints a durable grant; each use passes a consent seam. | Sensor caps already have the strongest per-use discipline. Their class is the most sensitive one in the table, and a `*` grant adding one is a genuine expansion (it adds the class). |
| `CapsReport` already reports `granted`, `requested`, `missing`, `usage`, `policy`, `catalog`, `ceiling`, `effective` (`world_caps.rs:1426-1454`). | The **facts** a diff needs already ship on the wire. This design adds no new server state; only the class vocabulary and the comparison rule. |
| Grant-on-use: a `deny`/`grant-on-use` policy where the first use mints a durable grant (`world_caps.rs:1335-1342`, `CapLedger::decide_for` → `Decision::AllowAndGrant`). | Grant-on-use grows the granted set *during* a connection. That is not a reconnect diff; §4 rule 9 states the seam rather than pretending the notice covers it. |
| No capability-diff or notice code exists in `pkgs/id` at this checkout (grep for `reconnect` finds only the replication snapshot path, `pkgs/id/src/world_replication_net.rs:206`). | Everything here is new semantics over existing facts. No implementation is implied by this record. |
| The used-set derivation tool is a separate, unbuilt design ([#5998](https://github.com/developing-today/code/issues/5998)); the declared-vs-derived split is [#5958](https://github.com/developing-today/code/issues/5958)'s. | The used set is a *derived over-approximation* with known inexactness (first-class function passing, dynamic loading). Treating it as a trigger would make the notice fire on analysis-tool changes. §5. |

## 1. What is compared

Two sets on each side, one of them a trigger and one a display-only detail.

**Compared for the trigger — the declared class set.**

```
declared(world) = requested ∪ platform_bound
```

- **`requested`** is the set the world declares in its registry entry: what it says it needs (S11–S13 of the #5958 brief). This is the set a user can hold someone to.
- **`platform_bound`** is what the platform the world was compiled against can reach (S14–S15: a user may hand a world a platform with *more* capability than it uses; the compile-time ceiling is a real upper bound). Omitting it would let an update swap to an expansive platform without changing a single declared request — an expansion the user never sees. Including it is the only reading consistent with "no silent upgrades of what a known world can do".
- `requested ∪ platform_bound`, not `∩`: the union is the authority the world actually holds, because the platform bound is reachable whether or not the world declares it.

Both are facts of the registry entry bound to the module content hash. The entry must be the signed, content-addressed one, or the baseline is attackable: a server that can serve a different manifest for the same world version can defeat any comparison.

**Displayed but not triggering — the used set.**

`used` (the derivation tool's output, #5998) is shown beside the declared set to answer #5958's S16 question ("it asks for all these, but it only uses this subset"). A used set *narrower* than declared is the normal, reassuring case. A used set *wider* than declared is a declaration defect (§5). Neither changes whether the notice appears.

**Not compared at all.**

- `granted` / `effective` / `ceiling`: these are the *host's* decisions about this world. A host lowering a ceiling is a decrease; raising one grants nothing on its own (`effective = granted ∩ allowed`). Neither can add a class the declared set does not already produce.
- `usage`: runtime accounting, not authority.

## 2. Capability classes — the "warning messages"

### 2.1 The rule

A class is a **user-meaningful disclosure**: one sentence of consequence a person can act on. Each capability name maps to exactly one class. A class either has a message (it is *disclosing*) or is *messageless* (it discloses nothing a person would decide on).

The message set of a declared set is not "the classes of its members" — it is the **maximal classes of its members under the subsumption order** (§3), with messageless classes dropped. That maximalisation is what implements Chrome's collapse rule without special cases.

### 2.2 The table (current registry, all 19 names)

Classes are ordered from the least to the most consequential only for readability; the machine order is the subsumption lattice in §3.

| Class id | Message (client-side text) | Members today | Why this is the disclosure |
| --- | --- | --- | --- |
| `cap.none` | *(messageless)* | `time.now`, `time.tick`, `random.u64`, `world.info`, `RocJson`, `RocGrid`, `RocText`, `RocCap` | Reading a clock, a random number, or its own world's metadata is not something a person decides about. The four `Roc*` platform modules are data types with no host effect but are ledger-visible names (`ProviderKind::PlatformModule`), so they need a mapping; it is messageless. |
| `world.presence` | "See who is in this world, and when they arrive or leave." | `players`, `players.list` | Other participants' presence is other people's information. |
| `chat.read` | "Read this world's chat history." | `chat.history` | Other participants' content. |
| `chat.write` | "Post messages to this world's chat as this world." | `chat.say` | Outward-facing: other people see it. |
| `world.records` | "Read the records this world stores." | `records.read` | World state, which may include other participants' contributions. |
| `world.schedule` | "Schedule actions that run later, without you present." | `schedule.at`, `schedule.cancel` | Time-shifted authority: an effect the user is not watching. |
| `device.screen` | "Draw a full-screen UI in your client." | `Screen` | Display control: the import is compile-gated (`check_imports`), and `Screen.render_json` is what produces a validated portable screen document the host renders. |
| `device.keyboard` | "Interpret your keyboard input." | `Key` | The `Key` module is compile-gated and is the parser for keyboard/terminal input (`Key.parse_events`, journaled like other world input). World input bytes reach `update` for every world, so what this class marks is a world that actively *interprets* keystrokes as keys, not one that merely receives text. Kept separate from `device.screen` because input capture and display are different consequences. |
| `device.location` | "Use this device's location." | `loc.position`, `loc.watch` | Device sensor; the registry already gives these the strongest discipline (explicit ceiling entry **and** explicit grant, plus a per-use consent seam). The class matches that weight. |

### 2.3 Reserved classes for the vocabulary #5958 must express

#5958 S12 requires the vocabulary to at least express standard, sandboxed file read+write, arbitrary file read, and arbitrary network out. None of those names is registered yet (`CATALOG_ENTRIES` at `world_caps.rs:346-490` holds only the 19 above). The class table must already have their slots, or the first provider to land one would ship with no warning:

| Class id | Message | Reserved for | Subsumption |
| --- | --- | --- | --- |
| `fs.sandbox` | "Read and write files in this world's own storage." | sandboxed file read+write (S12) | subsumed by `fs.read.arbitrary` |
| `fs.read.arbitrary` | "Read any file this server can read." | arbitrary file read (S12) | subsumes `fs.sandbox` |
| `net.out` | "Connect to any host on the network." | arbitrary network out (S12) | subsumes any future per-host `net.host.<name>` class |

The reserved classes are the load-bearing example for the collapse rule: adding one more *sandboxed* path inside `fs.sandbox` is silent once that class is accepted; adding arbitrary read is not, because it is a different, broader class that `fs.sandbox` does **not** subsume.

### 2.4 Where the class lives

**Class assignment is registry data, not string parsing.** `ProviderManifest` gains a `class` field (a class id). Prefix guessing is explicitly rejected: `loc.*` happening to be sensor caps today (`is_sensor_cap`, `world_caps.rs:1040`) is a different mechanism with different consequences, and coupling the two would mean a future `loc.calibrate` either silently joins the sensor discipline or silently leaves the warning table.

**Message *text* is client-side, keyed by class id.** A server that supplied the message text could choose the wording of the warning — the one thing the warning exists to prevent. The registry entry supplies the class id (signed with the entry, so it cannot be swapped per viewer); the client supplies the sentence.

**Unknown class id → the `unknown` class, which always discloses.** A class id a client does not know (a newer registry than the client) maps to a single class `unknown` with the message "Do something this version of the app cannot describe." It is disclosing and it is not subsumed by anything, so it triggers whenever it is new. Fail-closed: a client that cannot explain a capability must warn about it, not stay silent.

**Unknown capability name → the `unknown` class too.** The ledger already aggregates unknown names into one `<unknown>` usage bucket for DoS reasons (`UNKNOWN_CAP_USAGE_KEY`, `world_caps.rs:107`); the class table's `unknown` mirrors that for the same reason — a program cannot mint unlimited distinct classes by inventing names.

**Table versioning.** The class table is versioned with the client build. A diff records the table version it was computed under; if the version changed between the accepted baseline and the new comparison, the comparison is recomputed under the newer table and any class the newer table newly *splits out* behaves as new. (A table change that merges classes can only shrink the message set, which is silent by rule 2 of §4.)

## 3. Collapse and subsumption rules

### 3.1 The order

Subsumption is a partial order `≼` on class ids, read "is no broader than":

```
c ≼ d   means   accepting d means accepting c
```

Declared facts, transitively:

- `cap.none ≼ c` for every class `c` — a messageless capability is implied by anything, so accepting anything accepts it. (In practice `cap.none` is dropped from message sets before the comparison; the relation is stated for completeness of the lattice.)
- `fs.sandbox ≼ fs.read.arbitrary`
- `net.host.<name> ≼ net.out` for every reserved per-host class
- `c ≼ c` for every class

Nothing else is ordered. In particular `device.keyboard` and `device.screen` are incomparable; `chat.read` and `chat.write` are incomparable (reading is not implied by writing, and writing is not implied by reading); `world.presence` and `chat.read` are incomparable. **Collapse is never inferred from a shared prefix, a shared `RequiredScope`, or a shared `ProviderKind`.**

### 3.2 The collapse rules

1. **Within one class, membership is invisible.** `loc.position` and `loc.watch` are both `device.location`; granting one and adding the other adds no message and never triggers. The same holds for `players`/`players.list`, `schedule.at`/`schedule.cancel`, and the four `Roc*` modules.
2. **A broader accepted class covers a narrower new one.** If `fs.read.arbitrary` is accepted, adding a `fs.sandbox` capability is silent. The reverse never holds: adding arbitrary read to an accepted sandbox class triggers.
3. **Messageless capabilities never enter the comparison.** Adding a fifth `Roc*` data type, a new clock provider, or any `cap.none` member is always silent. This is the rule that keeps infrastructure growth from generating notices.
4. **Grandfather by class, not by name.** A baseline accepts *classes*; it is not a pile of accepted names. This is why an update that swaps `loc.position` for `loc.watch` without changing the class set is silent even though the raw requested set changed (Chrome's "a permission already collapsed under a broader granted warning does not trigger a disable").
5. **The collapsed set is the message set.** `messages(S) = maximal( { class(n) : n ∈ S } \ { cap.none, unknown-if-empty } )` under `≼`. Two declared sets with the same message set are indistinguishable to the notice — by construction, not by an extra equality rule.

### 3.3 `*` in a declared set

The registry allows `*` in a *grant* set with the existing expansion rules (`world_caps.rs:1358-1380`: every allowed name except sensor caps, which need an explicit grant). A *declared* set is not a grant set, so `*` there is a defect of the registry entry, not a shortcut: it is reported to the author and, for the comparison, expanded over `CATALOG` exactly as the ledger would, so a `*` declared set produces the union of the declaring classes of the catalog plus every reserved-class situation it would reach. Nothing about `*` turns off the trigger.

## 4. Diff-trigger rules

Let `B` be the **accepted baseline** — the message set the user last explicitly approved for this world — and `N` the message set computed from the world's current registry entry (§1, §2). Both are sets of class ids under one table version (§2.4).

| # | Situation | Set relation | Effect |
| --- | --- | --- | --- |
| 1 | **New warning message produced** — a class in `N` is not covered by `B` | `∃ c ∈ N : ¬(∃ b ∈ B, c ≼ b)` | **Trigger.** Re-approval required before the world connects. Every uncovered class is named in plain language. |
| 2 | **Removal or narrowing** — everything new is nothing | `N` covers no class outside `B`… i.e. `∀ c ∈ N, ∃ b ∈ B : c ≼ b` | **Silent.** No prompt, this connection proceeds. The baseline may then be narrowed to `N` (see 5.4). |
| 3 | **No change** | `N = B` | **Silent.** |
| 4 | **Sideways within accepted classes** — raw sets differ, class set does not | §3 | **Silent.** The raw diff is still recorded for the observability surface ([#6017](https://github.com/developing-today/code/issues/6017)); it is not shown as a warning. |
| 5 | **Messageless additions** | §3 rule 3 | **Silent.** |
| 6 | **Optional-capability additions** — declared `optional`, granted only if the user chooses later | no class in `N` | **Silent.** Optional capabilities are not authority the world holds on connect. When one is granted, the grant is a user action, not a diff. |
| 7 | **Used-set only** — `used` changes, declared set does not | `N = B` | **Silent as a notice.** If `used` now exceeds declared, that is a declaration defect: reported to the author and on the observability surface (§5). |
| 8 | **Host ceiling change** | §1 "not compared" | **Silent.** A lowering is a decrease; a raising grants nothing alone. |
| 9 | **Grant-on-use mints a grant mid-connection** | not a reconnect diff | **Out of scope here — named seam.** `Decision::AllowAndGrant` (`world_caps.rs:1335-1342`) grows `granted` while connected. This record does not stop it and #5959's notice does not cover it. Where grant-on-use is enabled, the world's policy (already reported as `policy: "grant-on-use"` in `CapsReport`) is what a user relies on, and the notice should say so rather than imply a guarantee it does not provide. |

**Rule 1 is the whole trigger.** Rules 2–9 exist to say precisely what is excluded, because each of them is a plausible way to accidentally produce a notice and burn the user's attention.

### 4.1 The user's choices (semantics only; #5959 owns the UI)

- **Proceed once** — connect holding `N`; `B` is unchanged, so the same change prompts again next time.
- **Proceed and accept** — connect holding `N`; `B := N`. This is the only path that changes the baseline, and it is always an explicit act.
- **Refuse** — do not connect. `B` unchanged.
- **Never, ever implicit accept.** A change approved by a click on "connect" without a decision is exactly the silent upgrade the requirement forbids.

### 4.2 Where the baseline lives

The accepted baseline is a **client-side** record, keyed by world id and the identity of what was accepted:

```
baseline_key = H(world_id ‖ module_content_hash ‖ declared_message_set ‖ table_version)
```

- Client-side, not server-side: a server that stores the baseline can also rewrite it, and the notice would then be something the server decides to show. The notice is a promise to the *user*.
- The module content hash is in the key because the comparison is about a *version* of a world. A new version with the same message set is silent (rule 2/3); the key changes, the message set does not, so a fresh record is written at the same `B`.
- Multiple devices: the baseline travels with the user's client state (the same place the accepted-invite and first-connect decisions already live), not with the world. Two devices disagreeing is acceptable and safe: the strictest one prompts.

## 5. The sideways case (#5959's open question, answered)

#5959 asks: *"A change in used with no change in requested is exactly the sideways case that matters."*

It matters, but it is **not a permission moment**:

1. **The used set is not authority; it is a claim about the code.** It is produced by static call-graph analysis with known inexactness (#5998: first-class function passing forces over-approximation; dynamic loading breaks exactness outright; #6000 owns the exception policy). A used set that grows can mean the code changed, the *analyzer* changed, or a library the world depends on changed. Prompting on it makes the notice fire on analysis-version churn — the fastest known way to teach users to click through warnings.
2. **A used set wider than the declared set is a defect of the world, and the author owes a declaration change.** When the derivation tool reports used ⊄ declared, the correct response is to require the author to widen the declaration; the widening then reaches the user through rule 1 of §4, with a real class and real wording. Routing it through the notice as a used-diff would hide *which* capability is newly reachable.
3. **A used set that moves sideways inside the declared set** (`loc.position` used instead of `loc.watch`) changes no class and no authority. That is rule 4: silent, recorded, inspectable.
4. **What the user sees** for the used set is the #5958 S16 view: "it asks for all these, but it only uses this subset." That is a *trust* surface — shown at first connect and in the observability view (#6017) — not an interruption.

So the answer to #5959's question is: **compare the declared set; show the used set.** A used-set change under an unchanged declared set never warns, and if the used set outgrew the declaration, the cure is a declaration change, not a notice.

## 6. What #5959 consumes — the contract

An implementer of the notice can build it directly from this section.

**Inputs**

- `registry_entry` (signed, content-addressed): `requested`, `platform_bound`, per-name `class`, per-name `optional` flag.
- `accepted_baseline`: `B`, the message set, plus the `baseline_key` and `table_version` it was recorded under.
- `client_class_table`: class ids → message text; the subsumption relation `≼`; the `unknown` fallback.

**Functions**

```
classes(S)   = { table(n).class : n ∈ S }            // unknown names → `unknown`
messages(S)  = maximal( classes(S) \ {cap.none} )    // maximal under ≼
N            = messages(requested ∪ platform_bound)
trigger      = ∃ c ∈ N : ¬(∃ b ∈ B : c ≼ b)
new_classes  = { c ∈ N : ¬(∃ b ∈ B : c ≼ b) }        // exactly what the notice names
```

**Givens**

- `trigger` is a pure function of `(B, N, ≼, table_version)`. No clocks, no history, no heuristics.
- `new_classes` is what the notice renders, in plain language, one line per class, using the client's text.
- The `declared` raw diff, the `used` set, and the over‑declaration flag ride along as detail for the expandable part and for #6017 — never as the trigger.

**Explicitly settled for #5959, so it does not re-decide them:**

- *What is compared* → `requested ∪ platform_bound`, mapped to classes (§1). Not the used set, not the granted/effective set.
- *The sideways case* → silent; surfaced as an over-declaration defect (§5).
- *Note on grant-on-use* → the notice is not the guarantee there (§4 rule 9); #5959 should say so where it describes what the user is promised.

**Left to #5959:** presentation, wording delivery, the choice controls, where the notice appears in the connect flow, and how it interacts with invites ([#5751](https://github.com/developing-today/code/issues/5751)) — including whether accepting an invite pins a capability baseline. The comparison this record defines is what an invite would have to pin.

## 7. Invariants implementation must hold

1. **Class-set determined.** The trigger depends only on `(B, N)`. Any two registry entries with the same message set produce the same verdict.
2. **Monotone in acceptance.** If `B` covers `N` (`∀ c ∈ N, ∃ b ∈ B : c ≼ b`), there is no trigger. Accepting can only ever silence.
3. **Never trigger on shrink.** `N ⊂ B` ⇒ silent. Removing authority is never an interruption.
4. **Fail-closed.** An unknown capability name or unknown class id maps to `unknown`, which nothing covers, so it triggers when new. Silence requires understanding.
5. **Set semantics.** Order, duplicates and spelling of the raw lists are irrelevant; the inputs are sets.
6. **Server cannot word the warning.** Message text comes from the client table; the entry supplies class ids only.
7. **No inference from prefixes or scopes.** Class membership is a declared field; `RequiredScope` and name prefixes never determine class.
8. **Idempotent.** Comparing a state against itself is silent; accepting and re-comparing is silent.
9. **Table-change safety.** Recomputing an old baseline under a newer table may only *shrink* the message set silently; a newly split-out class is new and triggers.
10. **Err toward warning.** Whenever the comparison cannot be computed (missing entry field, unparseable class, version skew), the result is a trigger with `unknown`, not silence.

These are stated as invariants so they can be property-tested: the state space is `(B, N, ≼)` over a small class vocabulary, and invariants 1–5, 8 and 9 are decidable over it without a host.

## 8. Alternatives rejected

- **Compare raw permission lists.** Chrome's model exists precisely because this fails: it fires on messageless and already-collapsed additions and trains users to dismiss. Rejected.
- **Compare a manifest hash.** Answers "did anything change", not "did anything get broader", and cannot name the change in plain language — which #5959 requires explicitly ("not a hash change"). Rejected.
- **Treat the discovered-used set as a first-class side of the diff.** Path-dependent, over-approximated, and analyzer-version-dependent (§5). Rejected as a trigger; retained as a display.
- **Compare `effective` (granted ∩ ceilings).** That is the host's current decision, and a host lowering a ceiling would produce a "change" that is in fact a decrease, while a host raising one grants nothing. It also puts the comparison where a host can influence it. Rejected.
- **Prompt on any change at all.** The fatigue failure mode, and it makes the notice meaningless. Rejected.
- **Never prompt; show a badge after connect.** Directly contradicts "no silent upgrades of what a known world can do" and "before the user connects". Rejected.
- **Server-supplied warning text.** Lets the serving party choose the warning. Rejected; class ids travel, text is local.
- **Store the baseline server-side.** Same problem one level up: whoever stores the baseline can rewrite it. Rejected; baseline is client state.
- **A single global "sensitive" flag instead of a class table.** Cannot say *what* got broader, cannot collapse, cannot express per-class acceptance. Rejected.

## 9. Open questions

1. **Invite pinning** ([#5751](https://github.com/developing-today/code/issues/5751)). Should accepting an invite pin the baseline the invite was issued under, so the world's later expansion prompts even on first join through the invite? This record's `baseline_key` is the thing such a pin would record; the policy is #5959/#5751's.
2. **Multi-user worlds.** Who is "the user" for a world whose grants an admin sets? The declared set is per world version, not per user, so the baseline is per user; a per-server admin surface for "what will my users be asked" is #6017's audience×surface question.
3. **Reserved-class registration.** When the first `fs.*` or `net.*` provider lands, its `class` field must be one of §2.3's ids or a new id added to the table *in the same change*, or it lands as `unknown` and every world using it warns. A registration-time check ("a class id must exist in the shipped table") is the enforcement point; this record does not place it.
4. **Wording review.** The §2.2 sentences are the semantics, not approved copy. They need a pass with the permission-UX child of #5955 ([#5960](https://github.com/developing-today/code/issues/5960)).
5. **Baseline versioning by platform bound.** `platform_bound` is part of `declared`. If two servers run different platform trees for the same world, the message set legitimately differs. Whether a user's baseline should be per server is open; the safe default (strictest prompts) is what §4.2 states.

## 10. Follow-ups

- File the `class` field addition on `ProviderManifest` and the reserved-class table as an implementation issue against `pkgs/id` — this record is a design deliverable and deliberately lands no code.
- Link this record from #5959 so its notice design consumes §6 rather than re-deriving the comparison.
- The registration-time "class id must exist" check (§9.3) belongs with whichever change first registers an `fs.*`/`net.*` provider.

## References

- [#5999](https://github.com/developing-today/code/issues/5999) — this design deliverable.
- [#5959](https://github.com/developing-today/code/issues/5959) — capability-change notices on reconnect; the consumer of §6.
- [#5955](https://github.com/developing-today/code/issues/5955) — the worlds design program.
- [#5958](https://github.com/developing-today/code/issues/5958) — capability model: requested vs discovered-used sets (S8–S16).
- [#5998](https://github.com/developing-today/code/issues/5998) — the used-set derivation tool whose output §5 treats as a display-only fact.
- [#6017](https://github.com/developing-today/code/issues/6017) — the observability surface that carries the raw diffs and history.
- [#5960](https://github.com/developing-today/code/issues/5960) — permission UX, owner of the final wording.
- [#5751](https://github.com/developing-today/code/issues/5751) — invites; the pinning question in §9.1.
- `pkgs/id/crates/id-core/src/world_caps.rs` — the registry vocabulary at `origin/main` `b6bd188624`: `ProviderManifest` (`:202-216`), `CATALOG_ENTRIES` (`:346-491`), `CapLedger` (`:1240`) with `decide_for` (`:1302`) and `effective` (`:1358`), `CapsReport` (`:1426-1454`), `is_sensor_cap` (`:1040-1042`), `UNKNOWN_CAP_USAGE_KEY` (`:107`).
- `doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings/` — the effective-authority formula the diff reuses.
- Chrome permission warnings (the precedent #5999 names): <https://developer.chrome.com/docs/extensions/develop/concepts/permission-warnings>; permission-collapse behaviour: <https://chromium.googlesource.com/chromium/src/+/a2e1372aaa6f2431a99e41489725f39561cd6b79/extensions/docs/permissions.md>.
