# Design: settings the owner changes from the app, kept at the edge (#14163, remainder slice 1)

Date: 2026-11-25. Design lane: felix (queue task 23728). Issue: #14163,
acceptance item 1 — "defaults carried by the deploy (from the
repository), and what the owner changes from the app kept at the edge.
One place says which is in force and since when."
Related: #14172 / #14185 (steps 1–2, merged and deployed), #13495
(sign-in settings — the deployment's; **out of scope here**), #13492
(second machine), #14158 (tracking).

Status of this record: **design of record for the owner-changed-settings
remainder of #14163.** It designs the override layer only; no code in
this PR. Implementation slices are filed from §7.

## 1. Where things stand on main (verified 2026-11-25)

Written against main at `941ec5e3e836807bafac41bcd94cd00168f378e2`
(the design worktree's base; the settings and edge files below are as
carried by the current deploy line).

- The platform's own settings are written once in
  `pm-stack/console/src/settings.ts`: `platformSettings()` returns the
  seven `PlatformSettings` (`repo`, `queue`, `queueDbUrl`,
  `queueTokenName`, `androidPackage`, `promptLabel`, `needsRepos`;
  `PLATFORM_SETTING_NAMES` fixes their order). Both sides carry them:
  the console on a machine as its defaults (`src/config.ts`, where the
  environment wins over them), and the Worker bundled into every deploy.
- The Worker goes by its own for six of them (`PLATFORM_OWN`);
  `needsRepos` alone is still a reporting machine's (`MACHINES_FOR_NOW`,
  with the reason recorded there: a machine answers only for items in a
  repository on its own list). `goesBy(told)` in settings.ts is the one
  merge: `{...told, ...own}`.
- At the edge (`pm-stack/console/edge/src/edge.ts`), what a machine
  told is kept as told in the `PmEdge` Durable Object's storage under
  the key `"cfg"` (written by the sync handler, only when it changes),
  and `Edge.config()` derives what is gone by from it:
  `told === null ? null : goesBy(told)`, cached per object lifetime.
  The `KV` interface the edge uses (`edge/src/edge-shared.ts`) is that
  object's storage behind `get`/`put`/`delete`
  (`edge/src/worker.ts`: `ctx.storage`).
- Reporting today: the health answer carries `machineSettingsDiffer`
  — the *names* in which a machine differs from the repository, never
  values (`settingsThatDiffer`). The Machines screen's answer
  (`machinesSaid`, `MachinesAnswer` in `src/edge-proto.ts`) carries
  `settingsDifferences`: per differing setting, both values and which
  side the platform goes by (`"platform"` / `"machine"`). That answer
  is owner-only; health is the public, names-only surface.
- The sync answer (`SyncResponse`, `src/edge-proto.ts`) is how the
  edge talks back to a reporting machine; today it carries no settings.
  A machine's console builds its config from its environment over
  `platformSettings()` (`src/config.ts`) and reports the result in
  every sync (`msg.config`, validated whole by `edgeConfig()`).
- Owner actions at the edge already exist as a mechanism: the owner,
  verified as elsewhere in edge.ts, takes actions the edge either does
  itself (`ownActions`, `src/actions.ts`) or holds for a machine
  (`held`, kept under `"held"`), and every such doing is written to the
  audit log kept under `"audit"` (`Edge.audit()`, `AUDIT_KEPT` entries).
- Sign-in is **not** a platform setting: `owners`, `issuers`,
  `certNames`, `androidFingerprints` live in `EdgeConfig`, and
  settings.ts says why — they are set when the hostname is, and are the
  deployment's. #13495 owns that remainder. Nothing below touches them.

**The gap this design closes:** there is no third source. A setting is
the repository's or (for `needsRepos`) a machine's; the owner cannot
change any of them from the app, and nothing at the edge records a
change, when it happened, or who made it.

## 2. Decisions

### D1. A separate, named class: owner-changeable settings

Not all seven settings become owner-changeable. A setting joins the
class only if changing it from the app is safe with no deploy and no
machine present:

- It cannot widen who may sign in (sign-in fields are not
  `PlatformSettings` at all, so this holds structurally).
- It cannot move where a credential is sent or what it is called:
  `queueDbUrl` and `queueTokenName` stay repository-only — an app-side
  typo there silently retargets the queue token.
- It is not the platform's identity: `repo` and `queue` stay
  repository-only; changing which repository the work is in is a
  migration, not a preference.
- `androidPackage` stays repository-only: it must agree with the
  shipped app and the asset-links the edge serves; a mismatch breaks
  the app link, not a preference.

