# Decision: semantic markup vocabulary v1 — element set and authoring syntax

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5849](https://github.com/developing-today/code/issues/5849),
  "id: design: semantic markup vocabulary v1 (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-override mapping"), decomposition item 1 ("Semantic markup
  vocabulary v1 — element set for website + GUI concepts, authoring
  syntax"). Programme: [#2056](https://github.com/developing-today/code/issues/2056).
- **Settled siblings this record builds on:** [#5853](https://github.com/developing-today/code/issues/5853)
  (`doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`),
  [#5852](https://github.com/developing-today/code/issues/5852)
  (`doc/2026-11-08_decision_id5852_native_rendering_os_standard_controls.md`),
  [#5854](https://github.com/developing-today/code/issues/5854)
  (`doc/2026-11-08_decision_id5854_per_target_composition.md`), and the
  output-safety contract of [#5785](https://github.com/developing-today/code/issues/5785)
  (`doc/2026-11-08_design_id5785.md`). Every one of those records assumes
  a vocabulary of "semantic elements"; this record is that vocabulary.
- **Still-open siblings whose seams this record names but does not
  decide:** [#5850](https://github.com/developing-today/code/issues/5850)
  (style assertion vocabulary — the `assert` slot, §7),
  [#5851](https://github.com/developing-today/code/issues/5851)
  (web mapping — element → DOM/classes, §5), and
  [#5855](https://github.com/developing-today/code/issues/5855)
  (pipeline and conformance suite, §10).
- **Evidence baseline:** `origin/main` at `30bd3a1398`, 2026-11-08
  (task 13590). Fresh reads: issue #5849 OPEN, no hard-stop labels;
  parent #5679 OPEN; siblings #5850, #5851, #5855 OPEN, #5852, #5853,
  #5854 CLOSED with their records on `main`; issue #5849 has no linked
  PR and no decision record. `git grep -iE 'semantic (markup|element)|
element kind|programs-as-websites'` over `pkgs/id/src` and
  `pkgs/id/crates` returns no registry, parser, or vocabulary type, so
  this record is the first decision for this follow-on rather than a
  description of existing code. Source issue #2146 (closed) supplies
  the serving context the parent extends.

## Decision summary

| Question              | Decision                                                                                                                                                                                                                                                |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Authoring syntax      | **HTML-shaped source from a closed registry**: element names and attributes follow HTML vocabulary where an HTML equivalent exists, values are always quoted attributes, and content is text. One semantic dialect, no second syntax to learn (§2)      |
| How raw HTML coexists | Raw HTML is admitted **only inside an explicit `<raw kind="html">` region** (and raw CSS in `<raw kind="css">`); a tag outside the registry is a **build-time error naming the nearest registry entries**, never an implicit raw pass-through (§3)      |
| Element set           | **Closed v1 registry of 32 elements** in four groups — document/structure, inline text, forms/actions, application chrome (§4)                                                                                                                          |
| Versioning            | Source declares `version="1"` on `document`; adding, renaming, or removing an element is a version change, and v1 parsers reject unknown versions (§2.4)                                                                                                |
| Target coverage       | Every registry element must be renderable on **web, custom-GUI, and native targets** in its standard rendering; no element is web-only. Elements that could not be given a defensible native/GUI meaning are **deferred**, not shipped as web-only (§5) |
| Accessibility         | Role, accessible-name source, allowed states, and keyboard contract are **properties of the element kind**, not attributes an author may omit. Authoring checks them at build time (§6)                                                                 |
| Styling seam          | Style assertions attach through **one `assert` attribute slot per element**; the assertion grammar and vocabulary are #5850's, the cascade is #5854's. The vocabulary never names a colour, size, or variant (§7)                                       |
| State and actions     | A fixed set of **binding slots** (`bind`, `items`, `show`, `disabled-when`, `action`, `on-*`) whose values are names in the program's declared state/action table. No expressions, no code, no target-specific syntax in source (§8)                    |
| Data safety           | Every value the vocabulary carries enters only a text, quoted-attribute, or sanitized-URL context per #5785 §1; **no script context exists in the vocabulary**, and guest-emitted output is never parsed as source (§3.3)                               |
| Deferred              | Trees, charts, carousels, rich-text editing, canvas/drawing, maps, spreadsheets, command palettes, drag-and-drop, regex validation, and i18n interpolation are named out of v1 with reasons (§9)                                                        |

## 1. What is already fixed (not re-decided here)

1. **Parent #5679:** two layers — raw HTML/CSS always supported, never
   gated; semantic markup describes _intent and structure_, not
   presentation; accessibility role is intrinsic per element; one
   content/semantic source per programme; authoring ergonomics mean
   "writable by hand, diff-friendly, a strict subset in spirit of what
   HTML authors already know (names/attributes mapped from HTML where
   possible)"; data binding must connect to state and actions "without
   embedding target-specific code".
2. **#5853:** presentation overrides bind by element instance or by
   **element kind + variant/emphasis role + state set**, and must cover
   the element kind's **full state set**. That record already treats
   "element kind" and "state set" as registry concepts; this record
   defines both.
3. **#5852:** native rendering maps each element kind to an OS-standard
   control and each assertion dimension per platform; it assumes every
   element has a native control, a state set, and an accessible name.
4. **#5854:** the cascade is platform defaults → theme tokens →
   assertions → override map → raw CSS (web only); conditional blocks
   are evaluated before the cascade; raw CSS is ignored with a warning
   off the web target.
5. **#5785:** a data value enters markup only in a text, quoted-attribute,
   or sanitized-URL context, never a script context; stored or forwarded
   data is never evaluated downstream.
6. Nothing in §2–§9 changes any of the above. §11 says so explicitly and
   names the questions that would.

## 2. Authoring syntax (the decision)

### 2.1 The source is HTML-shaped, not HTML

The semantic source is a single text document whose elements are
written like HTML:

```text
<document version="1">
  <page title="Account settings">
    <heading level="1">Account settings</heading>
    <form action="save-account">
      <field kind="text" label="Display name" bind="account.name" required></field>
      <field kind="choice" label="Language" bind="account.language"
             options="state.languages"></field>
      <alert kind="error" show="account.errors.name">Name is required.</alert>
      <button action="save-account" confirm="Save account changes?">Save</button>
    </form>
  </page>
</document>
```

Rules that make this the decision rather than a preference:

- **Closed registry.** Only element names in §4 parse; only the
  attributes listed for that element parse. An unknown element or
  attribute is a build error, not a warning and not a silent fallback
  (§3.2).
- **Names come from HTML where HTML has the concept.** `page`,
  `section`, `nav`, `heading`, `text`, `list`, `item`, `link`, `image`,
  `card`, `table`, `row`, `cell`, `form`, `field`, `button`, `dialog`
  are deliberately the HTML names for the HTML concepts, so an HTML
  author's intuition transfers and a reader can guess an element's
  meaning. Where no HTML element means the right thing (`master-detail`,
  `preference`, `tabs`, `tab`), the name is the plainest English word
  for the concept.
- **Quoted attributes only.** Attribute values are always double-quoted;
  unquoted attribute syntax does not exist in the grammar. This is
  #5785 §1's attribute-context rule made syntactic, so the escaper's
  guarantee cannot be sidestepped by an author.
- **Text is content.** Element content is text (escaped at render per
  #5785 §1) plus child elements from the registry. Inline emphasis is
  carried by `strong` and `emphasis` children of `text`, never by
  attribute-driven markup.
- **No presentation attributes.** There is no `class`, `style`, `width`,
  `color`, or `align` attribute anywhere in the registry. Appearance is
  assertions (§7) and per-target overrides (#5854). An HTML-shaped
  author who reaches for `style` gets a build error that points at the
  `assert` slot.
- **Comments** use `<!-- … -->`, matching HTML, and are stripped at
  build time. They may not contain elements.
- **Whitespace between elements is insignificant**; whitespace inside
  `text` is collapsed to single spaces and leading/trailing whitespace
  trimmed, so reflowing a paragraph produces a clean diff.

### 2.2 Why not a Markdown-adjacent syntax or an s-expression DSL

Markdown is excellent for prose and poor for the application chrome
this vocabulary must also express: a form with bound fields and
validation, tabs, a master-detail pane, a settings sheet. Encoding
those in Markdown means either fenced directives that are HTML-shaped
anyway, or attribute syntax that is less regular than attributes. An
s-expression form is regular and precise, but it is a second thing to
learn for an HTML-literate audience, it diffs worse for prose-heavy
text, and it makes the raw-HTML escape hatch read as a foreign island
rather than a sibling. §10 records both as rejected alternatives with
the costs accepted in exchange.

### 2.3 Hand-authorability and diff-friendliness

- One element per line is not required, but every element is
  independently parseable and independently reflowable: wrapping a long
  attribute list or a long paragraph never changes the tree.
- Elements have no required closing-tag ceremony beyond the closing tag
  itself; self-closing form (`<field … />`) is accepted wherever an
  element has no children, and the formatter emits the self-closing
  form for empty elements only.
- No construct depends on column position, indentation, or ordering.
  Source order is meaningful only where the registry says so (sibling
  order is reading order and focus order, §6).

### 2.4 Versioning

`document` requires `version`. v1 parsers accept `version="1"` and
reject any other value with a build error naming the supported
versions. Adding an element, renaming one, changing an element's state
set, or changing a binding slot's type is a version change; widening
what an existing element accepts (an optional attribute, an extra field
kind) is a compatible v1 change recorded in the registry changelog the
pipeline ships (#5855).

## 3. Raw HTML and the boundary with emitted output

### 3.1 The one raw region

Raw HTML lives in exactly one place:

```text
<raw kind="html"><div class="ascii-art">…</div></raw>
<raw kind="css">.ascii-art { font-family: monospace }</raw>
```

- Content of `<raw kind="html">` is passed through verbatim to the web
  target and degrades per #5854 §2.3 on GUI/native targets (author
  fallback, or the target's webview embedding where #5851/#5852 allow
  it).
- Content of `<raw kind="css">` is the raw-CSS layer of #5854 §2.1 —
  highest precedence on web only, ignored with a build-time warning
  elsewhere.
- `raw` regions are recognised syntactically, so the parser never has to
  guess whether a tag was meant as HTML or as vocabulary. Authors who
  want raw HTML write it inside the region; the region is greppable, so
  a reviewer can see every escape hatch in one search.
- `raw` content is **authored source**, not data. It is never populated
  from a bound value (§3.3).

### 3.2 No implicit raw, no silent fallback

A tag outside the registry is a build error that lists the three nearest
registry names with their one-line purposes. It is never treated as raw
HTML. The reason is determinism: a typo (`<secton>`) silently becoming a
raw HTML tag would change an element from semantic (accessibility role,
native control) to inert markup, and the failure would show up as a
missing control on native targets rather than at build time. The same
rule applies to attributes: `<button style="…">` fails, `<button
assert="…">` is the supported slot.

### 3.3 Emitted output is never source

The vocabulary has no element that renders another document, evaluates
a template, or interpolates a bound value as markup:

- Every bound value reaches a text, quoted-attribute, or sanitized-URL
  context (#5785 §1). A value that contains `<form>` renders as the
  characters `<form>`, not as an element.
- There is no `include`, `template`, `script`, `eval`, or
  `inner-html`-equivalent element in v1. Author-time composition of
  repository-local sources is a pipeline concern (#5855) and is out of
  scope here; untrusted composition is out of scope for the programme.
- This keeps the #2146 safe-rendering invariant intact: guest or
  forwarded output stays inert text/screen data, while authored source
  is the only thing parsed as markup.

## 4. The v1 registry

Thirty-two elements in four groups. Columns: element, purpose,
required attributes, accessibility role, states, and the events it can
raise (all events are actions in the program's action table, §8).

### 4.1 Document and structure

| Element         | Purpose                                  | Required                                            | Role              | States                          |
| --------------- | ---------------------------------------- | --------------------------------------------------- | ----------------- | ------------------------------- |
| `document`      | Root; declares `version`                 | `version`                                           | document          | —                               |
| `page`          | One routable screen/window               | `title`                                             | main region       | —                               |
| `section`       | Thematic group                           | `title` (omit only if the sole section of its page) | region with label | —                               |
| `nav`           | Navigation group                         | `title`                                             | navigation        | —                               |
| `heading`       | Heading at a level                       | `level` (1–6)                                       | heading, level    | —                               |
| `card`          | Grouped content with a subject           | `title`                                             | group             | —                               |
| `list`          | Ordered/unordered/description collection | `kind` (`unordered`/`ordered`/`description`)        | list              | —                               |
| `item`          | One list entry                           | —                                                   | listitem          | selected (in a selectable list) |
| `table`         | Tabular data                             | `title`                                             | table             | —                               |
| `row`           | Table row                                | —                                                   | row               | selected                        |
| `cell`          | Table cell                               | `kind` (`header`/`data`)                            | columnheader/cell | —                               |
| `image`         | Content image                            | `src`, `alt` (`alt=""` only when decorative)        | img               | —                               |
| `link`          | Navigation to an address                 | `to`                                                | link              | visited, focus                  |
| `master-detail` | Two-pane list/detail layout              | `title`                                             | region with label | —                               |
| `detail`        | The detail pane of `master-detail`       | —                                                   | region            | —                               |

### 4.2 Inline text

| Element    | Purpose                         | Required | Role      | States |
| ---------- | ------------------------------- | -------- | --------- | ------ |
| `text`     | A paragraph of prose            | —        | paragraph | —      |
| `strong`   | Strong importance inside `text` | —        | strong    | —      |
| `emphasis` | Stress emphasis inside `text`   | —        | emphasis  | —      |

### 4.3 Forms and actions

| Element  | Purpose                    | Required                                    | Role                | States                                        |
| -------- | -------------------------- | ------------------------------------------- | ------------------- | --------------------------------------------- |
| `form`   | Submission boundary        | `action`                                    | form                | invalid, submitting                           |
| `field`  | One labelled input         | `kind`, `label`                             | per kind (§4.3.1)   | default, focus, invalid, disabled, required   |
| `alert`  | Validation/status message  | `kind` (`error`/`warning`/`info`/`success`) | alert (live region) | —                                             |
| `button` | Action trigger             | `action`                                    | button              | default, hover, focus, active, disabled, busy |
| `dialog` | Modal confirmation/content | `title`, `show`                             | dialog              | open, closed                                  |

#### 4.3.1 Field kinds and their constraints

`kind` is one of `text`, `password`, `number`, `checkbox`, `choice`,
`date`, `file`. Constraints are declarative and every constraint must
be enforceable on every target in the element's standard rendering:

| Constraint                 | Applies to     | Meaning                                        |
| -------------------------- | -------------- | ---------------------------------------------- |
| `required`                 | all            | value must be present                          |
| `min-length`, `max-length` | text, password | character counts                               |
| `min`, `max`               | number, date   | inclusive bounds                               |
| `options`                  | choice         | collection of allowed values, bound by `items` |
| `multiple`                 | choice, file   | more than one value allowed                    |
| `accept`                   | file           | media/type list                                |

Arbitrary regular-expression validation is **not** in v1 (§9): a regex
has no native control meaning, so exposing one would let a web-only
validation rule into a source that must render identically in intent on
three targets.

### 4.4 Application chrome

| Element      | Purpose                                         | Required                | Role              | States                                                        |
| ------------ | ----------------------------------------------- | ----------------------- | ----------------- | ------------------------------------------------------------- |
| `menu`       | Command menu                                    | `title`                 | menu              | open, closed                                                  |
| `menu-item`  | One command                                     | `action`                | menuitem          | default, hover, focus, disabled, checked (for toggling items) |
| `toolbar`    | Group of frequent actions                       | `title`                 | toolbar           | —                                                             |
| `sidebar`    | Persistent secondary region                     | `title`                 | complementary     | collapsed, expanded                                           |
| `tabs`       | Tab set                                         | `title`                 | tablist           | —                                                             |
| `tab`        | One tab and its panel                           | `title`                 | tab               | selected, focus                                               |
| `settings`   | Settings/preferences group                      | `title`                 | region with label | —                                                             |
| `preference` | One setting                                     | `kind`, `label`, `bind` | per kind (§4.4.1) | default, focus, disabled                                      |
| `note`       | Non-essential annotation tied to nearby content | —                       | note              | —                                                             |

#### 4.4.1 Preference kinds

`kind` is one of `toggle`, `choice`, `text`, `number`; `options` is
required for `choice`. A preference is semantically a persisted
setting rather than a form submission — it binds directly to state
(§8) and has no submit action. It reuses `field`'s constraint set minus
`required` (a preference always has a value).

### 4.5 Child and sibling rules (checked at build)

The registry is not a flat bag; each element declares what it may
contain, and the build checks it:

- `document` contains `page`s only. `page` may contain any structure,
  form, or chrome group. `section`, `card`, `dialog`, `tab`, and
  `detail` may contain structure, form, or chrome groups.
- `master-detail` contains exactly one `list` (the master) and exactly
  one `detail`.
- `tabs` contains `tab`s only; each `tab` contains the panel content.
- `list` contains `item`s only; `item` may contain `text`, `link`,
  `image`, `heading` (level ≥ 2), and one action element.
- `form` contains `field`, `alert`, `text`, and `button` (at most one
  submit `button`; the form's `action` names the submit action).
- `text` contains inline children only (`strong`, `emphasis`, `link`).
- `menu` contains `menu-item`s; `toolbar` contains `button`s and
  `menu`s; `settings` contains `preference`s.
- `heading` and `text` may not be direct children of `document`.
- Any violation is a build error naming the element, its parent, and
  the allowed children.

## 5. Target coverage

Each element carries a minimum standard rendering on all three targets:

- **Web:** a DOM element with the declared role (#5851 owns the exact
  element/class mapping and the assertion → Tailwind table).
- **Custom GUI:** the standard rendering of the element kind, which a
  presentation override may replace (#5853).
- **Native:** the OS-standard control for that kind (#5852 owns the
  control table).

Two consequences, stated because they constrain the registry:

1. **No web-only elements.** If an element could only be given a
   meaningful web rendering (an `<iframe>`, an inline SVG, a `<canvas>`),
   it is not in the vocabulary; it belongs in `<raw kind="html">` or in
   the deferred list.
2. **No element that a target cannot make accessible.** Every element's
   role and name source must exist on every target, which is why the
   registry has no "generic container" element: `section` and `card`
   require a title, so a nameless `div` equivalent does not exist.

## 6. Accessibility by construction

- **Role** is a property of the element kind (tables in §4), not an
  attribute. There is no way to author an element without its role.
- **Accessible name** comes from, in order: the element's explicit name
  attribute (`label`, `title`, `alt`), then its first `heading`
  descendant, then its text content. An element with none of these is a
  build error — the vocabulary has no unnamed interactive element.
- **State set** per kind (tables in §4) is what #5853 requires an
  override to cover and what #5852 maps to platform states. State is
  never authored directly: `select`-like state follows from `bind` and
  `show`/`disabled-when` (§8), and an element's `kind` never changes at
  runtime.
- **Keyboard contract** is the element kind's: `button`, `menu-item`,
  `link`, and `tab` are in the tab order and activate on the platform's
  standard key; `list`/`table`/`menu` implement their platform's
  standard selection keys; `dialog` traps focus while open and restores
  it on close. These are registry properties, so a target cannot
  "forget" them without failing conformance (#5855).
- **Focus order** is document order, with `dialog` and `menu` moving
  focus into themselves while open. There is no `tabindex` attribute.
- **Live regions:** `alert` is the only live-region element, and its
  `kind` selects politeness (`error`/`warning` assertive, `info`/
  `success` polite). An author cannot mark arbitrary content as a live
  region.

## 7. The styling seam (owned by #5850 and #5854)

The vocabulary exposes exactly one styling attachment:

- Every element accepts at most one `assert` attribute whose value is a
  style-assertion expression in #5850's grammar (e.g. an emphasis or
  colour role applied to this element or to a subtree). The grammar,
  the role names, the token model, and the assertion → idiom mapping are
  #5850's and are deliberately not repeated here.
- The cascade between assertions, per-target overrides, and raw CSS is
  #5854 §2 unchanged.
- **Assertions never change semantics.** An `assert` value cannot add,
  remove, or rebind a role, name, state, or event; the build rejects an
  assertion that names a semantic property. This is the vocabulary-side
  half of #5853 invariant 1.
- The vocabulary names no colour, size, spacing, variant, or platform
  idiom. `#5850` may add assertion dimensions without changing this
  record, provided they stay inside the `assert` slot.

## 8. Data binding and state hooks

Binding is a fixed set of slots whose values are **names**, never
expressions or code:

| Slot                                            | Applies to                                           | Meaning                                                                                         |
| ----------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `bind`                                          | `field`, `preference`, `link`, `image`, `text`       | dotted path into the program's declared state table; read/write for inputs, read-only elsewhere |
| `items`                                         | `list`, `table`, `choice` field, `tabs`              | path to a collection; children are instantiated per entry with the entry as scope               |
| `show`                                          | any element                                          | boolean state path; false means the element is not rendered                                     |
| `disabled-when`                                 | `button`, `field`, `menu-item`, `preference`, `link` | boolean state path; true means the element is present but not operable                          |
| `action`                                        | `form`, `button`, `menu-item`                        | name in the program's action table                                                              |
| `on-change`, `on-submit`, `on-open`, `on-close` | where the element raises that event                  | name in the program's action table; at most one per event                                       |

Rules:

1. **Names, not expressions.** `show="account.errors.name"` names a
   boolean in the state table; `show="len(name) > 3"` is a build error.
   Condition logic lives in the program's action/state model (#2146's
   program model, extended by the parent), not in markup.
2. **State shape is declared elsewhere.** The state and action tables
   are part of the program, not this document. This record fixes the
   slots and their types (value, collection, boolean, action name); it
   does not define how state is declared or how actions are implemented.
3. **Events are the element kind's** (tables in §4). There is no generic
   `on-click`: a `button` raises activate/submit through `action`, a
   `field` raises change, a `dialog` raises open/close, a `tab` changes
   its bound selection.
4. **No target-specific code.** No JavaScript, CSS selector, shell, URL
   scheme, or platform identifier appears in a binding slot. A URL is
   data in `to`/`src` and passes the sanitizer (#5785 §1).
5. **Instance scope.** Inside `items`, bindings are relative to the
   entry, so the same source renders every row without per-row markup.

## 9. Deferred from v1, with reasons

Named here so that the omission is a decision rather than a gap:

| Deferred                                   | Why not v1                                                                                                                                                                     |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tree views                                 | Selection semantics, expansion state, and keyboard contract differ irreconcilably across web/GUI/native without a larger state model; `list` + `detail` covers the common case |
| Charts and data visualisation              | No standard control on any target; belongs to a future vocabulary with its own assertion dimensions and accessibility story                                                    |
| Carousels, animation, transitions          | Presentation-only; raw HTML/web CSS or a #5853 presentation override expresses them today                                                                                      |
| Rich-text editing                          | Needs an editing model (selection, undo, paste sanitisation) larger than markup; `text` is read-only prose                                                                     |
| Canvas/drawing surfaces                    | Guest-drawn content is inert screen data under #2146; embedding it as vocabulary would blur authored source and emitted output (§3.3)                                          |
| Maps, calendars, spreadsheets              | Each is a product-sized widget with its own accessibility model; `table`/`image` plus raw HTML cover v1 needs                                                                  |
| Command palette                            | Composition of `dialog` + `list` + `field`; a dedicated element would freeze one interaction style                                                                             |
| Drag-and-drop                              | Needs a pointer-model and reorder contract the parent has not settled; also poorly accessible without a keyboard equivalent                                                    |
| Regex validation                           | Not enforceable on native controls without divergence (§4.3.1)                                                                                                                 |
| i18n string interpolation in `text`        | Interpolation is a program-model/state concern; `text` accepts bound values through child elements, not string templates                                                       |
| `include`/`template` composition           | Untrusted composition is excluded by #5785 §3; trusted build-time composition belongs to the pipeline (#5855)                                                                  |
| Named anchors and deep links inside a page | Routing is page-level in v1; intra-page anchors need a routing decision the parent has not made                                                                                |

## 10. Invariants implementation must hold

1. **Closed registry.** Parsing accepts exactly the 32 elements and
   their listed attributes; unknown names are build errors with the
   nearest-match suggestion (§3.2).
2. **No presentation in source.** No registry attribute carries a
   colour, size, spacing, font, alignment, or platform identifier; the
   only styling slot is `assert` (§7).
3. **Role and name are unavoidable.** Every parsed element has a role
   from the registry and a resolvable accessible name; the parser has no
   successful output for an unnamed interactive element (§6).
4. **State set completeness.** Each element kind's state set is fixed
   and available to the build, so #5853's "every state covered" check
   and #5852's state mapping have an oracle rather than a convention.
5. **Raw is explicit and greppable.** All raw HTML/CSS appears inside
   `raw` regions; a build over any source can list every escape hatch
   by searching one element name (§3.1).
6. **Emitted output is inert.** A bound value never becomes an element,
   an attribute name, a URL scheme, or code; the same rules as #5785 §1
   apply at every target (§3.3).
7. **One dialect.** There is no second authoring syntax; a source that
   mixes Markdown directives or s-expressions is a build error, not a
   dialect (§2.2).
8. **Version gate.** `document version` is required, unknown versions
   fail, and an element or slot change ships as a version change (§2.4).
9. **Target parity.** Every registry element parses into a rendering
   intent that all three targets can express; the conformance suite
   (#5855) holds one fixture per element and asserts a role, a name, and
   the state set on each target (§5).
10. **Determinism.** Formatting and child-order checks depend only on
    the source tree, never on author whitespace, attribute order, or
    iteration order of a hash map (§2.3).

## 11. Relation to the parent — no parent decision changed

This record implements and narrows the parent's Layer 2 requirements; it
does not reverse any of them:

- The parent asks for a semantic layer that is an addition, never a
  gate, over raw HTML/CSS — §3.1 keeps raw HTML always available as an
  explicit region.
- The parent asks for names/attributes mapped from HTML "where
  possible" — §2.1 adopts HTML names wherever HTML has the concept and
  records the few names that have no HTML equivalent.
- The parent asks for intent-and-structure elements that carry
  accessibility by construction — §6 makes role, name, states, and
  keyboard contract registry properties.
- The parent asks for binding to state and actions without
  target-specific code — §8 fixes slots that name state and actions.
- The parent asks to decide which GUI concepts are in v1 — §4.4 and §9
  answer exactly that.

Questions that **would** change a parent decision, and therefore require
an explicit amendment on #5679 rather than a local choice here:

1. Making the semantic layer a gate over raw HTML (it is not; §3.1).
2. Allowing an author to name a raw colour/size in source (§7 forbids).
3. Allowing a target to omit a registry element rather than render its
   standard rendering (§5 forbids); or
4. Letting emitted output be parsed as source (§3.3 forbids, per #2146).

## 12. Alternatives considered and rejected

1. **Markdown-adjacent syntax with directive fenced blocks.** Rejected:
   expressive for prose, but forms, tabs, settings, and master-detail
   collapse into directives that recreate attributes with less regular
   grammar, and the diff story for long attribute lists is worse. Cost
   accepted: prose-heavy sources carry visible tags.
2. **S-expression/DSL source.** Rejected: rigorous and easy to parse,
   but a second language for an HTML-literate audience, poor line-level
   diffs for prose, and it makes the raw-HTML escape hatch read as
   foreign. Cost accepted: angle-bracket syntax carries HTML's
   verbosity.
3. **Semantic elements as HTML custom elements (e.g. `<ui-form>`)
   with raw HTML unprefixed.** Rejected: it makes the common case
   (`<form>`) ambiguous — the parser cannot tell intent — and pushes a
   prefix onto every element to protect a rare case. Cost accepted:
   authors must wrap raw HTML in `<raw>`, which is one extra line and
   makes escape hatches visible.
4. **A large v1 covering every widget, including trees, charts, and
   rich text.** Rejected: each deferred widget needs its own
   accessibility and state model, and freezing them now would lock the
   programme into semantics it cannot defend on native targets. Cost
   accepted: some products reach for raw HTML or overrides sooner (§9).
5. **Attribute-driven presentation (`style`, `class`, `width`) for
   familiarity.** Rejected: it would let a web idiom leak into a
   source that must render on three targets and would duplicate #5850's
   assertion vocabulary. Cost accepted: HTML authors must learn the
   `assert` slot.
6. **Free-form validation expressions, including regex.** Rejected:
   no native control equivalent, so validation would diverge per
   target. Cost accepted: some constraint logic moves into the action
   model.

## 13. Splits and follow-ons

None. The registry is one decision and splits would fragment the
accessibility and state contracts it defines. The seams this record
leaves to siblings are named, not guessed:

- **#5850** — assertion grammar, role names, token model (§7).
- **#5851** — element → DOM element/class mapping and the
  assertion → Tailwind table (§5).
- **#5855** — parser, formatter, build diagnostics, and the
  per-element conformance fixtures (§10).

If a future need appears to require a new element, the path is: file it
against the parent's vocabulary, decide it as a v2 (or a v1-compatible
widening) here, and only then implement — not a local extension in a
target pack.
