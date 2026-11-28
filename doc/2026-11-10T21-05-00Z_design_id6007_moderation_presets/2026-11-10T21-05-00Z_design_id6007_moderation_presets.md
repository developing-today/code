# Moderation preset tiers: named presets that compile to concrete rules

Status: design for #6007 (worlds design program #5955; parent topic #5973).
Wave-2 follow-up from the prior-art research: what made third-party
moderation bots adoptable was legibility — Paladin maps a content rating
(G/PG/PG-13/R) to preset AutoMod configurations and re-syncs them, and
admins choose a tier, not a rule list. This record defines id's version:
a small set of named tiers for world moderation packs, the concrete rule
schema they compile to, and how compilation, overriding, and re-sync work.

## Grounding: what exists today

- The moderation log is append-only and board-scoped (`boards.rs`):
  `ModerationAction{board, target, action, reason}` with actions `Hide`
  and `Restore`. Entries record; they "do not enforce a view" —
  enforcement lives in the view layer (`BoardView` folds the log plus
  the personal layer of per-viewer hides/mutes).
- Moderation authority is the `MODERATOR` world scope (#6004); the
  reserved/delegable split there is untouched by this design — presets
  configure *when the machinery acts*, never *who holds authority*.
- There is today: no report flow, no quarantine state, no content
  filter, no pack format. The tier table below therefore marks each
  lever **exists** or **new**, and quarantine is expressed through the
  existing log (see "Compilation target"), not a new state machine.

## The rule schema (what a preset compiles to)

A compiled rules record is a flat, versioned struct — deliberately few
fields, each mechanically checkable:

| Field | Values | Mechanism |
|---|---|---|
| `filter_mode` | `off` / `flag` / `hold` | **new** wordlist filter: `flag` marks matches in the view layer for moderator attention; `hold` keeps matches out of the board view until a moderator releases them |
| `wordlist_pack` | pack hash or `none` | **new**: named, hash-pinned wordlist packs (contents are a separate governance question, below) |
| `report_autohide_threshold` | integer or `off` | **new**: distinct-author reports within the window that trigger an automatic hide |
| `report_window_hours` | integer | **new** |
| `preapproval` | `none` / `new_authors` / `all` | **new**: posts in scope are held from the board view until moderator release |
| `link_hold` | `none` / `new_authors` / `all` | **new**: posts containing links are held for release under the same hold machinery |
| `hold_release_scope` | fixed: `MODERATOR` | **exists** (#6004 scope): only moderator-scope signers release holds |

Holds are not a new object type: a hold is the *absence* of view-layer
inclusion until release, and an auto-hide is a `Hide` log entry signed
by the world itself with `reason` naming the rule that fired
(`report_threshold`, `filter_hold`, `preapproval`, `link_hold`).
Release is a `Restore` entry (for auto-hides) or the view-layer release
record (for holds). Every automatic action is therefore visible in the
same append-only log a human moderation action uses, and reversible by
the same means. Nothing in this schema widens any authority: the world
key signing an auto-hide can only do what the compiled rules say, and
the rules are part of the world's published pack (below).

## The tier table

Three tiers. Names are user-facing vocabulary; world browsers display
them, so they are plain words, not internal codenames.

| Field | **Open** | **Standard** | **Strict** |
|---|---|---|---|
| `filter_mode` | `off` | `flag` | `hold` |
| `wordlist_pack` | `none` | baseline pack | baseline pack |
| `report_autohide_threshold` | `off` | 5 | 3 |
| `report_window_hours` | — | 24 | 24 |
| `preapproval` | `none` | `none` | `new_authors` |
| `link_hold` | `none` | `new_authors` | `all` |

- **Open** — for worlds that moderate by hand. The log and human
  Hide/Restore work exactly as today; nothing automatic fires.
- **Standard** — the default recommendation for public worlds. Spam
  and abuse are flagged, and a post five distinct authors report within
  a day leaves the view until a moderator looks — but nothing is held
  back pre-publication, so conversation is never gated.
- **Strict** — for worlds expecting hostile traffic. Filter matches
  and links wait for a moderator, and new authors' first posts wait
  too. The cost is moderator latency on every first interaction; the
  tier name should be chosen knowing that.

`report_autohide_threshold` counts **distinct authors** (not raw
reports), so one determined reporter cannot hide content alone. Report
weighting beyond distinct-count (account age, board standing) is a
follow-up, not v1.

## Compilation, overrides, re-sync

- A **moderation pack** (the sibling format work under #5955) names a
  tier and a tier-table version. Adopting a pack compiles the tier to
  the concrete rules record above; the compiled record — not the tier
  name — is what the machinery executes, and the record is stored with
  the world so anyone can inspect what is actually in force.
- A world may then adjust individual fields. The preset is a starting
  point with a name, not a lock. Browsers display the base tier plus a
  **customized** marker when the effective record differs from any
  tier's compiled output (`Standard (customized)`), and the pack view
  lists which fields were overridden — legibility is the product, so
  drift is shown, never hidden.
- When a tier table is republished at a new version, re-sync follows
  the Paladin pattern: fields the world has **not** overridden take the
  new compiled values; overridden fields stay pinned and are listed as
  pinned in the pack view. Re-sync is itself an append-only record, so
  a world's rule history is auditable end to end.
- Tier-table versions are immutable once published; a fix is a new
  version, never an edit — the same rule the log itself follows.

## Boundaries

- **#6004 (governance):** presets never grant, widen, or attenuate
  scopes. `RevokeAuthorized`, moderation-scope grants, and the
  reserved-authority list are unaffected; a pack cannot compile to a
  rule that acts outside the board it belongs to.
- **#5839 (content model):** read authority resolves at the note;
  holds and hides operate on board views of notes, never on the note's
  own authority. A hidden note is still the author's note.
- **Personal layer:** per-viewer hides and mutes are a separate layer
  (already in `BoardView`) and are not preset-controlled; a world's
  tier can never reach into a viewer's personal layer.

## Non-goals for v1

- No cross-world rule-pack subscription marketplace. The precedent
  review found no major platform offering first-party pack
  subscription with versioning; id is ahead of the field by having
  named, versioned, compiling presets at all — keep them few and
  simple rather than building distribution machinery now.
- No classifier/ML filtering. v1 levers are wordlists, reports, and
  preapproval — all inspectable, all explainable in one line.
- No global tiers. A tier binds to one world's pack; there is no
  platform-wide moderation posture in this design.

## Open questions (follow-ups, not blockers)

1. Wordlist pack contents and their governance: who publishes the
   baseline pack, how entries are appealed, and how a world sees what
   a pack would match before adopting it (a dry-run report against the
   world's recent posts would preserve legibility).
2. Report weighting and Sybil resistance beyond distinct-author counts.
3. Hold-queue UX: moderators need one queue across hold reasons; the
   log already carries the reason, so this is view work, not format
   work.
