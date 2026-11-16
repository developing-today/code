# Design: moderation pack format — data-only packs with a static policy core, feed subscriptions, and per-world parameters

- **Date:** 2026-11-09
- **Status:** design spec only — no implementation built from it in this change.
  Implementation slices are named in §9 and must be filed as their own issues.
- **Issue:** [#6005](https://github.com/developing-today/code/issues/6005).
  **Program:** [#5955](https://github.com/developing-today/code/issues/5955)
  (worlds design program). **Parent topic:**
  [#5973](https://github.com/developing-today/code/issues/5973) (world
  governance — moderation as parameterized libraries). This note pins the
  **data format** a pack is; #5973's interface design adopts the format, it
  does not re-decide the interface.
- **Composes with, does not replace:**
  [#6006](https://github.com/developing-today/code/issues/6006) (threat-list
  feed protocol — §4 of this note references its subscription contract
  verbatim and does not restate the feed protocol),
  [#5753](https://github.com/developing-today/code/issues/5753) (KDL
  substrate) and the KDL ⇄ JSON canonical conversion contract in
  `doc/2026-11-08_decision_id5906_kdl_json_contract.md`,
  [#5545](https://github.com/developing-today/code/issues/5545) (writer-set
  authority — who may change a pack is a writer-set question, §7),
  [#6007](https://github.com/developing-today/code/issues/6007) (preset tier
  defaults — a pack may _name_ a preset; the preset's contents are #6007's,
  not this note's).
- **Scope guard (from #6005):** the pack format only. No moderation
  runtime, no board behavior, no preset contents, no feed protocol.

---

## 0. The rules everything follows

1. **A pack is data, never code.** A pack contains declarations — rules,
   subscriptions, parameters — and nothing executable: no scripts, no
   expressions that evaluate at ingest, no hooks. This is the finding
   #6005 is built on: every successful cross-community moderation sharing
   system is structured data (Discord AutoMod rule objects, curated
   blacklist feeds plus policy presets); rules-as-code packs have no
   successful mainstream precedent, and executable shared moderation is
   the Garry's Mod failure mode. If a behavior cannot be expressed as
   data in this format, it belongs in a moderation _library_ under
   #5973 — never smuggled into a pack.
2. **The shared core and the local parameters are different objects.**
   A pack has a static, shareable **core** (policy rules + feed
   subscriptions) and a **parameter block** that is per-world by
   construction (§3). Adopting a pack means adopting its core verbatim
   and supplying your own parameter values; there is no edited-copy
   adoption. A world that wants different rules forks the pack — and a
   fork is a new pack with a new identity (§1), never a silent variant,
   because silent forking is the Reddit AutoModerator copy-paste failure:
   downstream copies never receive updates.
3. **Local always wins.** Local exemption and override lists (§5)
   outrank core rules and every feed entry, in both directions — the
   same rule the feed protocol states for feeds, applied one level up.
   No pack update can edit, shadow, or expire a local entry.
4. **Validity is strict, application is whole.** A pack (or pack update)
   that fails signature, hash, schema, or version checks is rejected
   whole; the world keeps enforcing its last good pack state. Partial
   application is forbidden (feed protocol rule 3, applied to packs).
5. **Deterministic evaluation.** Two worlds at the same pack version,
   the same feed heads, and the same local lists compute the same
   effective decision for every subject, modulo their parameter values
   (§6). Parameters are inputs to that computation, never hidden state.
6. **Every decision is explainable.** For any subject, a world can
   answer "why was this quarantined / allowed?" with the exact chain:
   local list hit, or core rule id, or feed id + entry + feed head seq,
   or the parameter that fired (e.g. report count ≥ quarantine
   threshold), or pack default. A rule or parameter that cannot appear
   in such an explanation does not belong in the format.

## 1. Pack identity

A pack is a signed object in the `pkgs/id` signed-object shape — the
same substrate the feed protocol reuses — not a new envelope:

| Field          | Meaning                                                                                                                                                        |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `author`       | The pack author's Ed25519 verifying key (32 bytes). The author is the pack's identity root; there is no separate registry.                                     |
| `pack_id`      | BLAKE3 over the canonical encoding of the pack's **genesis object** (§2). Stable for the pack's life; a fork gets a new genesis and therefore a new `pack_id`. |
| `name`         | Human-readable, non-unique, display only. Never used for resolution.                                                                                           |
| `seq` / `prev` | Per-author sequence number and prev-hash link, exactly as in the feed protocol's object chain. A pack version is one object in this chain.                     |
| `id`           | BLAKE3 over the encoded bytes of this version object, including the signature. Citing a pack version means citing this `id`.                                   |

A world **pins** a pack the same way a pack pins a feed (feed protocol
§4): by `pack_id` + author key. A subscription to a pack tracks the
author's chain from the pinned head; a chain fork is handled by the
feed protocol's fork rule, unchanged (enforce nothing newer from either
branch; resolution is a human act).

## 2. Pack object: the static core

One pack version object carries the whole core. Cores are **complete,
not diffs**: a version restates every rule and subscription, so any
version can be evaluated standalone and two versions can be compared
by simple structural diff. (Feed updates are chains of entries because
feeds are high-frequency; packs are low-frequency policy, and the
auditability of a complete restatement is worth more than delta
compactness.)

### 2.1 Rules

Each rule in the core is a declaration with:

| Field     | Meaning                                                                                                                                                                                                                                      |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rule_id` | Author-scoped stable string (e.g. `spam/first-post-links`). Stable across versions while the rule's intent is unchanged; a rule whose action changes materially gets a new `rule_id`.                                                        |
| `subject` | What the rule matches: a subject kind from the feed protocol's entry kinds (principal, content hash, domain, …). The pack format defines no new subject kinds.                                                                               |
| `match`   | The data the subject is matched against (literal values, or a reference to a named list _inside this core_). No patterns that require evaluation beyond literal/prefix membership — see rule 1 of §0.                                        |
| `action`  | One of the feed protocol's action vocabulary (`allow`, `flag`, `quarantine`, `deny` — the exact enum is #5973's interface to fix; this format requires only that the action be a value from a named, versioned vocabulary, never free text). |
| `note`    | Human-readable rationale, display only. Required, so every rule can appear in a §0-rule-6 explanation with its author's own words.                                                                                                           |

### 2.2 Feed subscriptions

A core subscribes to feeds **by reference to the feed protocol's
subscription contract** (`doc/2026-11-05T13-36-21Z_design_threat_list_feed_protocol.md`
§5), with exactly the fields that contract defines — feed id, pinned
key (or writer-set reference once #5545 lands), `rank`, mode
(`advisory` / `enforce`), and per-feed volume caps. This note adds no
subscription fields and changes no subscription semantics; if the feed
contract changes, pack subscriptions follow it by reference.

### 2.3 What the core must not contain

- Parameter **values** (§3). A core may declare that a parameter exists
  and what its schema is; it may not set it.
- Local lists (§5). They are world state, not pack content.
- Executable content of any kind (§0 rule 1), including "harmless"
  templating in `note` fields: notes are literal text.

## 3. Parameters: the per-world block

Parameters are how "the same thing, different moderators, different
threshold" (#5973) is a format property instead of a fork. The core
declares a **parameter schema**; each adopting world supplies a
**parameter block** — its own signed pack-side object, not part of the
pack object, versioned with the world's own pack-change chain (feed
protocol §5: subscribing is a world-side act recorded as a pack
change; supplying parameters is the same kind of act).

The parameter schema declares, per parameter: `name`, `type`,
`default` (used when a world's block omits it), and `range` where the
type has one. The format fixes the schema _mechanism_; the following
parameters are the ones #6005 names, and every conforming core that
supports the corresponding behavior must declare them under these
names so packs stay comparable across worlds:

| Parameter                     | Type                 | Meaning                                                                                                                                                                     |
| ----------------------------- | -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `report_quarantine_threshold` | integer ≥ 1          | Distinct-reporter count at which reported content is auto-quarantined pending review.                                                                                       |
| `moderators`                  | set of principal ids | Principals holding moderator powers under this pack in this world.                                                                                                          |
| `reviewers`                   | set of principal ids | Principals who may review quarantines and appeals. May overlap `moderators`; neither set implies the other.                                                                 |
| `local_exemptions`            | reference            | Names the world's local exemption list (§5). A reference, never the list itself — the list is world state.                                                                  |
| `appeal_window`               | duration             | How long a quarantined/denied subject's author may appeal. (Declared here because #5973 names appeal as a hook; its default is a preset question for #6007, not this note.) |

A world **may not** narrow a core rule by parameter. Parameters tune
thresholds, people, and windows; they never disable, reorder, or
rewrite rules. A world that needs a rule gone forks the pack (§0
rule 2) or overrides the specific subject locally (§5) — both are
visible acts; a parameter that silently guts a rule is neither.

## 4. Serialization: the KDL profile, and the canonical form

- **Authoring form: KDL**, in the manifest profile of #5753 — the same
  profile #5973 names for moderation-library configuration. A pack is
  authored, reviewed, and diffed as a KDL document. One pack version =
  one KDL document whose root node is `moderation-pack`, with child
  nodes `rule`, `subscribe`, and `parameter-schema` mirroring §2–§3.
- **Canonical form: the #5906 contract.** For hashing, signing, and
  storage, the KDL document is converted under the KDL ⇄ JSON
  canonical conversion and round-trip contract
  (`doc/2026-11-08_decision_id5906_kdl_json_contract.md`): canonical
  JSON is what gets BLAKE3-hashed and Ed25519-signed, and the `id`
  fields of §1 are over that canonical form. This note chooses no new
  serialization and defines no conversion rules of its own; anything
  the #5906 contract leaves open for packs is an amendment to that
  contract, not a pack-local exception.
- **No second wire format.** The feed protocol's signed-object
  envelope carries the canonical bytes; there is no pack-specific
  binary encoding.

## 5. Local lists

Local exemption and override lists are **world-side pack-change
objects** in the feed protocol's entry shape (§5 of that protocol:
local lists are pack-side lists in entry format), referenced from the
parameter block (§3), never embedded in a pack. Their precedence is
§0 rule 3. A pack update, a feed update, and a parameter change are
all powerless to alter them; only a world-side pack change signed
under the world's own authority (#5545 writer set, once landed) can.

## 6. Evaluation sketch (informative)

For a subject, in order — the feed protocol's §6 order, with the
pack's parameters made explicit:

1. Local lists: exemption → allow; override listing → its action.
   Stop.
2. Feed entries from the core's subscriptions, by subscription
   `rank` and the feed protocol's merge rules, in the subscription's
   mode. Stop at the deciding entry per that protocol.
3. Core rules in declaration order; first matching rule decides.
4. Report state: if the subject's distinct-reporter count ≥ the
   world's `report_quarantine_threshold` parameter, quarantine
   pending review by the world's `reviewers`.
5. Pack default: allow.

Steps 1–3 and 5 are identical for every adopter of the same pack
version at the same feed heads; step 4 is where per-world parameters
legitimately make outcomes differ (§0 rule 5).

## 7. Versioning and compatibility

- **Format version.** The pack object carries `format_version`
  (this note defines version 1). A reader that does not understand a
  pack's `format_version` rejects the pack whole (§0 rule 4); there
  is no partial or best-effort read of an unknown format version.
- **Pack versions** are the §1 chain: `seq` increments, `prev` links,
  each version a complete core (§2). An adopting world records the
  pack `id` it currently enforces; moving to a newer version is a
  world-side pack change, never an automatic pull — an author
  publishing a new version changes nothing in any adopting world
  until that world accepts it. (Feeds auto-advance because entries
  expire; policy does not get that treatment.)
- **Compatibility within a format version:** adding a rule,
  subscription, or optional parameter is compatible; removing or
  renaming a `rule_id`, changing a parameter's type or narrowing its
  range, or changing an action's meaning is **breaking** and requires
  the author to state it in the version's `note` — and a world should
  treat a breaking version like a new pack for review purposes.
  Unknown fields in a known format version are stored and not
  applied (the feed protocol's unknown-kind rule).
- **Schema evolution** of the format itself goes through a new
  `format_version` and an amendment to this note, never through
  per-pack extensions.

## 8. What this note deliberately does not decide

- The moderation-library **interface** (hooks, capability set,
  platform- vs world-provided pieces) — #5973.
- The action vocabulary's exact enum — #5973, constrained only as
  §2.1 states.
- Preset packs and tier defaults — #6007. A core may declare
  `preset` in its parameter schema naming a #6007 preset as the
  source of default parameter values; the preset's contents and
  tiers are #6007's.
- Feed protocol mechanics — #6006, referenced, not restated.
- Who may author/publish packs, and pack discovery — #5545 writer-set
  territory and a future distribution design; this format only
  requires that a pack be pinnable by author key (§1).

## 9. Implementation slices (for #5973 to adopt or file)

1. KDL profile schema for the `moderation-pack` root node (§4),
   validated against the #5906 conversion contract's round-trip tests.
2. Pack object encode/verify in `pkgs/id` beside the feed objects
   (§1–§2), reusing the signed-object substrate.
3. A conformance fixture set: two worlds, one pack, different
   parameter blocks, asserting §6 step-4 divergence and step 1–3/5
   agreement — the §0 rule 5 property, tested.
4. #5973's interface design cites this format for its pack parameter
   surface (§3) instead of re-specifying it.

---

_Evidence baseline: `origin/main` at drafting (2026-11-09); the feed
protocol spec and the #5906 KDL/JSON decision record read in full;
issues #6005, #5973 read in full. No merged PR implements any part of
this format yet; PR #7708 (referenced from the #6005 packet) is an
adjacent input, not an implementation of this spec._