The initial class is therefore exactly two settings:

- **`promptLabel`** — the label that marks an issue as an instruction
  from the owner. Purely presentational routing; a wrong value loses
  no data (unlabelled items simply are not picked up as prompts).
- **`needsRepos`** — the repositories looked in for what needs the
  owner. The owner is the person who knows which repositories they
  watch. An override here also settles, for the overridden case, the
  `MACHINES_FOR_NOW` tension: the platform goes by the owner's list,
  not a machine's (see D3 for what a machine then does).

The class is a new constant in settings.ts, `OWNER_CHANGEABLE =
["promptLabel", "needsRepos"]`, beside `PLATFORM_OWN` and
`MACHINES_FOR_NOW`, so the three classes read together in the one file
that already owns the vocabulary. Adding a setting to the class later
is a repository change (a PR), never an app action — the app can only
change values of settings already in the class.

### D2. Storage: one record in the edge object's own storage

Owner-changed settings live where everything else the edge keeps
lives: the `PmEdge` object's storage, through the same `KV`
interface, under one new key, **`"ownerSettings"`**, alongside `"cfg"`
— never inside it. `"cfg"` is a machine's word and is overwritten by
the next differing sync; an override stored there would be silently
erased by a machine report, which is the failure #14163 exists to end.

The record is one value, written whole (the object's storage reads
back whole or not at all, and the Durable Object serialises its
requests, so one record needs no locking):

```ts
interface OwnerSettings {
  /** Monotonic revision, starting at 1; every accepted change adds 1. */
  rev: number;
  /** When this revision was accepted, milliseconds (the edge's clock). */
  at: number;
  /** Who made it, as the edge verified them (the owner identity the
   * request was answered as). Never a machine: sync cannot write this
   * record (D5). */
  by: string;
  /** The overridden values, only for OWNER_CHANGEABLE names. Absence
   * of a name means "no override": the repository (or, for
   * needsRepos unoverridden, a machine) speaks for it. */
  values: Partial<Pick<PlatformSettings, "promptLabel" | "needsRepos">>;
}
```

Properties this shape buys:

- **Kept at the edge, across deploys.** Deploys ship the Worker's
  bundle; object storage is not in the bundle. An override survives
  every deploy until the owner clears it — which is the point of
  acceptance item 1, and also its hazard, handled in D4.
- **One place, since when.** The record *is* the one place: which
  settings are overridden, to what, since `at`, by `by`, at revision
  `rev`. §5's reporting is a projection of this record plus
  `platformSettings()`, not a second store.
- **History without a second store.** Each accepted set/clear also
  appends to the existing audit log (`Edge.audit()`), entries
  `{kind: "setting.set" | "setting.clear", name, rev, at, by}` —
  values are not written to the audit log (the current values are in
  the record; the log answers *that* a change happened, and when).
- **Bounded.** At most two names, validated as in D5; the record is a
  few hundred bytes beside `"cfg"`, `"held"` and `"audit"`.

### D3. Precedence: repository, then machine (for one), then owner

`goesBy()` grows from two layers to three, in settings.ts, as a new
pure function beside it — the merge stays in the one file both sides
already share:

```ts
effectiveSettings(told, owner, platform = platformSettings())
```

Order, lowest to highest:

1. **The repository** (`platformSettings()`, as the deploy carries
   it) — the default for every setting.
2. **A machine's told value**, for `needsRepos` only, and only while
   no owner override for it exists — today's `MACHINES_FOR_NOW`
   behaviour, unchanged.
3. **The owner's override**, for `OWNER_CHANGEABLE` names present in
   the record — above the repository for both, and above a machine's
   told list for `needsRepos`.

