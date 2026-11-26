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
    // The later folder is named in the message; a distinct source
    // in the other folder is not reported.
    let report = lint_folders(&[("en", EXAMPLE), ("fr", EXAMPLE)], &standard_plans());
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
