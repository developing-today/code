# Decision: rendering pipeline & conformance suite — compile/runtime split, per-target expected-output tests, author tooling/linting

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5855](https://github.com/developing-today/code/issues/5855),
  "id: design: rendering pipeline & conformance suite (from #5679)".
- **Parent design:** [#5679](https://github.com/developing-today/code/issues/5679)
  ("Markup & styling model for programs-as-websites — semantic markup,
  style-override mapping"), its "Compilation / rendering architecture"
  section and decomposition item 7. Programme:
  [#2056](https://github.com/developing-today/code/issues/2056).
- **Settled siblings consumed, not re-decided:**
  [#5850](https://github.com/developing-today/code/issues/5850)
  (style-assertion vocabulary); [#5852](https://github.com/developing-today/code/issues/5852)
  (native rendering: assertion → OS-standard controls,
  `doc/2026-11-08_decision_id5852_native_rendering_os_standard_controls.md`);
  [#5853](https://github.com/developing-today/code/issues/5853)
  (custom GUI rendering: wholly custom presentations as first-class
  overrides, `doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`);
  [#5854](https://github.com/developing-today/code/issues/5854)
  (per-target composition: composition order, conflict reporting,
  `doc/2026-11-08_decision_id5854_per_target_composition.md`).
- **Implementation follow-ups filed from this record:**
  [#7918](https://github.com/developing-today/code/issues/7918)
  (pipeline compiler core), [#7919](https://github.com/developing-today/code/issues/7919)
  (conformance corpus and golden-test harness),
  [#7920](https://github.com/developing-today/code/issues/7920)
  (author tooling — linter and multi-target preview). Filing schedules
  nothing; implementation timing remains a separate decision, as the
  issue itself states.
- **Evidence baseline:** `origin/main` at `30bd3a1398` (worktree
  `~/git/fleet/queue-12832-5855-rendering-pipeline`, 2026-11-08):
  issue #5855 OPEN, no hard-stop labels; parent #5679 OPEN; the
  sibling records above present in `doc/` on main; no pipeline or
  conformance decision record for #5855 present in `doc/` on main.

## Decision summary

| Question                         | Decision                                                                                                                                                                                                                                                         |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Compile/runtime split            | **Hybrid, compile-time-dominant:** everything decidable from source + target tables is decided ahead of time per target; the runtime only binds data/state and instantiates the compiled artefact (§2)                                                           |
| Pipeline shape                   | Five stages over one target-neutral IR: parse → validate → compose → lower per target → emit artefact + report. Mapping tables (#5851/#5852/#5853) are _data_ inputs to lowering, never code branches in the pipeline (§2)                                       |
| Conformance tests                | Golden corpus per target: DOM/class expectations for web, control-kind + accessibility-tree expectations for native/GUI. Pixel/screenshot comparison is advisory only, never the conformance gate (§3)                                                           |
| Unmapped assertion at build time | **Warning, never silent** — the build emits an assertion-coverage report per target; a target falling back to platform/layer default is recorded in that report (adopting the parent's open-question branch, consistent with #5852 §3.1 and #5854 §6) (§3.3, §4) |
| Author tooling                   | One linter over source + tables, one preview command rendering every target's artefact plus the coverage/conflict report. Same-specificity ties and essential content in conditional blocks are errors, not warnings (§4)                                        |
| Determinism                      | Same source + same table/theme versions + same toolchain version ⇒ byte-identical artefact and report (no timestamps inside artefacts). Any change to a mapping table is a versioned change with corpus re-blessing (§5)                                         |

## 1. Assumptions against the parent (stated, not silent)

Parent issue #5679 is open and has no settled decision record yet.
Following the precedent of the #5852–#5854 records, this record states
its assumptions instead of waiting for the parent to settle:

- **A1 — the model as written today.** One content/semantic source per
  programme; semantic markup describes intent and carries its
  accessibility role by construction; styling is semantic assertions
  over named roles; raw HTML/CSS always supported; per-target page
  copies explicitly forbidden; pixel-identical rendering across
  targets is a non-goal.
- **A2 — vocabulary in flight.** Issue
  [#5849](https://github.com/developing-today/code/issues/5849)
  (semantic markup vocabulary) is not settled yet, and issue
  [#5851](https://github.com/developing-today/code/issues/5851)
  (web rendering: assertion → Tailwind) has no landed record at this
  baseline. The pipeline is therefore defined over element _kinds_ and
  assertion _roles_ generically, and consumes both vocabularies and
  the web mapping table as versioned data. A settled #5849/#5851 that
  renames kinds, roles, or classes re-issues the tables, not this
  record's decisions.
- **A3 — cascade already settled.** Composition order, raw-CSS
  placement, and conflict rules are #5854's decisions; the pipeline
  executes that order in its compose stage and reports per #5854 §6.
  This record changes none of it.

**Parent-decision changes proposed: none.** Every decision below
operates inside the parent's fixed points; where the parent left a
question open to this ticket (compile/runtime split, conformance
shape, warning-vs-silent for unmapped assertions, tooling), this
record settles it and says so.

## 2. The pipeline

### 2.1 Stages

1. **Parse.** Semantic source (plus any raw HTML/CSS, kept as opaque
   raw nodes with their target rules per #5854 §2.3) parses to the IR.
   Parse errors are fatal, located, and never recovered by guessing.
2. **Validate.** IR is checked against the #5849 element vocabulary
   and the #5850 assertion vocabulary: unknown element kinds, unknown
   roles, and assertions applied to kinds that cannot carry them are
   build errors with source locations.
3. **Compose.** Conditional blocks are resolved for the target
   (include/exclude _before_ the cascade, per #5854 §5), then the
   #5854 composition order runs: platform defaults → theme/token
   values → semantic assertions → per-target override map → raw CSS
   (web only). Output is a _resolved_ IR per target: every node
   carries its final assertion set; values stay role-named until
   lowering, so theme changes never require re-parsing.
4. **Lower.** Each target emitter consumes the resolved IR plus its
   mapping table as data — web: the #5851 assertion → Tailwind/DOM
   table; native: the #5852 control + property tables per platform;
   custom GUI: the #5853 presentation-override rules. An assertion
   with no row in a target's table lowers to that layer's default and
   is recorded in the coverage report (§3.3); lowering never invents
   a mapping.
5. **Emit.** Per target: the artefact (§2.2) and one machine-readable
   report (JSON) covering assertion coverage, reported raw-CSS
   overrides, tie errors, and conditional-block exclusions. The
   report is a build artefact, diffable and CI-checkable, not log
   noise.

### 2.2 Compile/runtime split

- **Compile time (ahead of time, per target):** stages 1–5 above.
  Structure, composition, mapping, and conflict detection are fully
  decided before anything runs. Conformance (§3) tests these
  artefacts, so a non-conforming rendering is caught pre-ship, never
  first observed by a user.
- **Runtime (thin, per target):** exactly two jobs — (a) _binding_:
  connecting the compiled artefact's state hooks and actions to
  programme state (data binding was left to the vocabulary layer;
  the pipeline's contract is only that binding points survive
  lowering unchanged in number and kind), and (b) _instantiation_:
  creating the target's real controls/DOM from the artefact (native
  targets instantiate OS-standard controls per #5852; the artefact
  names control kinds, never drawing code).
- **Never at runtime:** parsing source, resolving the cascade,
  consulting mapping tables, or deciding fallbacks. A runtime that
  re-decides rendering makes the conformance suite meaningless,
  because what ships would no longer be what was tested.

### 2.3 Alternatives rejected

- **Pure runtime interpretation** (ship source, interpret per
  target at run time): rejected — conformance could not be verified
  pre-ship; every target pays parse/compose cost at startup; the
  same source could render differently under different runtime
  versions, breaking determinism (§5).
- **Fully static per-target binaries with rendering baked in as
  code:** rejected — mapping tables become code, so a theme or
  mapping correction (a data change under #5850/#5852/#5854) would
  require a code release per target; the worked-example requirement
  of the parent (one source, three targets) is about artefacts from
  one pipeline, not three codebases.
- **Per-target source transforms** (rewrite the semantic source into
  a target dialect, then use target-native tooling): rejected — it
  is the forbidden per-target copy pattern one level up, and
  conformance would test the transforms, not the model.

## 3. Conformance suite

### 3.1 Corpus

A versioned corpus of small semantic documents, each exercising a
named slice of the model: one element kind, one assertion role, one
composition interaction, one conditional-block case, one raw-mixing
case. The corpus is data (source + expected outputs), owned by
follow-up #7919, and grows by addition — a corpus case is never
edited to make a failing build pass; the expected output is
re-blessed only in the same change that changes the deciding table
or decision, with the reason in that change.

### 3.2 Expected outputs per target

- **Web:** normalised DOM structure plus the generated class set per
  node (order-normalised; author-visible raw CSS checked as text
  presence, not formatting).
- **Native (per platform):** the control-kind sequence plus the
  accessibility tree (role, label, state set) — the properties #5852
  makes contractual. Platform metrics (exact sizes, fonts) are
  explicitly _not_ expected outputs.
- **Custom GUI:** the presentation chosen per node (standard vs.
  which override) plus the same accessibility tree as native,
  because #5853 overrides change presentation, never semantics.
- **Pixel/screenshot comparison:** permitted as a local preview aid;
  never a CI gate and never a conformance expectation. The parent's
  non-goal (native targets look native, not identical) makes pixels
  the wrong contract, and pixel gates are flaky across OS versions.

### 3.3 Coverage rules

- Every element kind in the #5849 vocabulary and every assertion role
  in the #5850 vocabulary must have at least one corpus case, or the
  suite's own coverage check fails the build of the _suite_ (the
  pipeline reports the gap; it does not block unrelated programme
  builds).
- The per-target coverage report (§2.1 stage 5) lists, per target:
  assertions mapped, assertions defaulted (with the default used),
  and raw-CSS reported overrides. "Defaulted" is a warning class —
  visible in the report and in lint (§4) — never silently zero.

## 4. Author tooling and linting

One toolchain surface, owned by follow-up #7920:

- **Lint (errors):** unknown element kind or assertion role;
  assertion on a kind that cannot carry it; same-specificity
  override tie (per #5853 §2.5 / #5854 §6); essential content inside
  a conditional block (conditional blocks carry embellishment only,
  per the parent and #5854 §5); a per-target copy of a page
  detectable as duplicated semantic source across target folders.
- **Lint (warnings):** assertion with no mapping on some target
  (will default — the author decides whether that is acceptable,
  per §3.3); raw CSS overriding an assertion-derived property on
  web (reported override, per #5854 §6); conditional block excluding
  content on all targets.
- **Preview:** one command builds every target's artefact from the
  current source and tables and presents them together with the
  coverage/conflict report, so an author sees the web rendering, the
  native control/a11y shape, and the GUI presentation choice _before_
  committing — preview is the pipeline run in report mode, not a
  second implementation of the rendering rules.
- **No separate author dialect:** tooling consumes exactly the
  vocabularies and tables the pipeline consumes; a lint rule that
  cannot be expressed over those inputs does not ship.

## 5. Invariants implementation must hold

1. **One source.** No pipeline stage creates or accepts a per-target
   variant of the semantic source; target variation lives only in
   themes, tables, override maps, assets, and conditional blocks.
2. **Determinism.** Identical source + table/theme versions +
   toolchain version ⇒ byte-identical artefacts and reports.
3. **Tested = shipped.** Runtime never re-decides anything the
   compile stages decided (§2.2); what conformance tested is what
   runs.
4. **No silent fallback.** Every default, exclusion, or reported
   override appears in the build report; absence from the report
   means it did not happen.
5. **Accessibility survives lowering.** Every target artefact
   exposes, per node, the accessibility role the semantic kind
   carries by construction; a lowering that drops a role fails
   conformance (§3.2), on every target including custom GUI.
6. **Tables are data.** Mapping, vocabulary, and theme changes ship
   as versioned data changes with corpus re-blessing, never as
   pipeline code changes.

## 6. What this record does not do

- No implementation code, corpus files, or tooling (follow-ups
  #7918, #7919, #7920 own those, on a separately-decided schedule).
- No vocabulary, mapping-table, cascade, or platform-conflict
  decisions — those are #5849–#5854's, consumed above as fixed.
- No change to any parent #5679 decision; §1 lists the assumptions
  this record stands on, and a future settled parent record that
  contradicts an assumption re-opens only the sections naming it.