Everything else about today's merge is unchanged: the six
`PLATFORM_OWN` settings still ignore a machine's told values, and no
layer here can produce a sign-in value — `EdgeConfig`'s sign-in
fields pass through exactly as today (the deployment's, via the
machine that reports the hostname's setup, until #13495).

At the edge, `Edge.config()` becomes: read `told` as today, read the
owner record (cached like `cfg`, invalidated on a change made through
this object), and derive `goesBy`-then-override. The derived config is
still an `EdgeConfig`, so every existing consumer — `directClasses`,
`owned()`, the census's repository list, release serving — sees the
effective settings with no signature changes. One deliberate
consequence: when the owner overrides `needsRepos`, the edge reads
the census by the owner's list even while machines report another,
and a machine is *told* the effective list (D6) so what it answers
for agrees.

### D4. Clearing is a first-class act; staleness is shown, not hidden

- **Clear** removes one name from `values` (or the whole record),
  bumping `rev` and `at` like a set. With no override, the repository
  speaks again from the next deploy — no, from that moment: the
  repository's *current bundle* value, which is whatever main said at
  the last deploy.
- **The hazard:** an override outlives the repository change it was
  compensating for, and silently shadows a later deliberate change to
  the same setting in main. Mitigation is reporting, not expiry (an
  expiring preference is a surprise of the opposite kind): the
  in-force report (§5) shows, for every overridden setting, the
  repository's current value beside the override, so "the repository
  now says X, the owner overrode it to Y on <date>" is visible in the
  one place. The Machines/settings surface (§6) shows the same pair.
  No automatic expiry, no deploy-time clearing — either would make a
  deploy able to undo the owner, which is the laptop failure mode in a
  new costume.

### D5. The write path: owner actions, validated whole, owners only

- New own-action kinds in the edge's existing action mechanism:
  **`settings.set`** (`{name, value, baseRev?}`) and
  **`settings.clear`** (`{name?, baseRev?}`), done by the edge itself
  (never held for a machine — a machine must not be in this path at
  all). They ride the same owner verification as every other action
  the edge answers; a request the edge is not sure is the owner's is
  passed to the host as today, and the host has no such action, so it
  cannot happen there either.
- **Validation is whole-or-nothing**, in the spirit of `edgeConfig()`:
  the name must be in `OWNER_CHANGEABLE`; the value must be the
  setting's shape (`promptLabel`: a non-empty label-shaped string;
  `needsRepos`: a list of `owner/name` strings, de-duplicated, at
  most the dashboard census's order of magnitude — the validator
  reuses the string-list checks in edge-proto.ts); anything else
  refuses the action entire, record untouched.
- **Optimistic concurrency:** `baseRev`, when given, must equal the
  record's current `rev` or the action is refused with the current
  record in the answer (the app re-reads and the owner decides again).
  The single-object serialisation already prevents interleaved
  writes; `baseRev` prevents the two-tabs problem, not a race.
- **Machines cannot write it.** The sync handler ignores any
  owner-settings-shaped field in a report — there is no such field in
  `SyncRequest`, and none is added. A machine's report keeps doing
  exactly what it does today: updating `"cfg"` (told, for comparison)
  and nothing else about settings.

### D6. How a machine learns the effective settings

The sync answer is the channel that already exists. `SyncResponse`
gains one optional field:

```ts
/** The effective owner-changeable settings, when any differ from
 * what this machine's own defaults would give, with the revision. */
settings?: { rev: number; values: Partial<…> };
```

- Sent when the owner record exists and its values differ from what
  the machine's report implies it is using, and whenever `rev`
  changes; absent otherwise (older edges simply never send it, older
  consoles ignore unknown fields — the field is additive, like the
  `EdgeConfig` fields before it).
- The console's sync client (`src/sync.ts`) hands it to the console's
  config: for `OWNER_CHANGEABLE` names, the edge's effective values
  win over both the environment and `platformSettings()` when the
  console next builds its config — the same precedence as D3, seen
  from the machine. The machine records the `rev` it has applied and
  reports it in its next sync (one new optional `MachineReport`-sibling
  field, or in `have`-style metadata — slice 5 picks the exact slot),
  so the edge can say "machine X runs override rev 3" and, until it
  does, the machine shows as *not yet on the effective settings* —
  said, not acted on, exactly as #14163 item 3 already treats a
  machine on other settings.
- Until a machine applies them, nothing breaks: the edge goes by the
  effective settings for its own work; a machine that has not applied
  an overridden `needsRepos` may refuse a reply for a repository only
  on the owner's list — the held-action mechanism already holds such
  an action for a machine rather than losing it, and the Machines
  screen says why it waits.

## 3. What "in force" means, per setting

For each name in `PLATFORM_SETTING_NAMES`, exactly one source is in
force at any moment:

| Setting | Sources, highest first | In force today |
|---|---|---|
| `repo`, `queue`, `queueDbUrl`, `queueTokenName`, `androidPackage` | repository | repository |
| `promptLabel` | **owner** → repository | repository |
| `needsRepos` | **owner** → machine → repository | machine (in service: equal to repository since 2026-11-24, per #14163) |

Sign-in fields are not in this table: they are not platform settings
(#13495).

## 4. Reporting: one place, two surfaces (extends today's, replaces nothing)

The projection is computed in settings.ts as a pure function both
sides can use:

```ts
interface SettingInForce {
  name: keyof PlatformSettings;
  source: "owner" | "repository" | "machine";
  /** When the in-force value took effect: the override's `at` for
   * source "owner"; null otherwise (a repository value took effect
   * at the deploy, which the edge does not date today; a machine's,
   * at its last report). */
  since: number | null;
  value: unknown;          // owner-surface only
  repository: unknown;     // what the repository says, for contrast
  by?: string;             // source "owner" only
  rev?: number;            // source "owner" only
}
settingsInForce(told, owner, platform): SettingInForce[]
```

- **Health (public, names-and-sources only)** gains
  `settingsInForce: {name, source, since}[]` beside
  `machineSettingsDiffer`. No values, no `by` — health today never
  carries values, and an override's value (a repository list, a
  label) is the owner's business. `machineSettingsDiffer` stays as
  is: it answers a different question (where a *machine* disagrees),
  and removing it would break the checks that read it.
- **The Machines answer (owner-only)** gains `platform.settings:
  SettingInForce[]` with values, the repository's contrast value,
  `by` and `rev` — this is the one place the owner reads "which
  settings are in force and since when", on the screen that already
  shows where machines differ. The app (§6) renders it as the
  settings section of that screen.

## 5. The app surface

The Machines screen gains a **Platform settings** section, drawn from
`platform.settings`: each of the seven settings with its in-force
value, its source, and since when; for the two owner-changeable
settings, an edit affordance (set, and clear when overridden) that
issues the §D5 actions, and — when overridden — the repository's
value shown beside the override (D4). Repository-only settings are
shown read-only with their source; there is no disabled-looking
control that implies they could be changed. Sign-in is not shown as a
setting here at all (it is the deployment's; #13495's surface, when
it comes, is its own).

## 6. What this design does not do

- No implementation in this PR; §7's slices carry it.
- No sign-in settings, in any layer (#13495).
- No change to `PLATFORM_OWN` / `MACHINES_FOR_NOW` behaviour for
  unoverridden settings; `needsRepos` becomes platform-owned in
  general only with #13492's work, not here.
- No secrets: no setting in any class is a secret value (the queue
  token is referred to by name today and stays that way).
- No new storage system, no KV namespace, no migration: the record
  lives in the `PmEdge` object's existing storage.

## 7. Implementation slices (filed from this design)

Each is independently landable, in this order; S3–S5 each depend only
on S2 (and S1), not on each other. Filed 2026-11-25: S1 #14953
(enqueued as queue task 23732), S2 #14954, S3 #14955, S4 #14956,
S5 #14957. S2–S5 are deliberately not enqueued yet: each is enqueued
in the pass that lands its predecessor.

1. **S1 — the vocabulary and the merge (settings.ts).**
   `OWNER_CHANGEABLE`, the `OwnerSettings` shape, `effectiveSettings()`
   and `settingsInForce()` as pure functions, with tests in
   `pm-stack/console/test/settings.test.ts` naming their oracle (the
   precedence table in §3). No edge or app change.
2. **S2 — the edge keeps and goes by the record.**
   `"ownerSettings"` read/write in edge.ts behind the `KV` interface,
   `Edge.config()` deriving through `effectiveSettings()`, audit
   entries, sync explicitly not writing it; edge tests
   (`test/edge.test.ts`) for precedence in the live config path and
   for a sync carrying override-shaped data changing nothing.
3. **S3 — reporting.** Health's `settingsInForce` summary and
   `MachinesAnswer.platform.settings`, from `settingsInForce()`;
   tests on both surfaces (values absent from health, present in the
   Machines answer).
4. **S4 — the write path and the app.** `settings.set` /
   `settings.clear` own-actions with D5 validation and `baseRev`,
   plus the Machines screen's Platform settings section; app and
   edge tests.
5. **S5 — machines learn.** `SyncResponse.settings`, the console's
   adoption of effective values in `src/config.ts`/`src/sync.ts`,
   the applied-`rev` report-back, and the Machines screen saying a
   machine has not applied the current revision yet; console and
   edge tests.

Acceptance for the parent item (#14163 item 1) is met when S1–S4 are
in service (the owner can change a setting from the app, it is kept
at the edge, and one place says what is in force and since when) and
S5 has landed (machines agree with the platform). Seen-in-service
check, after S4 deploys: change `promptLabel` from the app with the
laptop off, watch the health answer's `settingsInForce` name it
`source: "owner"` with the change's time, then clear it and watch
the source return to `"repository"`.
