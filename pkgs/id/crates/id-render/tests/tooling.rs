#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for the author tooling (#5855 §4, issue
//! #7920): one test per lint rule the record settles, plus the
//! multi-target preview. Each test names the rule or behaviour
//! it verifies.

use std::collections::BTreeMap;

use id_render::compose::{AssertionInput, ComposeData, Specificity, ThemeTables};
use id_render::tooling::{
    Artefact, Preview, Severity, TargetOutcome, TargetPlan, lint, lint_folders, preview,
    standard_plans,
};

const BUTTON_PATH: &str = "document/page[0]/form[0]/button[0]";

/// The worked example from #5849 §2.1: no `assert` slots, no raw
/// CSS — the clean-document control.
const EXAMPLE: &str = r#"<document version="1">
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
</document>"#;

/// The assertion-bearing document of the pipeline's own tests:
/// six source assertions across page, heading, card, and button.
const ASSERTING_DOC: &str = r#"<document version="1">
  <page title="Settings" assert="tone surface-base">
    <heading level="1" assert="text heading">Settings</heading>
    <card title="Profile" assert="elevation elevation-raised, pad space-3">
      <text>Your public profile.</text>
    </card>
    <form action="save">
      <button action="save" assert="tone accent, emphasis primary">Save</button>
    </form>
  </page>
</document>"#;

const FORM_DOC: &str = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save</button>
    </form>
  </page>
</document>"#;

/// The #5854 §8a shape: raw CSS inside the button, carrying a
/// `colour-role` declaration.
const FORM_DOC_WITH_CSS: &str = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save<raw kind="css">button { colour-role: hotpink; letter-spacing: 0.02em; }</raw></button>
    </form>
  </page>
</document>"#;

#[test]
fn lint_folders_duplicate_source_is_an_error_decision_5855_s4() {
    // Rule: the same semantic source in two target folders (error).
    // Leading layout and attribute order differ, but those are not
    // semantic differences. The later folder is named in the message.
    let reformatted = format!(
        "\n{}",
        EXAMPLE.replace(
            "action=\"save-account\" confirm=\"Save account changes?\"",
            "confirm=\"Save account changes?\" action=\"save-account\"",
        )
    );
    let report = lint_folders(&[("en", EXAMPLE), ("fr", &reformatted)], &standard_plans());
    let dupes: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule == "duplicate-source")
        .collect();
    assert_eq!(dupes.len(), 1, "{report:?}");
    assert_eq!(dupes[0].severity, Severity::Error);
    assert!(dupes[0].message.contains("fr") && dupes[0].message.contains("en"));
}

#[test]
fn lint_folders_keeps_raw_content_significant_control() {
    // Oracle: raw regions pass through verbatim, so a content change means
    // the target folders contain distinct semantic source.
    let changed_raw = FORM_DOC_WITH_CSS.replace("hotpink", "rebeccapurple");
    let report = lint_folders(
        &[("en", FORM_DOC_WITH_CSS), ("fr", &changed_raw)],
        &standard_plans(),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule != "duplicate-source"),
        "{report:?}"
    );
}

#[test]
fn lint_folders_preserves_folder_for_source_diagnostics() {
    // Oracle: a source parse error reports the folder name while leaving
    // `target` for renderer-target diagnostics.
    let malformed = r#"<document version="1"><page title="T"></document>"#;
    let report = lint_folders(&[("en", EXAMPLE), ("fr", malformed)], &standard_plans());
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule == "parse")
        .expect("malformed source produces a parse diagnostic");
    assert!(diagnostic.message.starts_with("fr: "), "{diagnostic:?}");
    assert_eq!(diagnostic.target, None);
}

#[test]
fn lint_folders_distinct_sources_have_no_duplicate_control() {
    let other = EXAMPLE.replace("Account settings</heading>", "Preferences</heading>");
    let report = lint_folders(&[("en", EXAMPLE), ("fr", &other)], &standard_plans());
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.rule != "duplicate-source"),
        "{report:?}"
    );
}

