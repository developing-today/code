# Decision: native rendering — assertion → OS-standard controls

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5852](https://github.com/developing-today/code/issues/5852),
  "id: design: native rendering: assertion → OS-standard controls
  (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-override mapping"), the native-target bullet of its
  style-override mapping section. Programme:
  [#2056](https://github.com/developing-today/code/issues/2056).
- **Vocabulary consumed:** [#5850](https://github.com/developing-today/code/issues/5850)
  — the style-assertion vocabulary, settled in that issue's decision
  record (agent note, task 12817, 2026-11-08). Its §12 names this
  ticket as the consumer of its §§3–9 for the native mapping table.
  This record is that table, plus the control-selection and
  platform-conflict decisions the parent left to it.
- **Sibling precedent:** [#5853](https://github.com/developing-today/code/issues/5853),
  settled by `doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`:
  presentation overrides do **not** apply to native standard controls.
  This record follows that exclusion; it does not revisit it.
- **Evidence baseline:** `origin/main` at `a97c23c7df`, worktree
  `~/git/fleet/13953-5852-native-rendering`, 2026-11-08. Verified at
  that baseline, consistent with triage task 12344 (2026-11-08):
  issue #5852 is open with no hard-stop labels, no PR references it,
  and no native-control decision record exists in `doc/` on main.

## Decision summary

| Question | Decision |
| --- | --- |
| What renders on a native target | The platform's **OS-standard control** for the semantic element kind (§2) — never a drawn substitute, never a webview (§6) |
| What an assertion changes natively | Only what the platform exposes for that control: semantic colour/tint, typography style, control size/density class, spacing in platform units (§3). Everything else the assertion names is an explicit **platform-default** with a coverage warning, never a silent drop (§3.1) |
| The mapping's source of truth | One table per platform (§§2–3) in the target's theme/token file, keyed by the #5850 role names — content never names a platform control, colour, or class |
| Platform-conflict rule | **Platform wins for behaviour and accessibility; assertion wins for identity, subject to platform clamps** — stated as a decision in §4, adopting the branch the parent named first and #5850 §8 recorded for the vocabulary layer |
| States | Platform-standard state behaviour; the #5850 §5 state set and precedence order are the contract the platform behaviour is checked against, and any platform substitution is reported by tooling (§3.8) |
| Markup vocabulary dependency | Issue #5849 is not settled yet; element kinds here are the parent body's generic kinds (§1, assumption A2). A settled #5849 record that renames kinds re-issues §2's rows, not its decisions |

## 1. Assumptions against the parent (stated, not silent)

Parent issue #5679 is open and has no settled decision record yet.
Following the packet for this ticket and the precedent of the #5850
and #5853 records, this record states its assumptions instead of
waiting for the parent to settle:

- **A1 — the model as written today.** Semantic markup describes
  intent and structure; styling is semantic style assertions over
  named roles; each target renders assertions in its own idiom; on
  native targets the control follows host-OS conventions while
  honouring the assertion as far as the platform allows. If the
  settled parent changes this shape, §§2–4 are re-issued against it.
- **A2 — element kinds are the parent's generic kinds.** Page/section,
  navigation, heading/text, link, form/field, button/action, list/
  table, card, dialog, media, menu, toolbar, sidebar, tabs,
  settings/preferences, master-detail — as listed in the parent body.
  Issue #5849 (semantic markup vocabulary) has no settled record;
  this record does not fork it. Kind renames change §2's row labels
  only.
- **A3 — the #5850 vocabulary as settled.** Dimensions are the closed
  set of #5850 §2 (tone, emphasis, density, text, gap, pad, corner,
  elevation); colour, typography, state, spacing/density/shape/
  elevation roles are #5850 §§3–6; cascade is #5850 §7 (assertions <
  per-target overrides < raw CSS, web only — raw CSS does not exist
  on native targets). No role is added, removed, or renamed here.
- **A4 — no parent decision is changed.** Where the parent posed an
  open either/or that this ticket must close (the platform-conflict
  rule), §4 picks the branch the parent listed first — the same
  branch #5850 §8 recorded — and says so. Anything that would change
  a parent decision is named as out of scope in §7, not decided here.

## 2. Control selection: element kind → OS-standard control

One row per element kind (A2). The named control is the platform's
standard widget for that kind in its current UI toolkit (macOS:
AppKit/SwiftUI standard controls; Windows: WinUI standard controls;
GNOME: GTK 4 / libadwaita standard widgets). A kind with no standard
control on a platform renders as that platform's standard container
for the kind's content and is a platform-default entry (§3.1), never
a custom-drawn substitute.

| Element kind | Mac | Windows | GNOME |
| --- | --- | --- | --- |
| button/action | standard push button (`NSButton` / `Button`) | standard `Button` (WinUI) | standard `GtkButton` |
| form field (text entry) | standard text field | standard `TextBox` | standard `GtkEntry` / libadwaita row entry |
| form field (choice) | standard pop-up button / radio group / checkbox | standard `ComboBox` / `RadioButtons` / `CheckBox` | standard `GtkDropDown` / check / radio widgets |
| form | standard grouped form layout in a window/sheet | standard form layout in window/dialog | libadwaita preferences-style grouped rows (`AdwPreferencesGroup`) |
| link | standard link-styled button/text | standard `HyperlinkButton` | standard `GtkLinkButton` / link-styled label |
| dialog | standard alert/sheet | standard `ContentDialog` | standard `GtkAlertDialog` / libadwaita dialog |
| menu / toolbar | standard menu bar / toolbar | standard `MenuBar` / `CommandBar` | standard `GMenu` popovers / header-bar actions |
| tabs | standard segmented/tab view | standard `TabView` | libadwaita `AdwViewSwitcher` / `GtkNotebook` |
| list/table | standard outline/table view | standard `ListView` / data grid control | standard `GtkListView` / column view |
| navigation / sidebar | standard source-list sidebar | standard `NavigationView` | libadwaita `AdwNavigationSplitView` sidebar |
| settings/preferences | standard settings window form | standard settings page layout | libadwaita preferences window (`AdwPreferencesWindow`) |
| card / media / page-section / master-detail | standard grouped box / image view / split view composition of the above | standard card container / image / split layout composition of the above | libadwaita card style class / picture / split view composition of the above |
| heading/text | standard label with a §3.3 text style | standard `TextBlock` with a §3.3 style | standard label with a libadwaita typography style class |

Behaviour of every control in this table — keyboard operation,
focus traversal, VoiceOver/Narrator/Orca exposure, drag, undo,
context menus — is the platform control's own. The native renderer
configures controls; it does not re-implement them (§4, invariant 2).

## 3. Assertion mapping: every #5850 dimension, per platform

Each dimension below maps to a per-platform standard-control
rendering, or is marked an **explicit platform-default** (§3.1).
"Platform-default" in a cell means: the control keeps its OS
standard rendering for that dimension, the theme file carries the
`platform-default` marker for the (role, platform) pair, and the
coverage tooling emits the §3.1 warning. There are no other kinds
of gap: a role is either mapped or marked, never absent (#5850 §9).

### 3.1 The platform-default rule

Where a platform exposes no standard way to honour a role, the
entry is `platform-default`: the OS rendering stands, the assertion
is recorded as unmapped for that platform in the coverage report
("assertion has no native mapping on \<platform\>; platform default
used"), and the program's essential appearance must not depend on
that role on that platform — the same warning contract as #5850 §9.

### 3.2 Tone — colour roles (#5850 §3)

| Role group | Mac | Windows | GNOME |
| --- | --- | --- | --- |
| surface roles (base/raised/sunken/overlay/inverse) | system background / control / window semantic colours per container kind | WinUI theme background/ layer resources per container kind | libadwaita window/card/popover background variables per container kind |
| content roles (primary/secondary/muted/inverse/link) | label / secondary / tertiary label colours; link colour | primary / secondary text theme resources; hyperlink resource | foreground variables at full / dimmed opacity; link variable |
| accent + accent-content | the app accent / `controlAccentColor`, with content colour chosen by the platform for legibility | the system accent (`Accent` theme resource) and its on-accent resource | the libadwaita accent background/foreground variable pair |
| status roles (success/warning/danger/info, colour + surface tint) | system semantic colours (green/yellow/red/blue) and tinted control backgrounds where the control supports them, else colour-only with the tint as platform-default | WinUI system semantic colours; banner/message-bar standard severity styles carry the tint, else colour-only with the tint as platform-default | libadwaita success/warning/error/info style classes and variables; tint carried by standard banner/message widgets, else colour-only with the tint as platform-default |
| border roles (subtle/strong/focus) | separator colour; focus ring is the platform's (§3.8) | border theme resources; focus visual is the platform's (§3.8) | border variables; focus ring is the platform's (§3.8) |

State colours are derived by the platform control itself (hover/
pressed/disabled appearances are the control's), per #5850 §3's
derivation rule — the native renderer never paints state colours.

### 3.3 Text — typography roles (#5850 §4)

Roles display / heading / body / body-small / caption / label /
code / link map to the platform's semantic text styles: Mac —
Dynamic Type text styles (large title / title / headline / body /
caption, mono for code); Windows — WinUI text style resources
(display / title / body / caption, mono theme font for code);
GNOME — libadwaita typography style classes (`title-1`…`title-4`,
`body`, `caption`, `monospace` for code). The asserted typeface
*category* (sans/serif/mono) selects the platform's font in that
category; the concrete family is the platform's. Platform text
scaling (Dynamic Type, Windows text size, GNOME text scaling)
applies to the whole scale and is never overridden by an
assertion (#5850 invariant 5).

### 3.4 Emphasis — variant roles (#5850 §§2, 4)

- **primary** → the platform's default/prominent action form: Mac
  default (Return-key) button with accent tint; Windows accent
  `Button` style; GNOME `suggested-action` style class.
- **destructive** → the platform's destructive form: Mac
  destructive-role button (red content, confirmation behaviour is
  the dialog's); Windows platform-default for the button itself
  (no standard destructive button style) with the danger tone of
  §3.2 applied as content colour — the pair is a platform-default
  entry for style, mapped for tone; GNOME `destructive-action`
  style class.
- **subtle** → the platform's low-emphasis form: borderless/flat
  standard button on all three platforms.

### 3.5 Density (#5850 §6)

comfortable / compact map to the platform's control-size system:
Mac regular vs small/mini control sizes; Windows standard vs
compact density for supported controls; GNOME platform-default
(GTK has no per-control density system; density is recorded,
warned per §3.1, and spacing roles §3.6 still apply). Density
never changes typography steps (#5850 §6).

### 3.6 Gap / pad — spacing roles (#5850 §6)

space-0…space-8 map to the platform's layout unit (Mac points,
Windows effective pixels on its 4-unit grid, GNOME pixels on the
libadwaita spacing scale), applied as standard layout margins and
inter-control spacing of the §2 containers. Assertions name roles
only; the per-platform anchor values live in the theme file.

### 3.7 Corner and elevation (#5850 §6)

- **corner** (square/rounded/full): records intent; the standard
  control's own shape wins on all three platforms (§4). Every
  (corner, platform) pair for a standard control is therefore a
  platform-default entry, except where the platform itself offers
  a shape variant of the same standard control, which the theme
  may select. A program may not depend on corner for meaning.
- **elevation** (flat/raised/overlay): ordering only — Mac window/
  sheet/material layering, Windows standard shadow/acrylic
  layering for dialogs and flyouts, GNOME border/shadow treatment
  of cards and popovers. The rendering mechanism is the
  platform's; the asserted ordering is honoured.

### 3.8 States (#5850 §5)

The state set (hover, focus-visible, active, selected, disabled,
error, loading) and its precedence (disabled > error >
selected/hover/active, focus-visible always composited on top)
are the contract. On native targets the standard control's own
state behaviour is the rendering; conformance (§5) checks the
platform behaviour against the contract rather than re-creating
it. Platform substitutions the parent and #5850 already name —
native focus rings, forced-colours/high-contrast modes replacing
tone and border roles wholesale, reduced-motion — are §4
platform-wins cases, reported by tooling, never silent.

## 4. The platform-conflict rule (decision)

**Platform wins for behaviour and accessibility; the assertion
wins for identity, subject to platform clamps.** This is a
decision, not a sketch, and it is the branch the parent listed
first, already recorded for the vocabulary layer by #5850 §8.

- **Behaviour and accessibility — platform wins.** Keyboard and
  focus behaviour, focus visibility, minimum contrast, forced-
  colours and high-contrast modes, reduced motion, minimum
  interactive target size, and platform text scaling follow the
  host even when an assertion implies otherwise. No assertion,
  override, or theme value may weaken any of these.
- **Identity — assertion wins, clamped.** Accent/brand colour and
  typeface category follow the assertion/theme (§§3.2–3.3), except
  where the chosen value would breach the platform's contrast or
  legibility minimums: then the platform clamps the value to the
  nearest compliant one and the coverage/lint tooling reports the
  clamp with both values.
- **Shape and chrome — platform wins.** Control shape, corner
  (§3.7), window chrome, and standard glyph/iconography are the
  platform's; identity is carried by colour, type category, and
  content, not by redrawing controls.
- Every clamp, substitution, and platform-default is surfaced by
  tooling (§3.1); a silent divergence between what was asserted
  and what rendered is a defect (invariant 6).

## 5. Invariants implementation must hold

1. **Standard controls only.** Every interactive element on a
   native target is an OS-standard control from §2 (or a
   composition of them). Custom-drawn controls, image buttons,
   and webview substitutions are not native renderings (see §6).
2. **Platform behaviour is untouched.** The renderer configures
   standard controls through their platform APIs; it never
   subclasses, re-paints, or event-filters a control into
   non-standard behaviour.
3. **Coverage completeness.** Every #5850 role has, per platform,
   either a §3 mapping or an explicit platform-default marker in
   the theme file. A missing entry is a coverage error, not a
   fallback.
4. **No raw values, natively.** Content assertions name roles
   only (#5850 invariant 1); platform values live in theme/token
   files keyed by role name, one file per platform target.
5. **Accessibility identity.** Role, accessible name, value, and
   state come from the semantic element and are identical across
   platforms; the accessibility tree differs between platforms
   only in the platform's own presentation of the same tree.
6. **No silent divergence.** Clamps (§4), substitutions (§3.8),
   and platform-defaults (§3.1) are all reported by the coverage
   tooling; the report is the oracle for §§3–4 conformance.
7. **State precedence is the vocabulary's.** #5850 §5's order is
   checked against platform behaviour in conformance fixtures;
   where a platform's order differs and cannot be configured, the
   difference is a reported platform-default, never re-implemented
   by re-painting states.
8. **One source.** The semantic source and its assertions are
   byte-identical for Mac, Windows, GNOME, web, and custom
   targets; all native divergence lives in the per-platform theme
   files and the §2 selection table.

Conformance follows the family's model: per-platform expected-
output fixtures assert, for each fixture program, the control
class selected per element (§2), the theme key resolved per
asserted role (§3), and the accessibility tree (invariant 5).

## 6. Alternatives considered and rejected

1. **Custom-drawn controls on native targets** (the GUI target's
   presentation overrides applied natively). Rejected: contradicts
   the parent's native rule and the #5853 record's native
   exclusion; it re-implements focus, keyboard, and accessibility
   per program, which is the failure the native target exists to
   avoid.
2. **Skinning standard controls beyond platform APIs** (owner-
   drawn borders, painted state colours, forced corners).
   Rejected: it breaks under forced-colours, high-contrast, and
   OS upgrades, and silently voids §4's accessibility wins.
3. **Per-platform assertion sets** (Mac assertions, Windows
   assertions in content). Rejected by #5850 §11 for the
   vocabulary and rejected here for the mapping: content forks
   per platform and the one-source invariant (8) dies.
4. **Ignoring assertions on native** (pure platform look for
   everything, identity included). Rejected: the parent requires
   the assertion honoured as far as the platform allows; a
   program with no recognisable identity on any native target
   fails the design's purpose. §4's clamps are the boundary, not
   blanket surrender.
5. **Assertion wins unconditionally** (identity over platform,
   including behaviour). Rejected: an assertion that can suppress
   a focus indicator or override forced colours is an
   accessibility defect by construction (#5850 invariant 4).
6. **Webview embedding as the native rendering.** Rejected: it is
   the web target in a window — no standard controls, no platform
   behaviour, and the parent's raw-HTML degradation question
   already treats webview as a fallback to specify, not as the
   native target.

## 7. Relation to the parent — no parent decision changed

This record changes **no** decision in issue #5679 and none in
the settled #5850 vocabulary. It consumes #5850 §§3–9 as its
input (per #5850 §12), applies #5853's native exclusion of
presentation overrides, and closes the one question the parent
explicitly left open for the native target — the platform-
conflict rule — by adopting the parent's first-listed branch
(§4). Two dependencies are recorded, not forked: if the settled
#5679 record contradicts an assumption in §1, the assumption and
the rows depending on it are re-issued in a superseding record;
if settled #5849 renames element kinds, §2's row labels follow
the settled names with no change to §§3–5.

## 8. Splits and follow-ons

The design does not split further: control selection (§2), the
dimension mapping (§3), and the conflict rule (§4) are one
contract — a renderer cannot implement one without the other
two. No implementation ticket is filed by this record:
implementation timing is a separate decision, per the filing
note on issue #5852. When it is taken, the implementation ticket
should cite §2 as its selection table, §3 as its mapping oracle,
§4 as its conflict rule, and §5 as its acceptance invariants,
and be linked from issue #5679's native-rendering decomposition
item.