fn doc_with_assert(assert: &str) -> String {
    format!(
        r#"<document version="1"><page title="T"><form action="a"><button action="a" assert="{assert}">Go</button></form></page></document>"#
    )
}

#[test]
fn lint_unknown_element_is_an_error_decision_5855_s4() {
    // Rule: unknown element kind (error). A typo'd tag is outside
    // the #5849 vocabulary; the linter reports it under the rule
    // name, located.
    let src =
        r#"<document version="1"><page title="T"><secton title="X">hi</secton></page></document>"#;
    let report = lint(src, &standard_plans());
    assert!(report.has_errors(), "{report:?}");
    let error = report.errors().next().expect("one error");
    assert_eq!(error.rule, "unknown-element");
    assert!(error.message.contains("secton"), "{error:?}");
    assert_eq!(error.line, 1);
}

#[test]
fn lint_unknown_assertion_is_an_error_decision_5855_s4() {
    // Rule: unknown assertion role (error). `tonee` is not a
    // #5850 dimension; the linter reports the slot problem under
    // the rule name, at the element carrying the slot — and the
    // per-target rules never run on a document that does not
    // build.
    let report = lint(&doc_with_assert("tonee accent"), &standard_plans());
    assert!(report.has_errors(), "{report:?}");
    assert!(
        report.diagnostics.iter().all(|d| d.target.is_none()),
        "no per-target diagnostics may follow a source error: {report:?}"
    );
    let error = report
        .diagnostics
        .iter()
        .find(|d| d.rule == "unknown-assertion")
        .expect("the unknown-assertion rule fires");
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.element_path.as_deref(), Some(BUTTON_PATH));
    assert!(error.message.contains("tonee"), "{error:?}");
}

#[test]
fn lint_unmapped_assertion_warns_on_the_defaulting_target_decision_5855_s4_s3_3() {
    // Rule: assertion with no mapping on some target (warning).
    // `corner` is a platform-default entry on the native tables
    // (#5852 §3.7): on mac the assertion defaults, and the lint
    // warning names the target, the element, and the dimension —
    // the author decides whether that is acceptable.
    let src = ASSERTING_DOC.replace(
        "assert=\"tone accent, emphasis primary\"",
        "assert=\"tone accent, emphasis primary, corner corner-rounded\"",
    );
    let report = lint(&src, &[TargetPlan::new("mac")]);
    assert!(!report.has_errors(), "{report:?}");
    let warning = report
        .warnings()
        .find(|d| d.rule == "unmapped-assertion")
        .expect("the unmapped-assertion rule fires on mac");
    assert_eq!(warning.target.as_deref(), Some("mac"));
    assert_eq!(warning.element_path.as_deref(), Some(BUTTON_PATH));
    assert!(
        warning.message.contains("corner"),
        "the warning names the defaulted dimension: {warning:?}"
    );
}

#[test]
fn lint_raw_css_override_is_a_listed_warning_decision_5855_s4() {
    // Rule: raw CSS overriding an assertion-derived property on
    // web (warning, the #5854 §6b reported-override listing).
    // The composition data asserts colour-role accent on the
    // button; the raw CSS in the source wins with hotpink, and
    // the linter lists the override.
    let mut plan = TargetPlan::new("web");
    plan.compose = ComposeData {
        assertions: vec![AssertionInput {
            element_path: BUTTON_PATH.to_string(),
            property: "colour-role".to_string(),
            token: "accent".to_string(),
            specificity: Specificity::Instance,
        }],
        theme: ThemeTables {
            base: BTreeMap::from([("accent".to_string(), "green".to_string())]),
            ..Default::default()
        },
        ..Default::default()
    };
    let report = lint(FORM_DOC_WITH_CSS, &[plan]);
    assert!(!report.has_errors(), "{report:?}");
    let warning = report
        .warnings()
        .find(|d| d.rule == "raw-css-override")
        .expect("the raw-css-override rule fires on web");
    assert_eq!(warning.target.as_deref(), Some("web"));
    assert_eq!(warning.element_path.as_deref(), Some(BUTTON_PATH));
    assert!(warning.message.contains("colour-role"), "{warning:?}");
    assert!(
        warning.message.contains("hotpink") || warning.message.contains("button"),
        "{warning:?}"
    );
}

#[test]
fn lint_clean_document_reports_nothing_decision_5855_s4() {
    // The control: a valid document with no assertions and no
    // raw CSS lints clean on every standard target.
    let report = lint(EXAMPLE, &standard_plans());
    assert!(
        report.diagnostics.is_empty(),
        "a clean document produces no diagnostics: {report:?}"
    );
}

#[test]
fn preview_renders_every_target_with_its_coverage_decision_5855_s4() {
    // Preview: one call builds every target's artefact and
    // presents the coverage report alongside. All five standard
    // targets render the assertion-bearing document.
    let preview = preview(ASSERTING_DOC, &standard_plans()).expect("the document builds");
    let names: Vec<&str> = preview.targets.iter().map(TargetOutcome::target).collect();
    assert_eq!(names, ["web", "mac", "windows", "gnome", "gui"]);
    for outcome in &preview.targets {
        assert!(
            matches!(outcome, TargetOutcome::Rendered { .. }),
            "every target renders: {outcome:?}"
        );
    }
    // The web coverage accounts all six source assertions as
    // mapped, none defaulted (the pipeline's own web verdict on
    // this document).
    let TargetOutcome::Rendered {
        artefact, coverage, ..
    } = preview.outcome("web").expect("web outcome")
    else {
        panic!("web must render");
    };
    assert_eq!(coverage.mapped, 6, "{coverage:?}");
    assert!(coverage.defaulted.is_empty(), "{coverage:?}");
    let Artefact::Web(web) = artefact else {
        panic!("the web outcome carries the web artefact");
    };
    assert!(web.class_set.contains(&"bg-accent".to_string()));
}

#[test]
fn preview_reports_a_conflict_for_one_target_and_renders_the_rest_decision_5855_s4() {
    // Preview is the pipeline in report mode: a same-specificity
    // tie (#5854 §6a) on one target is that target's conflict
    // record, not a failed preview — the other target still
    // renders.
    let tie = ComposeData {
        assertions: vec![
            AssertionInput {
                element_path: BUTTON_PATH.to_string(),
                property: "colour-role".to_string(),
                token: "accent".to_string(),
                specificity: Specificity::Instance,
            },
            AssertionInput {
                element_path: BUTTON_PATH.to_string(),
                property: "colour-role".to_string(),
                token: "brand".to_string(),
                specificity: Specificity::Instance,
            },
        ],
        ..Default::default()
    };
    let mut web = TargetPlan::new("web");
    web.compose = tie;
    let plans = vec![web, TargetPlan::new("mac")];
    let preview: Preview = preview(FORM_DOC, &plans).expect("the document builds");
    let TargetOutcome::Conflict { errors, .. } = preview.outcome("web").expect("web outcome")
    else {
        panic!("the tied target must be a conflict outcome");
    };
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("same-specificity tie")),
        "{errors:?}"
    );
    assert!(
        matches!(
            preview.outcome("mac").expect("mac outcome"),
            TargetOutcome::Rendered { .. }
        ),
        "the untied target renders"
    );
}

// ---------------------------------------------------------------
// Conditional blocks (#7920 slice 3; #5854 §5, #5855 §4).
// ---------------------------------------------------------------

/// A web-only hint (the #5854 §8 worked example's shape): prose
/// with a documentation link, beside an unconditional form.
const CONDITIONAL_DOC: &str = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save</button>
    </form>
    <conditional target="web">
      <text>See the <link to="docs.account">account guide</link>.</text>
    </conditional>
    <text>Changes apply at once.</text>
  </page>
</document>"#;

const HINT_PATH: &str = "document/page[0]/conditional[0]/text[0]";

fn conditional_doc(block: &str) -> String {
    format!(
        r#"<document version="1"><page title="T"><form action="a"><button action="a">Go</button></form>{block}</page></document>"#
    )
}

fn plan(name: &str, form_factor: &str, capabilities: &[&str]) -> TargetPlan {
    let mut plan = TargetPlan::new(name);
    plan.spec.form_factor = form_factor.to_string();
    plan.spec.capabilities = capabilities.iter().map(|c| (*c).to_string()).collect();
    plan
}

#[test]
fn conditional_block_parses_and_keeps_its_structure_decision_5854_s5() {
    // Oracle: the block parses as its own element with its predicate
    // and children intact, and the document validates.
    let doc = id_render::parse_and_validate(CONDITIONAL_DOC).expect("conditional source builds");
    let id_render::Node::Element(page) = &doc.root.children[0] else {
        panic!("page");
    };
    let block = page
        .children
        .iter()
        .find_map(|n| match n {
            id_render::Node::Element(e) if e.name == "conditional" => Some(e),
            _ => None,
        })
        .expect("the block survives parsing");
    assert_eq!(block.attr("target"), Some("web"));
    assert!(
        matches!(&block.children[0], id_render::Node::Element(e) if e.name == "text"),
        "{block:?}"
    );
}

#[test]
fn malformed_conditionals_are_located_validation_errors_decision_5854_s5() {
    // Oracle: each malformed block fails stage 2 at the block's own
    // line and column, naming the broken rule.
    let cases = [
        (
            "<conditional><text>x</text></conditional>",
            "needs a predicate",
        ),
        (
            r#"<conditional target="web" show="state.tips"><text>x</text></conditional>"#,
            "`show` slot does not apply",
        ),
        (
            r#"<conditional target="web" assert="tone accent"><text>x</text></conditional>"#,
            "`assert` slot does not apply",
        ),
        (
            r#"<conditional target="web"><conditional target="mac"><text>x</text></conditional></conditional>"#,
            "do not nest",
        ),
        (
            r#"<conditional target="web||gui"><text>x</text></conditional>"#,
            "names, not expressions",
        ),
        (
            r#"<conditional target=""><text>x</text></conditional>"#,
            "at least one name",
        ),
        (
            r#"<conditional platform="web"><text>x</text></conditional>"#,
            "unknown attribute `platform`",
        ),
    ];
    for (block, want) in cases {
        let source = conditional_doc(block);
        let errors = id_render::parse_and_validate(&source).expect_err(block);
        let hit = errors
            .iter()
            .find(|e| e.message.contains(want))
            .unwrap_or_else(|| panic!("{block}: want {want:?} in {errors:?}"));
        // The innermost block is the one in error (the nesting case).
        let col = source.rfind("<conditional").expect("block in source") + 1;
        assert_eq!((hit.line, hit.col), (1, col), "{block}: {hit:?}");
    }
}

#[test]
fn conditional_is_transparent_to_child_rules_decision_5849_s4_5() {
    // Oracle: a block's children are checked as children of the
    // block's parent — an <item> in a block under <page> still
    // breaks the item-in-list rule; a <field> in a block under
    // <form> is a legal form child (the lint, not stage 2, owns
    // whether it is essential).
    let bad = conditional_doc(r#"<conditional target="web"><item>x</item></conditional>"#);
    let errors = id_render::parse_and_validate(&bad).expect_err("item outside list");
    assert!(
        errors.iter().any(|e| e
            .message
            .contains("<item> belongs inside a <list>, found parent <page>")),
        "{errors:?}"
    );
    let ok = r#"<document version="1"><page title="T"><form action="a"><conditional target="web"><field kind="text" label="Nick" bind="a.nick"></field></conditional><button action="a">Go</button></form></page></document>"#;
    id_render::parse_and_validate(ok).expect("field under form through a block validates");
}

#[test]
fn composition_keeps_conditional_content_per_target_decision_5854_s5() {
    // Oracle: the web target composes the hint at its block path;
    // mac and gui drop it and report the exclusion with the
    // predicate as written. The block itself never reaches the
    // resolved IR.
    let doc = id_render::parse_and_validate(CONDITIONAL_DOC).expect("builds");
    let web =
        id_render::compose_document(&doc, &TargetPlan::new("web").spec, &ComposeData::default())
            .expect("web composes");
    let mut web_paths = Vec::new();
    collect_paths(&web.root, &mut web_paths);
    assert!(web_paths.contains(&HINT_PATH.to_string()), "{web_paths:?}");
    assert!(
        web_paths.iter().all(|p| !p.ends_with("conditional[0]")),
        "the block is spliced, not rendered: {web_paths:?}"
    );
    assert!(web.report.conditional_exclusions.is_empty());
    // The unconditional text keeps its own index after the block.
    assert!(web_paths.contains(&"document/page[0]/text[0]".to_string()));

    let mac =
        id_render::compose_document(&doc, &TargetPlan::new("mac").spec, &ComposeData::default())
            .expect("mac composes");
    let mut mac_paths = Vec::new();
    collect_paths(&mac.root, &mut mac_paths);
    assert!(
        !mac_paths.iter().any(|p| p.contains("conditional")),
        "{mac_paths:?}"
    );
    assert!(mac_paths.contains(&"document/page[0]/text[0]".to_string()));
    assert_eq!(mac.report.conditional_exclusions.len(), 1);
    let excluded = &mac.report.conditional_exclusions[0];
    assert_eq!(excluded.element_path, "document/page[0]/conditional[0]");
    assert_eq!(excluded.predicate, r#"target="web""#);
}

fn collect_paths(el: &id_render::compose::ComposedElement, out: &mut Vec<String>) {
    out.push(el.path.clone());
    for child in &el.children {
        if let id_render::compose::ComposedNode::Element(child) = child {
            collect_paths(child, out);
        }
    }
}

#[test]
fn conditional_predicate_reads_form_factor_and_capability_decision_5854_s5() {
    // Oracle: every predicate attribute present must match; within
    // one attribute any listed name matches.
    let doc = id_render::parse_and_validate(&conditional_doc(
        r#"<conditional form-factor="handheld tablet" capability="touch"><note>Swipe to go back.</note></conditional>"#,
    ))
    .expect("builds");
    let page = |plan: &TargetPlan| {
        id_render::compose_document(&doc, &plan.spec, &ComposeData::default())
            .expect("composes")
            .report
            .conditional_exclusions
            .is_empty()
    };
    assert!(page(&plan("mac", "handheld", &["touch"])), "both match");
    assert!(
        page(&plan("mac", "tablet", &["touch", "pen"])),
        "second form factor"
    );
    assert!(!page(&plan("mac", "handheld", &[])), "capability missing");
    assert!(
        !page(&plan("mac", "desktop", &["touch"])),
        "form factor missing"
    );
}

#[test]
fn preview_shows_conditional_content_only_on_included_targets_decision_5855_s4() {
    // Oracle: the multi-target preview renders the hint in the web
    // artefact, not in the GUI artefact, and lists the GUI target's
    // exclusion beside its artefact.
    let preview: Preview = preview(CONDITIONAL_DOC, &standard_plans()).expect("previews");
    let TargetOutcome::Rendered {
        artefact: Artefact::Web(web),
        conditional_exclusions,
        ..
    } = preview.outcome("web").expect("web")
    else {
        panic!("web renders");
    };
    assert!(conditional_exclusions.is_empty());
    assert!(format!("{web:?}").contains(HINT_PATH), "{web:?}");
    let TargetOutcome::Rendered {
        artefact: Artefact::Gui(gui),
        conditional_exclusions,
        ..
    } = preview.outcome("gui").expect("gui")
    else {
        panic!("gui renders");
    };
    assert!(!format!("{gui:?}").contains(HINT_PATH), "{gui:?}");
    assert_eq!(
        conditional_exclusions.len(),
        1,
        "{conditional_exclusions:?}"
    );
}

#[test]
fn lint_essential_content_in_conditional_is_an_error_decision_5855_s4() {
    // Rule: essential content inside a conditional block (error),
    // located at the essential element.
    let source = r#"<document version="1"><page title="T"><form action="a"><conditional target="web"><button action="a">Go</button></conditional></form></page></document>"#;
    let report = lint(source, &standard_plans());
    let hits: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule == "conditional-essential")
        .collect();
    assert_eq!(hits.len(), 1, "{report:?}");
    assert_eq!(hits[0].severity, Severity::Error);
    assert_eq!(
        hits[0].element_path.as_deref(),
        Some("document/page[0]/form[0]/conditional[0]/button[0]")
    );
    assert_eq!(hits[0].col, source.find("<button").expect("button") + 1);
    assert!(hits[0].message.contains("<button>"), "{:?}", hits[0]);
}

#[test]
fn lint_stripped_source_must_still_build_decision_5854_s5() {
    // Oracle: the §5 conformance check — removing every block must
    // leave a source that builds. A master-detail whose detail pane
    // is conditional builds whole but not stripped.
    let source = r#"<document version="1"><page title="T"><master-detail title="Mail"><list kind="unordered"><item>One</item></list><conditional target="web"><detail><text>Body</text></detail></conditional></master-detail></page></document>"#;
    id_render::parse_and_validate(source).expect("the whole source builds");
    let report = lint(source, &standard_plans());
    let errors: Vec<_> = report
        .errors()
        .filter(|d| d.rule == "conditional-essential")
        .collect();
    assert!(
        errors.iter().any(|d| d.message.contains("<detail>")),
        "{report:?}"
    );
    assert!(
        errors
            .iter()
            .any(|d| d.message.contains("no longer builds") && d.message.contains("master-detail")),
        "{report:?}"
    );
}

#[test]
fn lint_conditional_excluded_on_every_target_warns_decision_5855_s4() {
    // Rule: a block every planned target excludes (warning). Both
    // terms are known to the plan set, but no one target has both.
    let plans = [plan("web", "desktop", &[]), plan("mac", "handheld", &[])];
    let source = conditional_doc(
        r#"<conditional target="web" form-factor="handheld"><note>n</note></conditional>"#,
    );
    let report = lint(&source, &plans);
    let hits: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule == "conditional-excluded-everywhere")
        .collect();
    assert_eq!(hits.len(), 1, "{report:?}");
    assert_eq!(hits[0].severity, Severity::Warning);
    assert_eq!(
        hits[0].element_path.as_deref(),
        Some("document/page[0]/conditional[0]")
    );
    assert!(!report.has_errors(), "{report:?}");
}

#[test]
fn lint_unknown_predicate_term_is_an_error_decision_5854_s5() {
    // Rule: a term no planned target declares is an error, never
    // "treat as false" — a typo'd capability must not drop content
    // silently.
    let source = conditional_doc(r#"<conditional capability="tuoch"><note>n</note></conditional>"#);
    let report = lint(&source, &[plan("web", "desktop", &["touch"])]);
    let hits: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule == "unknown-predicate")
        .collect();
    assert_eq!(hits.len(), 1, "{report:?}");
    assert_eq!(hits[0].severity, Severity::Error);
    assert!(hits[0].message.contains("tuoch"), "{:?}", hits[0]);
}

#[test]
fn lint_embellishment_only_conditional_is_clean_control() {
    // Control: a web-only hint of prose, a link and a note raises
    // no conditional diagnostic.
    let source = r#"<document version="1"><page title="T"><form action="a"><button action="a">Go</button></form><conditional target="web"><text>See <link to="docs.a">docs</link>.</text><note>Web only.</note></conditional></page></document>"#;
    let report = lint(source, &standard_plans());
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| !d.rule.starts_with("conditional") && d.rule != "unknown-predicate"),
        "{report:?}"
    );
    assert!(!report.has_errors(), "{report:?}");
}
